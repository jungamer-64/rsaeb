use alloc::vec::Vec;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::error::{InternalInvariantError, RunError};
use crate::rule::{OnceRuleCount, OnceRuleSlot, Rule, RuleRepeatState};

/// Internal once state set.
#[derive(Debug)]
pub(crate) struct OnceStateSet {
    /// Stored states.
    states: Vec<OnceRuleState>,
}

/// Internal once rule state alternatives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OnceRuleState {
    /// Fresh case.
    Fresh,
    /// Consumed case.
    Consumed,
}

/// Internal matched rule commit alternatives.
#[derive(Debug)]
pub(super) enum MatchedRuleCommit<'once> {
    /// Always case.
    Always,
    /// Once case.
    Once(OnceMatchPermit<'once>),
}

/// Internal once rule availability alternatives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OnceRuleAvailability {
    /// Available case.
    Available,
    /// Consumed case.
    Consumed,
}

/// Internal once match permit.
#[derive(Debug)]
pub(super) struct OnceMatchPermit<'once> {
    /// Stored state.
    state: &'once mut OnceRuleState,
}

impl<'once> OnceMatchPermit<'once> {
    /// Constructs the value from validated parts.
    const fn new(state: &'once mut OnceRuleState) -> Self {
        Self { state }
    }

    /// Runs the commit operation.
    fn commit(self) {
        *self.state = OnceRuleState::Consumed;
    }
}

impl MatchedRuleCommit<'_> {
    /// Runs the commit operation.
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

    /// Reports whether a parsed rule may currently be applied.
    ///
    /// # Errors
    ///
    /// Returns `RunError::InternalInvariant` if a parsed `(once)` rule points
    /// outside this run's once-state table.
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

    /// Returns the current state for one once-rule slot.
    ///
    /// # Errors
    ///
    /// Returns `RunError::InternalInvariant` if the slot is outside this run's
    /// once-state table.
    fn state_for_slot(&self, slot: OnceRuleSlot) -> Result<OnceRuleState, RunError> {
        self.states
            .get(slot.zero_based())
            .copied()
            .ok_or_else(|| InternalInvariantError::missing_once_rule_state().into())
    }

    /// Returns the mutable state for one once-rule slot.
    ///
    /// # Errors
    ///
    /// Returns `RunError::InternalInvariant` if the slot is outside this run's
    /// once-state table.
    fn state_for_slot_mut(&mut self, slot: OnceRuleSlot) -> Result<&mut OnceRuleState, RunError> {
        self.states
            .get_mut(slot.zero_based())
            .ok_or_else(|| InternalInvariantError::missing_once_rule_state().into())
    }
}

impl OnceRuleState {
    /// Runs the is fresh operation.
    const fn is_fresh(self) -> bool {
        matches!(self, Self::Fresh)
    }
}
