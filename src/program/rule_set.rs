use alloc::vec::Vec;

use crate::allocation::{AllocationContext, RequestedCapacity, try_push, try_reserve_total_exact};
use crate::error::{ParseError, ParseErrorKind, ParseLimitError, ParseRepresentationError};
use crate::inspect::{OnceRuleCount as PublicOnceRuleCount, RuleCount, RulePosition};
use crate::limits::RuleLimit;
use crate::rule::{OnceRuleCount, ParsedRule, Rule, RuleAvailability, RuleRepeatSyntax};

/// Immutable executable rule table built by the parser.
#[derive(Debug, PartialEq, Eq, Default)]
pub(crate) struct RuleSet {
    /// Parsed rules in execution order.
    rules: Vec<Rule>,
    /// Parser-assigned once-slot count for one runtime invocation.
    once_rule_count: OnceRuleCount,
}

/// Parser-owned rule table builder.
#[derive(Debug, PartialEq, Eq, Default)]
pub(crate) struct RuleSetBuilder {
    /// Parsed rules in execution order.
    rules: Vec<Rule>,
    /// Number of once slots needed by one run.
    next_once_rule_count: OnceRuleCount,
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
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if assigning the next `(once)` slot overflows.
    fn from_parsed(
        position: RulePosition,
        parsed: ParsedRule,
        next_once_rule_count: OnceRuleCount,
    ) -> Result<Self, ParseError> {
        let line_number = parsed.line_number();
        let (availability, next_once_rule_count) = match parsed.repeat_syntax() {
            RuleRepeatSyntax::Once => {
                let (slot, next_once_rule_count) =
                    next_once_rule_count.assign_next_slot().ok_or_else(|| {
                        ParseError::at_line(
                            line_number,
                            ParseErrorKind::Representation(ParseRepresentationError::OnceRuleCount),
                        )
                    })?;
                (RuleAvailability::Once(slot), next_once_rule_count)
            }
            RuleRepeatSyntax::Always => (RuleAvailability::Always, next_once_rule_count),
        };

        Ok(Self {
            rule: Rule::from_parsed(position, parsed, availability),
            next_once_rule_count,
        })
    }

    /// Source line used if storing this rule fails.
    const fn line_number(&self) -> crate::source::SourceLineNumber {
        self.rule.line_number()
    }
}

impl RuleSetBuilder {
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

        let pending = PendingRuleInsertion::from_parsed(
            insertion.position(),
            parsed,
            self.next_once_rule_count,
        )?;

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
        self.next_once_rule_count = pending.next_once_rule_count;
        Ok(())
    }

    /// Finalizes parsed rules into an immutable executable table.
    pub(crate) fn finish(self) -> RuleSet {
        RuleSet {
            rules: self.rules,
            once_rule_count: self.next_once_rule_count,
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
        PublicOnceRuleCount::new(self.once_rule_count.get())
    }

    /// Runtime slot count required for one execution.
    pub(crate) const fn once_rule_slot_count(&self) -> OnceRuleCount {
        self.once_rule_count
    }

    /// Borrows rules in execution order.
    pub(crate) fn as_slice(&self) -> &[Rule] {
        &self.rules
    }
}
