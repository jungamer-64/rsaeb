use alloc::vec::Vec;

use crate::allocation::{AllocationContext, AllocationError, try_push};
use crate::rule::{ParsedRule, Rule, RuleCount, RulePosition, RuleRepeat};

#[derive(Debug, PartialEq, Eq, Default)]
pub(crate) struct RuleSet {
    rules: Vec<Rule>,
}

impl RuleSet {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push_parsed_rule(&mut self, parsed: ParsedRule) -> Result<(), AllocationError> {
        let position = RulePosition::from_zero_based(self.rules.len()).ok_or_else(|| {
            AllocationError::capacity_overflow(AllocationContext::ProgramRuleTable)
        })?;

        let rule = Rule::from_parsed(parsed, position);

        try_push(&mut self.rules, rule, AllocationContext::ProgramRuleTable)?;
        Ok(())
    }

    pub(crate) fn rule_count(&self) -> RuleCount {
        RuleCount::new(self.rules.len())
    }

    pub(crate) fn once_rule_count(&self) -> RuleCount {
        RuleCount::new(
            self.rules
                .iter()
                .filter(|rule| rule.repeat() == RuleRepeat::Once)
                .count(),
        )
    }

    pub(crate) fn as_slice(&self) -> &[Rule] {
        &self.rules
    }
}
