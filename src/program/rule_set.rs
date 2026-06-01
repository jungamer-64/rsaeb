use alloc::vec::Vec;
use core::slice;

use crate::allocation::{AllocationContext, RequestedCapacity, try_push, try_reserve_total_exact};
use crate::error::{ParseError, ParseErrorKind, ParseLimitError, ParseRepresentationError};
use crate::inspect::{OnceRuleCount as PublicOnceRuleCount, RuleCount, RulePosition};
use crate::limits::RuleLimit;
use crate::rule::{OnceRuleSlot, ParsedRule, Rule, RuleAvailability, RuleRepeatSyntax};

/// Immutable executable rule table built by the parser.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RuleSet {
    /// Parsed rules in execution order.
    rules: Vec<Rule>,
    /// Parsed `(once)` rule count assigned while building this rule table.
    once_rule_count: PublicOnceRuleCount,
}

/// Borrowed executable rule scan minted from one parsed rule table.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RuleScan<'program> {
    /// Parsed executable rules in execution order.
    rules: &'program [Rule],
}

/// Parser-owned rule table builder.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RuleSetBuilder {
    /// Parsed rules in execution order.
    rules: Vec<Rule>,
    /// Parsed `(once)` rules seen so far.
    once_rule_count: PublicOnceRuleCount,
}

/// Parsed rule after repeat-state assignment but before table insertion.
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

/// Cursor pointing to an executable rule line in one active rule-attempt run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ActiveRuleCursor<'program> {
    /// Parsed executable rule selected on the next attempt.
    current: &'program Rule,
    /// Remaining parsed rules after the current target.
    remaining_after_current: &'program [Rule],
}

/// Cursor movement after a non-applying rule line has been consumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleCursorAfterMiss<'program> {
    /// Cursor advanced to the next executable rule.
    Advanced(ActiveRuleCursor<'program>),
    /// The consumed miss was the final executable rule.
    Exhausted,
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
    /// Assigns runtime availability to one parsed rule before storage.
    fn from_parsed(
        position: RulePosition,
        parsed: ParsedRule,
        availability: RuleAvailability,
    ) -> Self {
        Self {
            rule: Rule::from_parsed(position, parsed, availability),
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
        let (availability, next_once_rule_count) =
            self.assign_rule_availability(&parsed, line_number)?;

        let pending = PendingRuleInsertion::from_parsed(insertion.position(), parsed, availability);

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

    /// Assigns parsed repeat syntax to runtime availability.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the next parsed `(once)` count cannot be
    /// represented.
    fn assign_rule_availability(
        &self,
        parsed: &ParsedRule,
        line_number: crate::source::SourceLineNumber,
    ) -> Result<(RuleAvailability, PublicOnceRuleCount), ParseError> {
        match parsed.repeat_syntax() {
            RuleRepeatSyntax::Always => Ok((RuleAvailability::Always, self.once_rule_count)),
            RuleRepeatSyntax::Once => {
                let slot = OnceRuleSlot::from_count(self.once_rule_count);
                let next_once_rule_count =
                    self.once_rule_count.checked_next().ok_or_else(|| {
                        ParseError::at_line(
                            line_number,
                            ParseErrorKind::Representation(ParseRepresentationError::RuleCount),
                        )
                    })?;
                Ok((RuleAvailability::Once(slot), next_once_rule_count))
            }
        }
    }
}

impl RuleSet {
    /// Total executable rules in this table.
    pub(crate) fn rule_count(&self) -> RuleCount {
        RuleCount::new(self.rules.len())
    }

    /// Public count of parsed `(once)` rules.
    pub(crate) fn once_rule_count(&self) -> PublicOnceRuleCount {
        self.once_rule_count
    }

    /// Borrows rules in execution order.
    pub(crate) fn as_slice(&self) -> &[Rule] {
        &self.rules
    }

    /// Starts a runtime scan over this table's executable rules.
    pub(crate) fn scan(&self) -> RuleScan<'_> {
        RuleScan { rules: &self.rules }
    }
}

impl<'program> RuleScan<'program> {
    /// Iterates executable rules in parser-owned execution order.
    pub(crate) fn iter(self) -> slice::Iter<'program, Rule> {
        self.rules.iter()
    }

    /// Mints the first active rule cursor for a non-empty rule table.
    pub(crate) fn first_cursor(self) -> Option<ActiveRuleCursor<'program>> {
        ActiveRuleCursor::from_rules(self.rules)
    }

    /// Splits a cursor into the selected rule and the only legal post-miss cursor movement.
    pub(crate) fn consume_cursor(
        self,
        cursor: ActiveRuleCursor<'program>,
    ) -> (&'program Rule, RuleCursorAfterMiss<'program>) {
        cursor.into_target()
    }
}

impl<'program> ActiveRuleCursor<'program> {
    /// Builds a cursor from a non-empty parsed rule slice.
    fn from_rules(rules: &'program [Rule]) -> Option<Self> {
        let (current, remaining_after_current) = rules.split_first()?;
        Some(Self {
            current,
            remaining_after_current,
        })
    }

    /// Splits this cursor into its selected rule and miss continuation.
    fn into_target(self) -> (&'program Rule, RuleCursorAfterMiss<'program>) {
        let after_miss = match Self::from_rules(self.remaining_after_current) {
            Some(cursor) => RuleCursorAfterMiss::Advanced(cursor),
            None => RuleCursorAfterMiss::Exhausted,
        };
        (self.current, after_miss)
    }
}
