use alloc::vec::Vec;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::error::{InternalInvariantError, RunError};
use crate::rule::{OnceRuleCount, OnceRuleSlot, Rule, RuleRepeatState};

#[derive(Debug)]
pub(crate) struct OnceStateSet {
    states: Vec<OnceRuleState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OnceRuleState {
    Fresh,
    Consumed,
}

#[derive(Debug)]
pub(super) enum MatchedRuleCommit<'once> {
    Always,
    Once(OnceMatchPermit<'once>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OnceRuleAvailability {
    Available,
    Consumed,
}

#[derive(Debug)]
pub(super) struct OnceMatchPermit<'once> {
    state: &'once mut OnceRuleState,
}

impl<'once> OnceMatchPermit<'once> {
    const fn new(state: &'once mut OnceRuleState) -> Self {
        Self { state }
    }

    fn commit(self) {
        *self.state = OnceRuleState::Consumed;
    }
}

impl MatchedRuleCommit<'_> {
    pub(super) fn commit(self) {
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

    pub(super) fn availability_for_rule(
        &self,
        rule: &Rule,
    ) -> Result<OnceRuleAvailability, RunError> {
        match rule.repeat_state() {
            RuleRepeatState::Always => Ok(OnceRuleAvailability::Available),
            RuleRepeatState::Once(slot) => {
                let state = self.state_for_slot(slot)?;
                if state.is_fresh() {
                    Ok(OnceRuleAvailability::Available)
                } else {
                    Ok(OnceRuleAvailability::Consumed)
                }
            }
        }
    }

    /// Returns the unique commit permit for a rule already selected as
    /// available.
    ///
    /// # Errors
    ///
    /// Returns `RunError::InternalInvariant` if the parsed rule points outside
    /// this run's once-state table or if a consumed rule is committed without a
    /// fresh availability check.
    pub(super) fn commit_for_available_rule(
        &mut self,
        rule: &Rule,
    ) -> Result<MatchedRuleCommit<'_>, RunError> {
        match rule.repeat_state() {
            RuleRepeatState::Always => Ok(MatchedRuleCommit::Always),
            RuleRepeatState::Once(slot) => {
                let state = self.state_for_slot_mut(slot)?;
                if state.is_fresh() {
                    Ok(MatchedRuleCommit::Once(OnceMatchPermit::new(state)))
                } else {
                    Err(InternalInvariantError::consumed_once_rule_commit().into())
                }
            }
        }
    }

    fn state_for_slot(&self, slot: OnceRuleSlot) -> Result<OnceRuleState, RunError> {
        self.states
            .get(slot.zero_based())
            .copied()
            .ok_or_else(|| InternalInvariantError::missing_once_rule_state().into())
    }

    fn state_for_slot_mut(&mut self, slot: OnceRuleSlot) -> Result<&mut OnceRuleState, RunError> {
        self.states
            .get_mut(slot.zero_based())
            .ok_or_else(|| InternalInvariantError::missing_once_rule_state().into())
    }
}

impl OnceRuleState {
    const fn is_fresh(self) -> bool {
        matches!(self, Self::Fresh)
    }
}
