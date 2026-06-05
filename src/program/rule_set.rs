use alloc::vec::Vec;
use core::slice;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::error::{
    EmptyProgramParseError, ExecutableProgramParseError, ParseError, ParseErrorKind,
    ParseLimitError,
};
use crate::inspect::{
    AlwaysReturnRuleView, AlwaysRewriteRuleView, ExecutableRuleCount, OnceReturnRuleView,
    OnceRewriteRuleView, RuleCount, RulePosition, RuleView,
};
use crate::limits::RuleLimit;
use crate::rule::{ParsedRule, ReturnRule, RewriteRule};
use crate::source::SourceLineNumber;

/// Non-empty immutable executable rule table.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ExecutableRuleSet {
    /// First executable rule, separated so executable scans have an infallible head.
    first: StoredRule,
    /// Remaining executable rules in execution order.
    remaining: Vec<StoredRule>,
    /// Non-zero executable-rule count assigned by this topology.
    rule_count: ExecutableRuleCount,
}

/// Executable rule stored with topology-derived witnesses.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StoredRule {
    /// Reusable non-terminal rewrite rule.
    AlwaysRewrite(StoredAlwaysRewriteRule),
    /// Once-only non-terminal rewrite rule.
    OnceRewrite(StoredOnceRewriteRule),
    /// Reusable terminal return rule.
    AlwaysReturn(StoredAlwaysReturnRule),
    /// Once-only terminal return rule.
    OnceReturn(StoredOnceReturnRule),
}

/// Borrowed stored rule from one executable topology.
#[derive(Debug, Clone, Copy)]
pub(crate) struct StoredRuleRef<'program> {
    /// Stored executable rule borrowed from the program topology.
    rule: &'program StoredRule,
}

/// Runtime-facing stored rule view that preserves once-slot provenance.
#[derive(Debug, Clone, Copy)]
pub(crate) enum RuntimeStoredRule<'program> {
    /// Reusable non-terminal rewrite rule.
    AlwaysRewrite(AlwaysRewriteRuleView<'program>),
    /// Once-only non-terminal rewrite rule.
    OnceRewrite(OnceRewriteRuleView<'program>),
    /// Reusable terminal return rule.
    AlwaysReturn(AlwaysReturnRuleView<'program>),
    /// Once-only terminal return rule.
    OnceReturn(OnceReturnRuleView<'program>),
}

/// Stored reusable rewrite rule.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct StoredAlwaysRewriteRule {
    /// Execution-order position assigned by the executable topology.
    position: RulePosition,
    /// Positionless parsed rule payload.
    rule: RewriteRule,
}

/// Stored once-only rewrite rule.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct StoredOnceRewriteRule {
    /// Execution-order position assigned by the executable topology.
    position: RulePosition,
    /// Positionless parsed rule payload.
    rule: RewriteRule,
}

/// Stored reusable return rule.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct StoredAlwaysReturnRule {
    /// Execution-order position assigned by the executable topology.
    position: RulePosition,
    /// Positionless parsed rule payload.
    rule: ReturnRule,
}

/// Stored once-only return rule.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct StoredOnceReturnRule {
    /// Execution-order position assigned by the executable topology.
    position: RulePosition,
    /// Positionless parsed rule payload.
    rule: ReturnRule,
}

/// Stored rule with topology-derived witnesses.
struct AssignedStoredRule {
    /// Stored executable rule with topology witnesses.
    rule: StoredRule,
}

/// Borrowed executable rule scan minted from one non-empty parsed rule table.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RuleScan<'program> {
    /// First stored executable rule.
    first: &'program StoredRule,
    /// Remaining stored executable rules in execution order.
    remaining: &'program [StoredRule],
}

/// Iterator over a non-empty executable rule scan.
pub(crate) struct RuleScanIter<'program> {
    /// Stored rules in execution order.
    rules: core::iter::Chain<
        core::iter::Once<&'program StoredRule>,
        slice::Iter<'program, StoredRule>,
    >,
}

/// Iterator over the tail of an executable rule scan.
pub(crate) struct RuleScanTail<'program> {
    /// Remaining stored rules after the split head.
    rules: slice::Iter<'program, StoredRule>,
}

/// Parser sink that decides the target program shape while source is parsed.
pub(crate) trait RuleSink: Sized {
    /// Program value produced after the source accepted by this sink is parsed.
    type Output;
    /// Error domain for parser failures plus target-shape mismatch.
    type Error: From<ParseError>;

    /// Starts the sink before source lines are parsed.
    fn new() -> Self;

