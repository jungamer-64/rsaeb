use alloc::vec::Vec;

use crate::allocation::{AllocationContext, RequestedCapacity, try_push, try_reserve_total_exact};
use crate::error::{
    ParseError, ParseErrorKind, ParseLimitError, ParseRepresentationError, RuleAttemptCursorError,
};
use crate::inspect::{OnceRuleCount as PublicOnceRuleCount, RuleCount, RulePosition};
use crate::limits::RuleLimit;
use crate::rule::{OnceRuleSlot, ParsedRule, Rule, RuleAvailability, RuleRepeatSyntax};

/// Immutable executable rule table built by the parser.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RuleSet {
    /// Parsed rules in execution order.
    rules: Vec<Rule>,
    /// Parsed `(once)` slot count assigned while building this rule table.
    once_rule_count: PublicOnceRuleCount,
}

/// Parser-owned rule table builder.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RuleSetBuilder {
    /// Parsed rules in execution order.
    rules: Vec<Rule>,
    /// Next `(once)` slot to assign.
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

/// Zero-based executable rule index minted from a concrete rule set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RuleIndex {
    /// Zero-based rule-table offset.
    zero_based: usize,
    /// Public one-based rule position for diagnostics.
    position: RulePosition,
}

/// Cursor pointing to the next executable rule line in one rule-attempt run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleCursor {
    /// Cursor points at the next executable rule index.
    Active(ActiveRuleCursor),
    /// No executable rule remains in this pass.
    Exhausted,
}

/// Active cursor state for rule-attempt execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ActiveRuleCursor {
    /// Zero-based rule index to evaluate next.
    next_rule_index: RuleIndex,
    /// Final executable rule index in this program.
    final_rule_index: RuleIndex,
}

/// Cursor movement after a non-applying rule line has been consumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleCursorAfterMiss {
    /// Cursor advanced to the next executable rule.
    Advanced(ActiveRuleCursor),
    /// The consumed miss was the final executable rule.
    Stable,
}

/// Rule target selected by a rule-attempt cursor.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RuleTarget<'program> {
    /// Parsed rule selected by the cursor.
    rule: &'program Rule,
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

        if !limit.accepts(attempted_rule_count) {
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

    /// Starts rule-attempt execution over this table's executable rules.
    pub(crate) fn rule_attempt_cursor(&self) -> RuleCursor {
        let Some(final_rule_index) = RuleIndex::last_for(self.rule_count()) else {
            return RuleCursor::Exhausted;
        };

        RuleCursor::Active(ActiveRuleCursor {
            next_rule_index: RuleIndex::first(),
            final_rule_index,
        })
    }

    /// Selects the parsed rule pointed at by an active rule-attempt cursor.
    ///
    /// # Errors
    ///
    /// Returns `RuleAttemptCursorError` if the cursor points outside this parsed
    /// rule table.
    pub(crate) fn target_for_cursor(
        &self,
        active_cursor: ActiveRuleCursor,
    ) -> Result<RuleTarget<'_>, RuleAttemptCursorError> {
        let rule = self
            .rules
            .get(active_cursor.current_index().get())
            .ok_or_else(|| {
                RuleAttemptCursorError::missing_rule(active_cursor.current_position())
            })?;
        Ok(RuleTarget { rule })
    }
}

impl RuleCursor {
    /// Takes the active cursor state, leaving this cursor exhausted until the attempt commits.
    pub(crate) fn take_active(&mut self) -> Option<ActiveRuleCursor> {
        match core::mem::replace(self, Self::Exhausted) {
            Self::Active(active) => Some(active),
            Self::Exhausted => None,
        }
    }
}

impl ActiveRuleCursor {
    /// Current zero-based rule index.
    pub(crate) const fn current_index(self) -> RuleIndex {
        self.next_rule_index
    }

    /// Current public rule position.
    pub(crate) const fn current_position(self) -> RulePosition {
        self.next_rule_index.position()
    }

    /// Advances after a miss or reports that the pass is stable.
    pub(crate) fn advance_after_miss(self) -> RuleCursorAfterMiss {
        if self.next_rule_index >= self.final_rule_index {
            return RuleCursorAfterMiss::Stable;
        }

        if let Some(next_rule_index) = self.next_rule_index.checked_next() {
            RuleCursorAfterMiss::Advanced(Self {
                next_rule_index,
                final_rule_index: self.final_rule_index,
            })
        } else {
            RuleCursorAfterMiss::Stable
        }
    }

    /// Resets to the first executable rule after a committed match.
    pub(crate) const fn reset_to_first(self) -> Self {
        Self {
            next_rule_index: RuleIndex::first(),
            final_rule_index: self.final_rule_index,
        }
    }
}

impl RuleIndex {
    /// First executable rule index.
    const fn first() -> Self {
        Self {
            zero_based: 0,
            position: RulePosition::FIRST,
        }
    }

    /// Builds an index from a zero-based rule-table offset.
    fn from_zero_based(zero_based: usize) -> Option<Self> {
        let position = RulePosition::from_zero_based(zero_based)?;
        Some(Self {
            zero_based,
            position,
        })
    }

    /// Final executable rule index for a parsed rule count.
    fn last_for(rule_count: RuleCount) -> Option<Self> {
        let zero_based = rule_count.get().checked_sub(1)?;
        Self::from_zero_based(zero_based)
    }

    /// Returns the checked next index.
    fn checked_next(self) -> Option<Self> {
        let zero_based = self.zero_based.checked_add(1)?;
        Self::from_zero_based(zero_based)
    }

    /// Zero-based rule-table offset.
    pub(crate) const fn get(self) -> usize {
        self.zero_based
    }

    /// Public one-based rule position.
    const fn position(self) -> RulePosition {
        self.position
    }
}

impl<'program> RuleTarget<'program> {
    /// Parsed rule selected by the cursor.
    pub(crate) const fn rule(self) -> &'program Rule {
        self.rule
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::RuleAttemptStepError;
    use crate::test_support::{TestFailure, TestResult, ensure_eq};

    /// # Errors
    ///
    /// Returns `TestFailure` if an active cursor pointing outside the rule table
    /// is folded into an executable-rule absence instead of a cursor error.
    #[test]
    fn missing_cursor_rule_is_rule_attempt_step_error() -> TestResult {
        let rule_set = RuleSet {
            rules: Vec::new(),
            once_rule_count: PublicOnceRuleCount::ZERO,
        };
        let cursor = ActiveRuleCursor {
            next_rule_index: RuleIndex::first(),
            final_rule_index: RuleIndex::first(),
        };

        let Err(RuleAttemptStepError::RuleCursor(error)) = rule_set
            .target_for_cursor(cursor)
            .map_err(RuleAttemptStepError::from)
        else {
            return Err(TestFailure::message("expected missing cursor rule error"));
        };

        ensure_eq!(error.rule(), RulePosition::FIRST)
    }
}
