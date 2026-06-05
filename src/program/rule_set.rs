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
    OnceRewriteRuleView, OnceRuleCount, RuleCount, RulePosition, RuleView,
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
    /// Dense count of once-only executable rules assigned by this topology.
    once_rule_count: OnceRuleCount,
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
    /// Once-only non-terminal rewrite rule and its dense once slot.
    OnceRewrite(OnceRewriteRuleView<'program>, OnceRuleSlot),
    /// Reusable terminal return rule.
    AlwaysReturn(AlwaysReturnRuleView<'program>),
    /// Once-only terminal return rule and its dense once slot.
    OnceReturn(OnceReturnRuleView<'program>, OnceRuleSlot),
}

/// Runtime availability slot assigned to one parsed once-only rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct OnceRuleSlot {
    /// Zero-based slot in the per-run once-state table.
    zero_based: usize,
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
    /// Dense once-state slot assigned by the executable topology.
    slot: OnceRuleSlot,
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
    /// Dense once-state slot assigned by the executable topology.
    slot: OnceRuleSlot,
    /// Positionless parsed rule payload.
    rule: ReturnRule,
}

/// Stored rule plus the next once-rule count after assignment.
struct AssignedStoredRule {
    /// Stored executable rule with topology witnesses.
    rule: StoredRule,
    /// Once-rule count after accepting this rule.
    once_rule_count: OnceRuleCount,
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
                let assigned =
                    StoredRule::assign_topology(rule, RulePosition::FIRST, OnceRuleCount::ZERO)?;
                *self = Self::NonEmpty(ExecutableRuleSet {
                    first: assigned.rule,
                    remaining: Vec::new(),
                    rule_count: ExecutableRuleCount::ONE,
                    once_rule_count: assigned.once_rule_count,
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
                    rule_set.once_rule_count,
                )?;
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
                rule_set.once_rule_count = assigned.once_rule_count;
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
    if limit.admit(attempted_count).is_some() {
        Ok(())
    } else {
        Err(ParseError::at_line(
            line_number,
            ParseErrorKind::Limit(ParseLimitError::rules(limit, attempted_count)),
        ))
    }
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

/// Reports that dense once-slot growth exceeded the platform representation.
fn once_rule_count_overflow(line_number: SourceLineNumber) -> ParseError {
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

    /// Public count of parsed `(once)` rules derived from concrete variants.
    pub(crate) const fn once_rule_count(&self) -> OnceRuleCount {
        self.once_rule_count
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
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if assigning a dense once slot would exceed the
    /// platform count representation.
    fn assign_topology(
        rule: ParsedRule,
        position: RulePosition,
        once_rule_count: OnceRuleCount,
    ) -> Result<AssignedStoredRule, ParseError> {
        let line_number = rule.line_number();
        match rule {
            ParsedRule::AlwaysRewrite(rule) => Ok(AssignedStoredRule {
                rule: Self::AlwaysRewrite(StoredAlwaysRewriteRule { position, rule }),
                once_rule_count,
            }),
            ParsedRule::OnceRewrite(rule) => {
                let (slot, once_rule_count) = assign_once_slot(once_rule_count, line_number)?;
                Ok(AssignedStoredRule {
                    rule: Self::OnceRewrite(StoredOnceRewriteRule {
                        position,
                        slot,
                        rule,
                    }),
                    once_rule_count,
                })
            }
            ParsedRule::AlwaysReturn(rule) => Ok(AssignedStoredRule {
                rule: Self::AlwaysReturn(StoredAlwaysReturnRule { position, rule }),
                once_rule_count,
            }),
            ParsedRule::OnceReturn(rule) => {
                let (slot, once_rule_count) = assign_once_slot(once_rule_count, line_number)?;
                Ok(AssignedStoredRule {
                    rule: Self::OnceReturn(StoredOnceReturnRule {
                        position,
                        slot,
                        rule,
                    }),
                    once_rule_count,
                })
            }
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
            Self::OnceRewrite(rule) => RuntimeStoredRule::OnceRewrite(
                OnceRewriteRuleView::new(rule.position, &rule.rule),
                rule.slot,
            ),
            Self::AlwaysReturn(rule) => RuntimeStoredRule::AlwaysReturn(AlwaysReturnRuleView::new(
                rule.position,
                &rule.rule,
            )),
            Self::OnceReturn(rule) => RuntimeStoredRule::OnceReturn(
                OnceReturnRuleView::new(rule.position, &rule.rule),
                rule.slot,
            ),
        }
    }
}

/// Assigns the next dense once slot and advances the count witness.
///
/// # Errors
///
/// Returns `ParseError` if advancing the once-rule count would exceed the
/// platform count representation.
fn assign_once_slot(
    count: OnceRuleCount,
    line_number: SourceLineNumber,
) -> Result<(OnceRuleSlot, OnceRuleCount), ParseError> {
    let slot = OnceRuleSlot::from_next_count(count);
    let next_count = count
        .checked_next()
        .ok_or_else(|| once_rule_count_overflow(line_number))?;
    Ok((slot, next_count))
}

impl OnceRuleSlot {
    /// Builds the next slot from the current accepted once-rule count.
    const fn from_next_count(count: OnceRuleCount) -> Self {
        Self {
            zero_based: count.get(),
        }
    }

    /// Zero-based slot in a per-run once-state table.
    pub(crate) const fn index(self) -> usize {
        self.zero_based
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
