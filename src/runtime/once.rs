use alloc::vec::Vec;

use crate::allocation::{AllocationContext, AllocationError, try_push, try_reserve_total_exact};
use crate::error::RuntimeInvariantError;
use crate::rule::{OnceRuleSlot, OnceRuleSlotCount, RuleSchedule};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuleEligibility {
    Eligible(MatchedRuleSchedule),
    ConsumedOnce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MatchedRuleSchedule {
    Always,
    Once(OnceRuleSlot),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnceRuleState {
    Fresh,
    Consumed,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct OnceRunStates {
    states: Vec<OnceRuleState>,
}

impl OnceRunStates {
    pub(super) fn new(once_slot_count: OnceRuleSlotCount) -> Result<Self, AllocationError> {
        let mut states = Vec::new();
        let state_count = once_slot_count.get();
        try_reserve_total_exact(
            &mut states,
            state_count,
            AllocationContext::RuntimeOnceRuleState,
        )?;

        for _ in 0..state_count {
            try_push(
                &mut states,
                OnceRuleState::Fresh,
                AllocationContext::RuntimeOnceRuleState,
            )?;
        }

        Ok(Self { states })
    }

    pub(super) fn eligibility(
        &self,
        schedule: RuleSchedule,
    ) -> Result<RuleEligibility, RuntimeInvariantError> {
        match schedule {
            RuleSchedule::Always => Ok(RuleEligibility::Eligible(MatchedRuleSchedule::Always)),
            RuleSchedule::Once(slot) => {
                let once_state_count = self.states.len();
                let state = self.states.get(slot.get()).copied().ok_or_else(|| {
                    RuntimeInvariantError::missing_once_rule_state(slot.get(), once_state_count)
                })?;

                match state {
                    OnceRuleState::Fresh => {
                        Ok(RuleEligibility::Eligible(MatchedRuleSchedule::Once(slot)))
                    }
                    OnceRuleState::Consumed => Ok(RuleEligibility::ConsumedOnce),
                }
            }
        }
    }

    pub(super) fn consume(
        &mut self,
        schedule: MatchedRuleSchedule,
    ) -> Result<(), RuntimeInvariantError> {
        match schedule {
            MatchedRuleSchedule::Always => Ok(()),
            MatchedRuleSchedule::Once(slot) => {
                let once_state_count = self.states.len();
                let state = self.states.get_mut(slot.get()).ok_or_else(|| {
                    RuntimeInvariantError::missing_once_rule_state(slot.get(), once_state_count)
                })?;

                match state {
                    OnceRuleState::Fresh => {
                        *state = OnceRuleState::Consumed;
                        Ok(())
                    }
                    OnceRuleState::Consumed => {
                        Err(RuntimeInvariantError::consumed_once_rule_slot(slot.get()))
                    }
                }
            }
        }
    }
}
