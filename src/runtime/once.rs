use alloc::vec::Vec;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::error::{InternalInvariantError, RunError};
use crate::rule::{OnceRuleCount, OnceRuleSlot, Rule, RuleRepeatState};

/// Per-run consumption table for parsed `(once)` rules.
#[derive(Debug)]
pub(crate) struct OnceStateSet {
    /// Consumption state indexed by parsed once-rule slots.
    states: Vec<OnceRuleState>,
}

/// Consumption state for one `(once)` rule during a single run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OnceRuleState {
    /// Rule has not committed in this run.
    Fresh,
    /// Rule has already committed in this run.
    Consumed,
}

/// Linear commit action for a matched rule.
#[derive(Debug)]
pub(super) enum MatchedRuleCommit<'once> {
    /// Rule has no once-state side effect.
    Always,
    /// Rule owns the unique permit to consume its once state.
    Once(OnceMatchPermit<'once>),
}

/// Availability of a rule before matching work is attempted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OnceRuleAvailability {
    /// Rule may be considered by the matcher.
    Available,
    /// Rule has already committed in this run.
    Consumed,
}

/// Unique mutable permit that consumes one fresh once-rule state on commit.
#[derive(Debug)]
pub(super) struct OnceMatchPermit<'once> {
    /// Fresh once-state cell reserved for the matched rule.
    state: &'once mut OnceRuleState,
}

impl<'once> OnceMatchPermit<'once> {
    /// Creates the commit permit after availability has been checked.
    const fn new(state: &'once mut OnceRuleState) -> Self {
        Self { state }
    }

    /// Marks the reserved once-rule state as consumed.
    fn commit(self) {
        *self.state = OnceRuleState::Consumed;
    }
}

impl MatchedRuleCommit<'_> {
    /// Applies the rule's once-state side effect after rewrite success.
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
    /// Whether this once slot can still be matched.
    const fn is_fresh(self) -> bool {
        matches!(self, Self::Fresh)
    }
}
