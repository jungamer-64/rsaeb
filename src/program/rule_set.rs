use alloc::vec::Vec;
use core::slice;

use crate::allocation::{AllocationContext, RequestedCapacity, try_push, try_reserve_total_exact};
use crate::error::{ParseError, ParseErrorKind, ParseLimitError, ParseRepresentationError};
use crate::inspect::{OnceRuleCount as PublicOnceRuleCount, RuleCount, RulePosition};
use crate::limits::RuleLimit;
use crate::rule::{ParsedRule, Rule, RuleRepeatBehavior, RuleRepeatSyntax};

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

/// Cursor pointing to the next executable rule line in one rule-attempt run.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RuleCursor {
    /// Zero-based rule-table offset selected on the next attempt.
    next_rule_index: usize,
}

/// Cursor movement after a non-applying rule line has been consumed.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RuleCursorAfterMiss {
    /// Cursor advanced to the next executable rule.
    Advanced(RuleCursor),
    /// The consumed miss was the final executable rule.
    Stable,
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
    /// Assigns runtime repeat behavior to one parsed rule before storage.
    fn from_parsed(
        position: RulePosition,
        parsed: ParsedRule,
        repeat_behavior: RuleRepeatBehavior,
    ) -> Self {
        Self {
            rule: Rule::from_parsed(position, parsed, repeat_behavior),
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
        let (repeat_behavior, next_once_rule_count) =
            self.assign_rule_repeat_behavior(&parsed, line_number)?;

        let pending =
            PendingRuleInsertion::from_parsed(insertion.position(), parsed, repeat_behavior);

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

    /// Assigns parsed repeat syntax to runtime repeat behavior.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the next parsed `(once)` count cannot be
    /// represented.
    fn assign_rule_repeat_behavior(
        &self,
        parsed: &ParsedRule,
        line_number: crate::source::SourceLineNumber,
    ) -> Result<(RuleRepeatBehavior, PublicOnceRuleCount), ParseError> {
        match parsed.repeat_syntax() {
            RuleRepeatSyntax::Always => Ok((RuleRepeatBehavior::Always, self.once_rule_count)),
            RuleRepeatSyntax::Once => {
                let next_once_rule_count =
                    self.once_rule_count.checked_next().ok_or_else(|| {
                        ParseError::at_line(
                            line_number,
                            ParseErrorKind::Representation(ParseRepresentationError::RuleCount),
                        )
                    })?;
                Ok((RuleRepeatBehavior::Once, next_once_rule_count))
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

    /// Number of executable rules in this scan.
    pub(crate) fn rule_count(self) -> RuleCount {
        RuleCount::new(self.rules.len())
    }

    /// Returns the rule at a cursor position.
    pub(crate) fn rule_at_cursor(self, cursor: &RuleCursor) -> Option<&'program Rule> {
        self.rules.get(cursor.next_rule_index)
    }

    /// Cursor movement allowed after the current cursor consumes a miss.
    pub(crate) fn after_miss(self, cursor: &RuleCursor) -> RuleCursorAfterMiss {
        let next_index = cursor.next_rule_index.saturating_add(1);
        if next_index < self.rules.len() {
            RuleCursorAfterMiss::Advanced(RuleCursor {
                next_rule_index: next_index,
            })
        } else {
            RuleCursorAfterMiss::Stable
        }
    }
}

impl RuleCursor {
    /// First executable rule cursor for a fresh pass.
    pub(crate) const fn first() -> Self {
        Self { next_rule_index: 0 }
    }

    /// Zero-based rule-table offset selected on the next attempt.
    pub(crate) const fn next_rule_index(&self) -> usize {
        self.next_rule_index
    }
}
