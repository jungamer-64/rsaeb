use alloc::vec::Vec;

use crate::allocation::{AllocationContext, AllocationError, try_push, try_reserve_total_exact};
use crate::inspect::RuleCount;
use crate::rule::{OnceRuleSlot, Rule};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct OnceStateSet {
    states: Vec<OnceRuleState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OnceRuleState {
    Fresh,
    Consumed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MatchedRuleCommit {
    Always,
    Once(OnceRuleSlot),
}

impl OnceStateSet {
    /// Builds per-execution `(once)` state from parse-time rule slots.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the per-execution once-state table cannot
    /// be allocated.
    pub(crate) fn new(once_rule_count: RuleCount) -> Result<Self, AllocationError> {
        let mut states = Vec::new();
        try_reserve_total_exact(
            &mut states,
            once_rule_count.get(),
            AllocationContext::RuntimeOnceRuleState,
        )?;

        for _ in 0..once_rule_count.get() {
            try_push(
                &mut states,
                OnceRuleState::Fresh,
                AllocationContext::RuntimeOnceRuleState,
            )?;
        }

        Ok(Self { states })
    }

    pub(crate) fn commit_token_for_rule(&self, rule: &Rule) -> Option<MatchedRuleCommit> {
        match rule.once_slot() {
            None => Some(MatchedRuleCommit::Always),
            Some(slot) => self
                .states
                .get(slot.zero_based())
                .is_some_and(OnceRuleState::is_fresh)
                .then_some(MatchedRuleCommit::Once(slot)),
        }
    }

    pub(crate) fn commit(&mut self, token: MatchedRuleCommit) {
        match token {
            MatchedRuleCommit::Always => {}
            MatchedRuleCommit::Once(slot) => {
                if let Some(state) = self.states.get_mut(slot.zero_based()) {
                    *state = OnceRuleState::Consumed;
                }
            }
        }
    }
}

impl OnceRuleState {
    const fn is_fresh(&self) -> bool {
        matches!(self, Self::Fresh)
    }
}
