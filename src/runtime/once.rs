use alloc::vec::Vec;
use core::cell::Cell;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::rule::{OnceRuleCount, OnceRuleSlot, Rule, RuleRepeatState};

#[derive(Debug)]
pub(crate) struct OnceStateSet {
    states: Vec<Cell<OnceRuleState>>,
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
pub(crate) enum RuleAvailability<'once> {
    Available(MatchedRuleCommit<'once>),
    Unavailable(OnceRuleUnavailable),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OnceRuleUnavailable {
    Consumed,
    MissingSlot,
}

#[derive(Debug)]
pub(crate) struct ValidOnceCommit<'once> {
    state: &'once Cell<OnceRuleState>,
}

impl<'once> ValidOnceCommit<'once> {
    const fn new(state: &'once Cell<OnceRuleState>) -> Self {
        Self { state }
    }

    fn commit(self) {
        self.state.set(OnceRuleState::Consumed);
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
                Cell::new(OnceRuleState::Fresh),
                AllocationContext::RuntimeOnceRuleState,
            )?;
        }

        Ok(Self { states })
    }

    pub(crate) fn availability_for_rule(&self, rule: &Rule) -> RuleAvailability<'_> {
        match rule.repeat_state() {
            RuleRepeatState::Always => RuleAvailability::Available(MatchedRuleCommit::Always),
            RuleRepeatState::Once(slot) => match self.once_commit_for_slot(slot) {
                Ok(commit) => RuleAvailability::Available(MatchedRuleCommit::Once(commit)),
                Err(reason) => RuleAvailability::Unavailable(reason),
            },
        }
    }

    /// Returns the commit witness for a concrete `(once)` slot.
    ///
    /// # Errors
    ///
    /// Returns `OnceRuleUnavailable::MissingSlot` if the parsed rule points
    /// outside this run's once-state table, or `OnceRuleUnavailable::Consumed`
    /// if the slot has already committed in this run.
    fn once_commit_for_slot(
        &self,
        slot: OnceRuleSlot,
    ) -> Result<ValidOnceCommit<'_>, OnceRuleUnavailable> {
        let state = self
            .states
            .get(slot.zero_based())
            .ok_or(OnceRuleUnavailable::MissingSlot)?;

        if state.get().is_fresh() {
            Ok(ValidOnceCommit::new(state))
        } else {
            Err(OnceRuleUnavailable::Consumed)
        }
    }
}

impl OnceRuleState {
    const fn is_fresh(self) -> bool {
        matches!(self, Self::Fresh)
    }
}