    /// Accepts one fully parsed executable rule.
    ///
    /// # Errors
    ///
    /// Returns this sink's error if the rule exceeds parser limits, cannot be
    /// stored, or proves that the requested target shape is invalid.
    fn push_rule(&mut self, rule: ParsedRule, limit: RuleLimit) -> Result<(), Self::Error>;

    /// Finishes the target-shape parse.
    ///
    /// # Errors
    ///
    /// Returns this sink's error if the parsed source has the wrong executable
    /// shape for the requested program target.
    fn finish(self) -> Result<Self::Output, Self::Error>;
}

/// Parser-owned executable rule table builder.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ExecutableRuleSetBuilder {
    /// No executable rule has been parsed.
    Empty,
    /// At least one executable rule has been parsed.
    NonEmpty(ExecutableRuleSet),
}

/// Stateless parser sink for source that must contain no executable rules.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EmptyRuleSetBuilder;

impl RuleSink for ExecutableRuleSetBuilder {
    type Output = ExecutableRuleSet;
    type Error = ExecutableProgramParseError;

    fn new() -> Self {
        Self::Empty
    }

    fn push_rule(&mut self, rule: ParsedRule, limit: RuleLimit) -> Result<(), Self::Error> {
        let line_number = rule.line_number();
        match self {
            Self::Empty => {
                let attempted_count = RuleCount::new(1);
                ensure_rule_limit(attempted_count, limit, line_number)?;
                let assigned = StoredRule::assign_topology(rule, RulePosition::FIRST);
                *self = Self::NonEmpty(ExecutableRuleSet {
                    first: assigned.rule,
                    remaining: Vec::new(),
                    rule_count: ExecutableRuleCount::ONE,
                });
            }
            Self::NonEmpty(rule_set) => {
                let attempted_count = rule_set
                    .rule_count
                    .checked_next_rule_count()
                    .ok_or_else(|| rule_count_overflow(line_number))?;
                ensure_rule_limit(attempted_count, limit, line_number)?;
                let executable_count = ExecutableRuleCount::from_rule_count(attempted_count)
                    .ok_or_else(|| rule_count_overflow(line_number))?;
                let assigned = StoredRule::assign_topology(
                    rule,
                    RulePosition::from_executable_count(executable_count),
                );
                let remaining_capacity = attempted_count
                    .get()
                    .checked_sub(1)
                    .ok_or_else(|| rule_count_overflow(line_number))?;
                let requested_capacity = RequestedCapacity::new(remaining_capacity);
                try_reserve_total_exact(
                    &mut rule_set.remaining,
                    requested_capacity,
                    AllocationContext::ProgramRuleTable,
                )
                .map_err(|error| {
                    ParseError::at_line(line_number, ParseErrorKind::Allocation(error))
                })?;
                try_push(
                    &mut rule_set.remaining,
                    assigned.rule,
                    AllocationContext::ProgramRuleTable,
                )
                .map_err(|error| {
                    ParseError::at_line(line_number, ParseErrorKind::Allocation(error))
                })?;
                rule_set.rule_count = executable_count;
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<Self::Output, Self::Error> {
        match self {
            Self::Empty => Err(ExecutableProgramParseError::NoExecutableRules),
            Self::NonEmpty(rule_set) => Ok(rule_set),
        }
    }
}

impl RuleSink for EmptyRuleSetBuilder {
    type Output = ();
    type Error = EmptyProgramParseError;

    fn new() -> Self {
        Self
    }

    fn push_rule(&mut self, rule: ParsedRule, _limit: RuleLimit) -> Result<(), Self::Error> {
        Err(EmptyProgramParseError::ExecutableRule {
            line_number: rule.line_number(),
        })
    }

    fn finish(self) -> Result<Self::Output, Self::Error> {
        Ok(())
    }
}

/// Checks one topology-derived rule count against the selected parser policy.
///
/// # Errors
///
/// Returns `ParseError` when accepting the rule would exceed the parser rule
/// limit.
fn ensure_rule_limit(
    attempted_count: RuleCount,
    limit: RuleLimit,
    line_number: SourceLineNumber,
) -> Result<(), ParseError> {
    let _rule_count_permit = limit.admit(attempted_count).ok_or_else(|| {
        ParseError::at_line(
            line_number,
            ParseErrorKind::Limit(ParseLimitError::rules(limit, attempted_count)),
        )
    })?;
    Ok(())
}

/// Reports that topology count growth exceeded the platform representation.
fn rule_count_overflow(line_number: SourceLineNumber) -> ParseError {
    ParseError::at_line(
        line_number,
        ParseErrorKind::Allocation(AllocationError::capacity_overflow(
            AllocationContext::ProgramRuleTable,
        )),
    )
}

impl ExecutableRuleSet {
    /// Total executable rules derived from this non-empty table's topology.
    pub(crate) const fn rule_count(&self) -> ExecutableRuleCount {
        self.rule_count
    }

    /// Iterates executable rules in execution order.
    pub(crate) fn iter(&self) -> RuleScanIter<'_> {
        self.scan().iter()
    }

    /// Starts a runtime scan over this non-empty table's executable rules.
    pub(crate) fn scan(&self) -> RuleScan<'_> {
        RuleScan {
            first: &self.first,
            remaining: self.remaining.as_slice(),
        }
    }
}

impl StoredRule {
    /// Assigns topology witnesses to one parser-produced rule.
    fn assign_topology(rule: ParsedRule, position: RulePosition) -> AssignedStoredRule {
        match rule {
            ParsedRule::AlwaysRewrite(rule) => AssignedStoredRule {
                rule: Self::AlwaysRewrite(StoredAlwaysRewriteRule { position, rule }),
            },
            ParsedRule::OnceRewrite(rule) => AssignedStoredRule {
                rule: Self::OnceRewrite(StoredOnceRewriteRule { position, rule }),
            },
            ParsedRule::AlwaysReturn(rule) => AssignedStoredRule {
                rule: Self::AlwaysReturn(StoredAlwaysReturnRule { position, rule }),
            },
            ParsedRule::OnceReturn(rule) => AssignedStoredRule {
                rule: Self::OnceReturn(StoredOnceReturnRule { position, rule }),
            },
        }
    }

