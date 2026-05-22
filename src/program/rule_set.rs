use alloc::vec::Vec;

use crate::allocation::{AllocationContext, AllocationError, try_push};
use crate::error::{ParseError, ParseErrorKind, ParseLimitError};
use crate::inspect::{OnceRuleCount as PublicOnceRuleCount, RuleCount, RulePosition};
use crate::rule::{OnceRuleCount, ParsedRule, Rule, RuleRepeatState, RuleRepeatSyntax};

use super::RuleLimit;

#[derive(Debug, PartialEq, Eq, Default)]
pub(crate) struct RuleSet {
    rules: Vec<Rule>,
    once_rule_count: OnceRuleCount,
}

impl RuleSet {
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
        let attempted_rule_count = self.rules.len().checked_add(1).ok_or_else(|| {
            ParseError::at_line(
                line_number,
                ParseErrorKind::Allocation(AllocationError::capacity_overflow(
                    AllocationContext::ProgramRuleTable,
                )),
            )
        })?;

        if attempted_rule_count > limit.get() {
            return Err(ParseError::at_line(
                line_number,
                ParseErrorKind::Limit(ParseLimitError::rules(
                    limit,
                    RuleCount::new(attempted_rule_count),
                )),
            ));
        }

        if RulePosition::from_zero_based(self.rules.len()).is_none() {
            return Err(ParseError::at_line(
                line_number,
                ParseErrorKind::Allocation(AllocationError::capacity_overflow(
                    AllocationContext::ProgramRuleTable,
                )),
            ));
        }

        let (repeat, next_once_rule_count) = match parsed.repeat_syntax() {
            RuleRepeatSyntax::Once => {
                let (slot, next_once_rule_count) =
                    self.once_rule_count.reserve_next_slot().ok_or_else(|| {
                        ParseError::at_line(
                            line_number,
                            ParseErrorKind::Allocation(AllocationError::capacity_overflow(
                                AllocationContext::ProgramRuleTable,
                            )),
                        )
                    })?;
                (RuleRepeatState::Once(slot), Some(next_once_rule_count))
            }
            RuleRepeatSyntax::Always => (RuleRepeatState::Always, None),
        };

        let rule = Rule::from_parsed(parsed, repeat);

        try_push(&mut self.rules, rule, AllocationContext::ProgramRuleTable)
            .map_err(|error| ParseError::at_line(line_number, ParseErrorKind::Allocation(error)))?;
        if let Some(next_once_rule_count) = next_once_rule_count {
            self.once_rule_count = next_once_rule_count;
        }
        Ok(())
    }

    pub(crate) fn rule_count(&self) -> RuleCount {
        RuleCount::new(self.rules.len())
    }

    pub(crate) fn once_rule_count(&self) -> PublicOnceRuleCount {
        PublicOnceRuleCount::new(self.once_rule_count.get())
    }

    pub(crate) const fn once_slot_count(&self) -> OnceRuleCount {
        self.once_rule_count
    }

    pub(crate) fn as_slice(&self) -> &[Rule] {
        &self.rules
    }
}
