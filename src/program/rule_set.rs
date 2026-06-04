use alloc::vec::Vec;
use core::slice;

use crate::allocation::{AllocationContext, RequestedCapacity, try_push, try_reserve_total_exact};
use crate::error::{
    EmptyProgramParseError, ExecutableProgramParseError, ParseError, ParseErrorKind,
    ParseLimitError,
};
use crate::inspect::{OnceRuleCount, RuleCount, RulePosition, RuleView};
use crate::limits::RuleLimit;
use crate::rule::Rule;

/// Non-empty immutable executable rule table.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ExecutableRuleSet {
    /// First executable rule, separated so executable scans have an infallible head.
    first: Rule,
    /// Remaining executable rules in execution order.
    remaining: Vec<Rule>,
}

/// One borrowed executable rule paired with its topology-derived position.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PositionedRule<'program> {
    /// Execution-order position within the containing executable rule set.
    position: RulePosition,
    /// Positionless executable rule stored by the program.
    rule: &'program Rule,
}

/// Borrowed executable rule scan minted from one non-empty parsed rule table.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RuleScan<'program> {
    /// First parsed executable rule.
    first: &'program Rule,
    /// Remaining parsed executable rules in execution order.
    remaining: &'program [Rule],
}

/// Iterator over a non-empty executable rule scan.
pub(crate) struct RuleScanIter<'program> {
    /// First rule that has not yet been yielded.
    first: Option<&'program Rule>,
    /// Remaining parsed executable rules.
    remaining: slice::Iter<'program, Rule>,
    /// Zero-based position assigned to the next yielded rule.
    next_zero_based: usize,
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
    fn push_rule(&mut self, rule: Rule, limit: RuleLimit) -> Result<(), Self::Error>;

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

    fn push_rule(&mut self, rule: Rule, limit: RuleLimit) -> Result<(), Self::Error> {
        let line_number = rule.line_number();
        match self {
            Self::Empty => {
                ensure_rule_limit(RuleCount::new(1), limit, line_number)?;
                *self = Self::NonEmpty(ExecutableRuleSet {
                    first: rule,
                    remaining: Vec::new(),
                });
            }
            Self::NonEmpty(rule_set) => {
                let attempted_count = RuleCount::new(rule_set.rule_count().get().saturating_add(1));
                ensure_rule_limit(attempted_count, limit, line_number)?;
                let requested_capacity =
                    RequestedCapacity::new(attempted_count.get().saturating_sub(1));
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
                    rule,
                    AllocationContext::ProgramRuleTable,
                )
                .map_err(|error| {
                    ParseError::at_line(line_number, ParseErrorKind::Allocation(error))
                })?;
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

    fn push_rule(&mut self, rule: Rule, _limit: RuleLimit) -> Result<(), Self::Error> {
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
    line_number: crate::source::SourceLineNumber,
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

impl ExecutableRuleSet {
    /// Total executable rules derived from this non-empty table's topology.
    pub(crate) fn rule_count(&self) -> RuleCount {
        RuleCount::new(self.remaining.len().saturating_add(1))
    }

    /// Public count of parsed `(once)` rules derived from concrete variants.
    pub(crate) fn once_rule_count(&self) -> OnceRuleCount {
        OnceRuleCount::new(
            self.iter()
                .filter(|positioned| positioned.rule().is_once())
                .count(),
        )
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

impl<'program> PositionedRule<'program> {
    /// Positionless executable rule.
    pub(crate) const fn rule(self) -> &'program Rule {
        self.rule
    }

    /// Projects this position-bearing witness into the public typed rule view.
    pub(crate) fn view(self) -> RuleView<'program> {
        RuleView::new(self.position, self.rule)
    }
}

impl<'program> RuleScan<'program> {
    /// Iterates executable rules with topology-derived positions.
    pub(crate) fn iter(self) -> RuleScanIter<'program> {
        RuleScanIter {
            first: Some(self.first),
            remaining: self.remaining.iter(),
            next_zero_based: 0,
        }
    }

    /// Splits this non-empty scan into its first positioned rule and positioned tail.
    pub(crate) fn split_first(self) -> (PositionedRule<'program>, RuleScanIter<'program>) {
        (
            PositionedRule {
                position: RulePosition::from_zero_based(0),
                rule: self.first,
            },
            RuleScanIter {
                first: None,
                remaining: self.remaining.iter(),
                next_zero_based: 1,
            },
        )
    }
}

impl<'program> Iterator for RuleScanIter<'program> {
    type Item = PositionedRule<'program>;

    fn next(&mut self) -> Option<Self::Item> {
        let rule = match self.first.take() {
            Some(first) => first,
            None => self.remaining.next()?,
        };
        let position = RulePosition::from_zero_based(self.next_zero_based);
        self.next_zero_based = self.next_zero_based.saturating_add(1);
        Some(PositionedRule { position, rule })
    }
}

impl ExactSizeIterator for RuleScanIter<'_> {
    fn len(&self) -> usize {
        let first_len = usize::from(self.first.is_some());
        self.remaining.len().saturating_add(first_len)
    }
}