    /// Projects this stored rule into the public typed rule view.
    fn view(&self) -> RuleView<'_> {
        match self {
            Self::AlwaysRewrite(rule) => RuleView::from_always_rewrite(rule.position, &rule.rule),
            Self::OnceRewrite(rule) => RuleView::from_once_rewrite(rule.position, &rule.rule),
            Self::AlwaysReturn(rule) => RuleView::from_always_return(rule.position, &rule.rule),
            Self::OnceReturn(rule) => RuleView::from_once_return(rule.position, &rule.rule),
        }
    }

    /// Projects this stored rule into the runtime view with once-slot provenance.
    fn runtime_rule(&self) -> RuntimeStoredRule<'_> {
        match self {
            Self::AlwaysRewrite(rule) => RuntimeStoredRule::AlwaysRewrite(
                AlwaysRewriteRuleView::new(rule.position, &rule.rule),
            ),
            Self::OnceRewrite(rule) => {
                RuntimeStoredRule::OnceRewrite(OnceRewriteRuleView::new(rule.position, &rule.rule))
            }
            Self::AlwaysReturn(rule) => RuntimeStoredRule::AlwaysReturn(AlwaysReturnRuleView::new(
                rule.position,
                &rule.rule,
            )),
            Self::OnceReturn(rule) => {
                RuntimeStoredRule::OnceReturn(OnceReturnRuleView::new(rule.position, &rule.rule))
            }
        }
    }
}

impl<'program> StoredRuleRef<'program> {
    /// Borrows one stored executable rule.
    const fn new(rule: &'program StoredRule) -> Self {
        Self { rule }
    }

    /// Projects this stored rule into the public typed rule view.
    pub(crate) fn view(self) -> RuleView<'program> {
        self.rule.view()
    }

    /// Projects this stored rule into the runtime view with once-slot provenance.
    pub(crate) fn runtime_rule(self) -> RuntimeStoredRule<'program> {
        self.rule.runtime_rule()
    }
}

impl<'program> RuleScan<'program> {
    /// Iterates executable rules with stored topology witnesses.
    pub(crate) fn iter(self) -> RuleScanIter<'program> {
        RuleScanIter {
            rules: core::iter::once(self.first).chain(self.remaining.iter()),
        }
    }

    /// Splits this non-empty scan into its first stored rule and stored tail.
    pub(crate) fn split_first(self) -> (StoredRuleRef<'program>, RuleScanTail<'program>) {
        (
            StoredRuleRef::new(self.first),
            RuleScanTail {
                rules: self.remaining.iter(),
            },
        )
    }
}

impl<'program> Iterator for RuleScanIter<'program> {
    type Item = StoredRuleRef<'program>;

    fn next(&mut self) -> Option<Self::Item> {
        self.rules.next().map(StoredRuleRef::new)
    }
}

impl ExactSizeIterator for RuleScanIter<'_> {
    fn len(&self) -> usize {
        self.rules.size_hint().0
    }
}

impl<'program> Iterator for RuleScanTail<'program> {
    type Item = StoredRuleRef<'program>;

    fn next(&mut self) -> Option<Self::Item> {
        self.rules.next().map(StoredRuleRef::new)
    }
}

impl ExactSizeIterator for RuleScanTail<'_> {
    fn len(&self) -> usize {
        self.rules.len()
    }
}
