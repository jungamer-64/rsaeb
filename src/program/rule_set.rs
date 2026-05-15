use alloc::vec::Vec;

use crate::allocation::{AllocationContext, AllocationError, try_push};
use crate::rule::{OnceRuleSlotCount, ParsedRule, Rule, RuleCount, RulePosition};

#[derive(Debug, PartialEq, Eq, Default)]
pub(crate) struct RuleSet {
    rules: Vec<Rule>,
    once_slot_count: OnceRuleSlotCount,
}

impl RuleSet {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push_parsed_rule(&mut self, parsed: ParsedRule) -> Result<(), AllocationError> {
        let position = RulePosition::from_zero_based(self.rules.len()).ok_or_else(|| {
            AllocationError::capacity_overflow(AllocationContext::ProgramRuleTable)
        })?;

        let (rule, next_once_slot_count) =
            Rule::from_parsed(parsed, position, self.once_slot_count)?;

        try_push(&mut self.rules, rule, AllocationContext::ProgramRuleTable)?;

        self.once_slot_count = next_once_slot_count;
        Ok(())
    }

    pub(crate) fn rule_count(&self) -> RuleCount {
        RuleCount::new(self.rules.len())
    }

    pub(crate) fn once_rule_count(&self) -> RuleCount {
        self.once_slot_count.as_rule_count()
    }

    pub(crate) const fn once_slot_count(&self) -> OnceRuleSlotCount {
        self.once_slot_count
    }

    pub(crate) fn rule_at_position(&self, position: RulePosition) -> Option<&Rule> {
        self.rules.get(position.zero_based())
    }

    pub(crate) fn as_slice(&self) -> &[Rule] {
        &self.rules
    }
}
