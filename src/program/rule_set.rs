use alloc::vec::Vec;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::error::{ParseError, ParseErrorKind, ParseLimitError};
use crate::inspect::{OnceRuleCount as PublicOnceRuleCount, RuleCount, RuleTableIndex};
use crate::limits::RuleLimit;
use crate::rule::{OnceRuleCount, ParsedRule, Rule, RuleRepeatState, RuleRepeatSyntax};

/// Immutable executable rule table built by the parser.
#[derive(Debug, PartialEq, Eq, Default)]
pub(crate) struct RuleSet {
    /// Parsed rules in execution order.
    rules: Vec<Rule>,
    /// Number of once slots needed by one run.
    once_rule_count: OnceRuleCount,
}

/// Parsed rule after repeat-state assignment but before table insertion.
struct PendingRuleInsertion {
    /// Rule ready for storage in execution order.
    rule: Rule,
    /// Once-slot count after this rule is accepted.
    next_once_rule_count: OnceRuleCount,
}

/// Checked permission to insert one parsed rule into the table.
struct RuleInsertionPermit {
    /// Rule count after the permitted insertion.
    attempted_rule_count: RuleCount,
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
                ParseErrorKind::Allocation(AllocationError::capacity_overflow(
                    AllocationContext::ProgramRuleTable,
                )),
            )
        })?;
        let attempted_rule_count = RuleCount::new(attempted_rule_count);

        if !limit.accepts(attempted_rule_count) {
            return Err(ParseError::at_line(
                line_number,
                ParseErrorKind::Limit(ParseLimitError::rules(limit, attempted_rule_count)),
            ));
        }

        if RuleTableIndex::from_zero_based(current_len).is_none()
            || crate::inspect::RulePosition::from_zero_based(current_len).is_none()
        {
            return Err(ParseError::at_line(
                line_number,
                ParseErrorKind::Allocation(AllocationError::capacity_overflow(
                    AllocationContext::ProgramRuleTable,
                )),
            ));
        }

        Ok(Self {
            attempted_rule_count,
        })
    }

    /// Rule-table capacity required before insertion.
    const fn requested_capacity(self) -> RequestedCapacity {
        RequestedCapacity::from_rule_count(self.attempted_rule_count)
    }
}

impl PendingRuleInsertion {
    /// Assigns runtime repeat state to one parsed rule before storage.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if assigning the next `(once)` slot overflows.
    fn from_parsed(
        parsed: ParsedRule,
        current_once_rule_count: OnceRuleCount,
    ) -> Result<Self, ParseError> {
        let line_number = parsed.line_number();
        let (repeat, next_once_rule_count) = match parsed.repeat_syntax() {
            RuleRepeatSyntax::Once => {
                let (slot, next_once_rule_count) =
                    current_once_rule_count.reserve_next_slot().ok_or_else(|| {
                        ParseError::at_line(
                            line_number,
                            ParseErrorKind::Allocation(AllocationError::capacity_overflow(
                                AllocationContext::ProgramRuleTable,
                            )),
                        )
                    })?;
                (RuleRepeatState::Once(slot), next_once_rule_count)
            }
            RuleRepeatSyntax::Always => (RuleRepeatState::Always, current_once_rule_count),
        };

        Ok(Self {
            rule: Rule::from_parsed(parsed, repeat),
            next_once_rule_count,
        })
    }

    /// Source line used if storing this rule fails.
    const fn line_number(&self) -> crate::source::SourceLineNumber {
        self.rule.line_number()
    }
}

impl RuleSet {
    /// Starts an empty parsed rule table.
    pub(crate) fn new() -> Self {
        Self::default()
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

        let pending = PendingRuleInsertion::from_parsed(parsed, self.once_rule_count)?;

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
        self.once_rule_count = pending.next_once_rule_count;
        Ok(())
    }

    /// Total executable rules in this table.
    pub(crate) fn rule_count(&self) -> RuleCount {
        RuleCount::new(self.rules.len())
    }

    /// Public count of parsed `(once)` rules.
    pub(crate) fn once_rule_count(&self) -> PublicOnceRuleCount {
        PublicOnceRuleCount::new(self.once_rule_count.get())
    }

    /// Internal once-slot count used to initialize per-run state.
    pub(crate) const fn once_slot_count(&self) -> OnceRuleCount {
        self.once_rule_count
    }

    /// Borrows rules in execution order.
    pub(crate) fn as_slice(&self) -> &[Rule] {
        &self.rules
    }
}
