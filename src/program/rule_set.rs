use alloc::vec::Vec;
use core::slice;

use crate::allocation::{AllocationContext, RequestedCapacity, try_push, try_reserve_total_exact};
use crate::error::{ParseError, ParseErrorKind, ParseLimitError, ParseRepresentationError};
use crate::inspect::{OnceRuleCount as PublicOnceRuleCount, RuleCount, RulePosition};
use crate::limits::RuleLimit;
use crate::rule::{ParsedRule, Rule};

/// Parser-built rule table before executable shape classification.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RuleSet {
    /// Parsed rules in execution order.
    rules: Vec<Rule>,
    /// Parsed `(once)` rule count assigned while building this rule table.
    once_rule_count: PublicOnceRuleCount,
}

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

/// Parser-built shape after empty/executable classification.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RuleSetShape {
    /// No executable rules were parsed.
    Empty,
    /// At least one executable rule was parsed.
    Executable(ExecutableRuleSet),
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

/// Parser-owned rule table builder.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RuleSetBuilder {
    /// Parsed rules in execution order.
    rules: Vec<Rule>,
    /// Parsed `(once)` rules seen so far.
    once_rule_count: PublicOnceRuleCount,
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
        limit: crate::limits::RuleLimit,
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

    /// Rule-table capacity required before insertion.
    const fn requested_capacity(&self) -> RequestedCapacity {
        RequestedCapacity::from_rule_count(self.attempted_rule_count)
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

impl RuleSetBuilder {
    /// Starts an empty parsed rule table.
    pub(crate) fn new() -> Self {
        Self {
            rules: Vec::new(),
            once_rule_count: PublicOnceRuleCount::ZERO,
        }
    }

    /// Stores one parsed rule and assigns its program-local position.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the rule position cannot be represented or
    /// the rule table cannot grow.
    pub(crate) fn push_parsed_rule(
        &mut self,
        parsed: ParsedRule,
        limit: RuleLimit,
    ) -> Result<(), ParseError> {
        let line_number = parsed.line_number();
        let insertion = RuleInsertionPermit::new(self.rules.len(), limit, line_number)?;
        let next_once_rule_count = self.count_parsed_once_rule(&parsed, line_number)?;

        let pending = PendingRuleInsertion::from_parsed(insertion.position(), parsed);

        let pending_line_number = pending.line_number();
        try_reserve_total_exact(
            &mut self.rules,
            insertion.requested_capacity(),
            AllocationContext::ProgramRuleTable,
        )
        .map_err(|error| {
            ParseError::at_line(pending_line_number, ParseErrorKind::Allocation(error))
        })?;
        try_push(
            &mut self.rules,
            pending.rule,
            AllocationContext::ProgramRuleTable,
        )
        .map_err(|error| {
            ParseError::at_line(pending_line_number, ParseErrorKind::Allocation(error))
        })?;
        self.once_rule_count = next_once_rule_count;
        Ok(())
    }

    /// Finalizes parsed rules into an immutable executable table.
    pub(crate) fn finish(self) -> RuleSet {
        RuleSet {
            rules: self.rules,
            once_rule_count: self.once_rule_count,
        }
    }

    /// Computes the next parsed `(once)` count from this rule's repeat behavior.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the next parsed `(once)` count cannot be
    /// represented.
    fn count_parsed_once_rule(
        &self,
        parsed: &ParsedRule,
        line_number: crate::source::SourceLineNumber,
    ) -> Result<PublicOnceRuleCount, ParseError> {
        match parsed {
            ParsedRule::AlwaysRewrite(_) | ParsedRule::AlwaysReturn(_) => Ok(self.once_rule_count),
            ParsedRule::OnceRewrite(_) | ParsedRule::OnceReturn(_) => {
                let next_once_rule_count =
                    self.once_rule_count.checked_next().ok_or_else(|| {
                        ParseError::at_line(
                            line_number,
                            ParseErrorKind::Representation(ParseRepresentationError::RuleCount),
                        )
                    })?;
                Ok(next_once_rule_count)
            }
        }
    }
}

impl RuleSet {
    /// Classifies this parser-built rule table by executable shape.
    pub(crate) fn into_shape(self) -> RuleSetShape {
        let rule_count = RuleCount::new(self.rules.len());
        let mut rules = self.rules.into_iter();
        match rules.next() {
            Some(first) => RuleSetShape::Executable(ExecutableRuleSet {
                first,
                remaining: rules.collect(),
                rule_count,
                once_rule_count: self.once_rule_count,
            }),
            None => RuleSetShape::Empty,
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
