use alloc::vec::Vec;

use crate::allocation::{AllocationContext, AllocationError, try_push, try_reserve_total_exact};
use crate::rule::{Rule, RuleRepeat};

#[derive(Debug, PartialEq, Eq)]
pub(super) struct RuntimeRules<'program> {
    entries: Vec<RuntimeRule<'program>>,
}

#[derive(Debug, PartialEq, Eq)]
struct RuntimeRule<'program> {
    rule: &'program Rule,
    availability: RuleAvailability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuleAvailability {
    Always,
    Once(OnceRuleState),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OnceRuleState {
    Fresh,
    Consumed,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum MatchedRuleCommit<'runtime> {
    Always,
    Once(&'runtime mut OnceRuleState),
}

impl<'program> RuntimeRules<'program> {
    /// Builds per-execution rule availability from parsed rules.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the per-execution rule-state table cannot
    /// be allocated.
    pub(super) fn new(rules: &'program [Rule]) -> Result<Self, AllocationError> {
        let mut entries = Vec::new();
        try_reserve_total_exact(
            &mut entries,
            rules.len(),
            AllocationContext::RuntimeOnceRuleState,
        )?;

        for rule in rules {
            try_push(
                &mut entries,
                RuntimeRule {
                    rule,
                    availability: RuleAvailability::from_repeat(rule.repeat()),
                },
                AllocationContext::RuntimeOnceRuleState,
            )?;
        }

        Ok(Self { entries })
    }

    pub(super) fn iter_available_mut(
        &mut self,
    ) -> impl Iterator<Item = (&'program Rule, MatchedRuleCommit<'_>)> {
        self.entries
            .iter_mut()
            .filter_map(RuntimeRule::available_rule)
    }
}

impl<'program> RuntimeRule<'program> {
    fn available_rule(&mut self) -> Option<(&'program Rule, MatchedRuleCommit<'_>)> {
        let commit = self.availability.commit_token()?;
        Some((self.rule, commit))
    }
}

impl RuleAvailability {
    const fn from_repeat(repeat: RuleRepeat) -> Self {
        match repeat {
            RuleRepeat::Always => Self::Always,
            RuleRepeat::Once => Self::Once(OnceRuleState::Fresh),
        }
    }

    fn commit_token(&mut self) -> Option<MatchedRuleCommit<'_>> {
        match self {
            Self::Always => Some(MatchedRuleCommit::Always),
            Self::Once(state @ OnceRuleState::Fresh) => Some(MatchedRuleCommit::Once(state)),
            Self::Once(OnceRuleState::Consumed) => None,
        }
    }
}

impl MatchedRuleCommit<'_> {
    pub(super) fn commit(self) {
        match self {
            Self::Always => {}
            Self::Once(state) => {
                *state = OnceRuleState::Consumed;
            }
        }
    }
}
