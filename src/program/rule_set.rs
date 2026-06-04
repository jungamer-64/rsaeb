use alloc::vec::Vec;
use core::slice;

use crate::allocation::{AllocationContext, RequestedCapacity, try_push, try_reserve_total_exact};
use crate::error::{
    EmptyProgramParseError, ExecutableProgramParseError, ParseError, ParseErrorKind,
    ParseLimitError, ParseRepresentationError,
};
use crate::inspect::{OnceRuleCount as PublicOnceRuleCount, RuleCount, RulePosition};
use crate::limits::RuleLimit;
use crate::rule::{ParsedRule, Rule};

/// Non-empty immutable executable rule table.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ExecutableRuleSet {
    /// First executable rule, separated so executable scans have an infallible head.
    first: Rule,
    /// Remaining executable rules in execution order.
    remaining: Vec<Rule>,
    /// Total executable rule count.
    rule_count: RuleCount,
    /// Parsed `(once)` rule count assigned while building this rule table.
    once_rule_count: PublicOnceRuleCount,
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
}

/// Parser sink that decides the target program shape while source is parsed.
pub(crate) trait ParsedRuleSink: Sized {
    /// Program value produced after the full source has been parsed.
    type Output;
    /// Error domain for parser failures plus target-shape mismatch.
    type Error: From<ParseError>;

    /// Starts the sink before source lines are parsed.
    fn new() -> Self;

    /// Accepts one parsed executable rule after syntax validation.
    ///
    /// # Errors
    ///
    /// Returns this sink's error if the parsed rule cannot be represented,
    /// exceeds parser limits, or cannot be stored.
    fn push_parsed_rule(&mut self, parsed: ParsedRule, limit: RuleLimit)
    -> Result<(), Self::Error>;

    /// Finishes the target-shape parse after every source line has been checked.
    ///
    /// # Errors
    ///
    /// Returns this sink's error if the parsed source has the wrong executable
    /// shape for the requested program target.
    fn finish(self) -> Result<Self::Output, Self::Error>;
}

/// Parser-owned executable rule table builder.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ExecutableRuleSetBuilder {
    /// First parsed executable rule, if any has been accepted.
    first: Option<Rule>,
    /// Later parsed rules in execution order.
    remaining: Vec<Rule>,
    /// Parsed executable rules seen so far.
    rule_count: RuleCount,
    /// Parsed `(once)` rules seen so far.
    once_rule_count: PublicOnceRuleCount,
}

/// Parser-owned empty target builder.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EmptyRuleSetBuilder {
    /// Parsed executable rules seen so far.
    rule_count: RuleCount,
}

/// Parsed rule after execution position assignment but before table insertion.
struct PendingRuleInsertion {
    /// Rule ready for storage in execution order.
    rule: Rule,
}

/// Checked permission to insert one parsed rule into the table.
struct RuleInsertionPermit {
    /// Rule count after the permitted insertion.
    attempted_rule_count: RuleCount,
    /// Execution-order position assigned to the accepted rule.
    position: RulePosition,
}

impl RuleInsertionPermit {
    /// Checks rule-count budget and table-position representability together.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the next rule count overflows, exceeds the
    /// parser rule budget, or cannot be represented as a checked rule position.
    fn new(
        current_len: usize,
        limit: RuleLimit,
        line_number: crate::source::SourceLineNumber,
    ) -> Result<Self, ParseError> {
        let attempted_rule_count = current_len.checked_add(1).ok_or_else(|| {
            ParseError::at_line(
                line_number,
                ParseErrorKind::Representation(ParseRepresentationError::RuleCount),
            )
        })?;
        let attempted_rule_count = RuleCount::new(attempted_rule_count);

        if limit.admit(attempted_rule_count).is_none() {
            return Err(ParseError::at_line(
                line_number,
                ParseErrorKind::Limit(ParseLimitError::rules(limit, attempted_rule_count)),
            ));
        }

        let position = RulePosition::from_zero_based(current_len).ok_or_else(|| {
            ParseError::at_line(
                line_number,
                ParseErrorKind::Representation(ParseRepresentationError::RulePosition),
            )
        })?;

        Ok(Self {
            attempted_rule_count,
            position,
        })
    }

    /// Remaining-rule capacity required before insertion into a split table.
    const fn remaining_requested_capacity(&self) -> RequestedCapacity {
        RequestedCapacity::new(self.attempted_rule_count.get().saturating_sub(1))
    }

    /// Rule count after this insertion commits.
    const fn rule_count(&self) -> RuleCount {
        self.attempted_rule_count
    }

    /// Execution-order position assigned to this insertion.
    const fn position(&self) -> RulePosition {
        self.position
    }
}

impl PendingRuleInsertion {
    /// Assigns execution position to one parsed rule before storage.
    fn from_parsed(position: RulePosition, parsed: ParsedRule) -> Self {
        Self {
            rule: Rule::from_parsed(position, parsed),
        }
    }

    /// Source line used if storing this rule fails.
    const fn line_number(&self) -> crate::source::SourceLineNumber {
        self.rule.line_number()
    }
}

