use alloc::vec::Vec;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::error::{RunError, RunInvariantError};
use crate::inspect::OnceRuleCount as PublicOnceRuleCount;
use crate::rule::{OnceRuleCount, Rule, RuleAvailability};

/// Per-run execution state for parsed `(once)` slots.
#[derive(Debug)]
pub(crate) struct OnceStateSet {
    /// One runtime state cell for each parsed `(once)` rule.
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

/// Availability of a parsed rule according to per-run `(once)` state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OnceRuleAvailability {
    /// Rule is available to be matched against runtime state.
    Available,
    /// Rule has already committed during this runtime invocation.
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
    /// Builds per-execution once-slot state directly from parsed rules.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the per-execution once-state table cannot
    /// be allocated.
    pub(crate) fn new(rules: &[Rule]) -> Result<Self, AllocationError> {
        let once_rule_count = OnceRuleCount::new(
            rules
                .iter()
                .filter(|rule| rule.availability().is_once())
                .count(),
        );
        let mut states = Vec::new();
        try_reserve_total_exact(
            &mut states,
            RequestedCapacity::from_once_rule_count(once_rule_count),
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

    /// Returns this rule's current per-run once-state availability.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if a parsed `(once)` rule references a runtime slot
    /// missing from this run's once-state table.
    pub(super) fn availability(&self, rule: &Rule) -> Result<OnceRuleAvailability, RunError> {
        match rule.availability() {
            RuleAvailability::Always => Ok(OnceRuleAvailability::Available),
            RuleAvailability::Once(slot) => {
                let available_slots = PublicOnceRuleCount::new(self.states.len());
                let Some(state) = self.states.get(slot.get()) else {
                    return Err(RunInvariantError::MissingOnceRuleState {
                        rule: rule.position(),
                        available_slots,
                    }
                    .into());
                };

                match state {
                    OnceRuleState::Fresh => Ok(OnceRuleAvailability::Available),
                    OnceRuleState::Consumed => Ok(OnceRuleAvailability::Consumed),
                }
            }
        }
    }

    /// Reserves the commit side effect for a rule already proven available.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if a parsed `(once)` rule references a runtime slot
    /// missing from this run's once-state table.
    pub(super) fn reserve_available_commit(
        &mut self,
        rule: &Rule,
    ) -> Result<MatchedRuleCommit<'_>, RunError> {
        match rule.availability() {
            RuleAvailability::Always => Ok(MatchedRuleCommit::Always),
            RuleAvailability::Once(slot) => {
                let available_slots = PublicOnceRuleCount::new(self.states.len());
                let Some(state) = self.states.get_mut(slot.get()) else {
                    return Err(RunInvariantError::MissingOnceRuleState {
                        rule: rule.position(),
                        available_slots,
                    }
                    .into());
                };

                match state {
                    OnceRuleState::Fresh => {
                        Ok(MatchedRuleCommit::Once(OnceMatchPermit::new(state)))
                    }
                    OnceRuleState::Consumed => Err(RunInvariantError::MissingOnceRuleState {
                        rule: rule.position(),
                        available_slots,
                    }
                    .into()),
                }
            }
        }
    }
}
