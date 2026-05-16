use alloc::vec::Vec;

use crate::allocation::{AllocationContext, AllocationError, try_push, try_reserve_total_exact};
use crate::rule::{OnceRuleCount, OnceRuleSlot, Rule, RuleRepeatState};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct OnceStateSet {
    states: Vec<OnceRuleState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OnceRuleState {
    Fresh,
    Consumed,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MatchedRuleCommit {
    Always,
    Once(ValidOnceCommit),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidOnceCommit {
    slot: OnceRuleSlot,
}

impl ValidOnceCommit {
    const fn new(slot: OnceRuleSlot) -> Self {
        Self { slot }
    }

    const fn slot(&self) -> OnceRuleSlot {
        self.slot
    }
}

impl OnceStateSet {
    /// Builds per-execution `(once)` state from parse-time rule slots.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the per-execution once-state table cannot
    /// be allocated.
    pub(crate) fn new(once_rule_count: OnceRuleCount) -> Result<Self, AllocationError> {
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
        match rule.repeat_state() {
            RuleRepeatState::Always => Some(MatchedRuleCommit::Always),
            RuleRepeatState::Once(slot) => {
                self.valid_once_commit(slot).map(MatchedRuleCommit::Once)
            }
        }
    }

    pub(crate) fn commit(&mut self, token: MatchedRuleCommit) {
        match token {
            MatchedRuleCommit::Always => {}
            MatchedRuleCommit::Once(commit) => {
                if let Some(state) = self.states.get_mut(commit.slot().zero_based()) {
                    *state = OnceRuleState::Consumed;
                }
            }
        }
    }

    fn valid_once_commit(&self, slot: OnceRuleSlot) -> Option<ValidOnceCommit> {
        self.states
            .get(slot.zero_based())
            .copied()
            .is_some_and(OnceRuleState::is_fresh)
            .then_some(ValidOnceCommit::new(slot))
    }
}

impl OnceRuleState {
    const fn is_fresh(self) -> bool {
        matches!(self, Self::Fresh)
    }
}