impl ParsedRuleSink for ExecutableRuleSetBuilder {
    type Output = ExecutableRuleSet;
    type Error = ExecutableProgramParseError;

    /// Starts an empty parsed rule table.
    fn new() -> Self {
        Self {
            first: None,
            remaining: Vec::new(),
            rule_count: RuleCount::new(0),
            once_rule_count: PublicOnceRuleCount::ZERO,
        }
    }

    /// Stores one parsed rule and assigns its program-local position.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the rule position cannot be represented or
    /// the rule table cannot grow.
    fn push_parsed_rule(
        &mut self,
        parsed: ParsedRule,
        limit: RuleLimit,
    ) -> Result<(), Self::Error> {
        let line_number = parsed.line_number();
        let insertion = RuleInsertionPermit::new(self.rule_count.get(), limit, line_number)?;
        let next_once_rule_count =
            next_once_rule_count(self.once_rule_count, &parsed, line_number)?;

        let pending = PendingRuleInsertion::from_parsed(insertion.position(), parsed);

        if self.first.is_none() {
            self.first = Some(pending.rule);
        } else {
            let pending_line_number = pending.line_number();
            try_reserve_total_exact(
                &mut self.remaining,
                insertion.remaining_requested_capacity(),
                AllocationContext::ProgramRuleTable,
            )
            .map_err(|error| {
                ParseError::at_line(pending_line_number, ParseErrorKind::Allocation(error))
            })?;
            try_push(
                &mut self.remaining,
                pending.rule,
                AllocationContext::ProgramRuleTable,
            )
            .map_err(|error| {
                ParseError::at_line(pending_line_number, ParseErrorKind::Allocation(error))
            })?;
        }
        self.rule_count = insertion.rule_count();
        self.once_rule_count = next_once_rule_count;
        Ok(())
    }

    fn finish(self) -> Result<Self::Output, Self::Error> {
        let Some(first) = self.first else {
            return Err(ExecutableProgramParseError::NoExecutableRules);
        };
        Ok(ExecutableRuleSet {
            first,
            remaining: self.remaining,
            rule_count: self.rule_count,
            once_rule_count: self.once_rule_count,
        })
    }
}

impl ParsedRuleSink for EmptyRuleSetBuilder {
    type Output = ();
    type Error = EmptyProgramParseError;

    /// Starts an empty target-shape sink.
    fn new() -> Self {
        Self {
            rule_count: RuleCount::new(0),
        }
    }

    /// Counts one parsed rule without retaining executable rule storage.
    fn push_parsed_rule(
        &mut self,
        parsed: ParsedRule,
        limit: RuleLimit,
    ) -> Result<(), Self::Error> {
        let line_number = parsed.line_number();
        let insertion = RuleInsertionPermit::new(self.rule_count.get(), limit, line_number)?;
        self.rule_count = insertion.rule_count();
        Ok(())
    }

    fn finish(self) -> Result<Self::Output, Self::Error> {
        if self.rule_count.get() == 0 {
            Ok(())
        } else {
            Err(EmptyProgramParseError::ExecutableRules {
                rule_count: self.rule_count,
            })
        }
    }
}

/// Computes the next parsed `(once)` count from this rule's repeat behavior.
///
/// # Errors
///
/// Returns `ParseError` if the next parsed `(once)` count cannot be
/// represented.
fn next_once_rule_count(
    current: PublicOnceRuleCount,
    parsed: &ParsedRule,
    line_number: crate::source::SourceLineNumber,
) -> Result<PublicOnceRuleCount, ParseError> {
    match parsed {
        ParsedRule::AlwaysRewrite(_) | ParsedRule::AlwaysReturn(_) => Ok(current),
        ParsedRule::OnceRewrite(_) | ParsedRule::OnceReturn(_) => {
            current.checked_next().ok_or_else(|| {
                ParseError::at_line(
                    line_number,
                    ParseErrorKind::Representation(ParseRepresentationError::RuleCount),
                )
            })
        }
    }
}

impl ExecutableRuleSet {
    /// Total executable rules in this table.
    pub(crate) const fn rule_count(&self) -> RuleCount {
        self.rule_count
    }

    /// Public count of parsed `(once)` rules.
    pub(crate) const fn once_rule_count(&self) -> PublicOnceRuleCount {
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

impl<'program> RuleScan<'program> {
    /// Iterates executable rules in parser-owned execution order.
    pub(crate) fn iter(self) -> RuleScanIter<'program> {
        RuleScanIter {
            first: Some(self.first),
            remaining: self.remaining.iter(),
        }
    }

    /// Splits this non-empty scan into its first rule and remaining rules.
    pub(crate) const fn split_first(self) -> (&'program Rule, &'program [Rule]) {
        (self.first, self.remaining)
    }
}

impl<'program> Iterator for RuleScanIter<'program> {
    type Item = &'program Rule;

    fn next(&mut self) -> Option<Self::Item> {
        match self.first.take() {
            Some(first) => Some(first),
            None => self.remaining.next(),
        }
    }
}
