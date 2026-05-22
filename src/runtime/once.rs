use alloc::vec::Vec;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
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

#[derive(Debug)]
pub(crate) enum MatchedRuleCommit<'once> {
    Always,
    Once(ValidOnceCommit<'once>),
}

#[derive(Debug)]
pub(crate) struct ValidOnceCommit<'once> {
    state: &'once mut OnceRuleState,
}

impl<'once> ValidOnceCommit<'once> {
    const fn new(state: &'once mut OnceRuleState) -> Self {
        Self { state }
    }

    fn commit(self) {
        *self.state = OnceRuleState::Consumed;
    }
}

impl MatchedRuleCommit<'_> {
    pub(crate) fn commit(self) {
        match self {
            Self::Always => {}
            Self::Once(commit) => commit.commit(),
        }
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
            RequestedCapacity::new(once_rule_count.get()),
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

    pub(crate) fn rule_can_commit(&self, rule: &Rule) -> bool {
        match rule.repeat_state() {
            RuleRepeatState::Always => true,
            RuleRepeatState::Once(slot) => self
                .states
                .get(slot.zero_based())
                .is_some_and(|state| state.is_fresh()),
        }
    }

    pub(crate) fn matched_commit_for_rule(&mut self, rule: &Rule) -> Option<MatchedRuleCommit<'_>> {
        match rule.repeat_state() {
            RuleRepeatState::Always => Some(MatchedRuleCommit::Always),
            RuleRepeatState::Once(slot) => {
                self.valid_once_commit(slot).map(MatchedRuleCommit::Once)
            }
        }
    }

    fn valid_once_commit(&mut self, slot: OnceRuleSlot) -> Option<ValidOnceCommit<'_>> {
        self.states
            .get_mut(slot.zero_based())
            .filter(|state| state.is_fresh())
            .map(ValidOnceCommit::new)
    }
}

impl OnceRuleState {
    const fn is_fresh(self) -> bool {
        matches!(self, Self::Fresh)
    }
}
