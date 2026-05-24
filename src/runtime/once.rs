use alloc::vec::Vec;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::error::{RunError, RunInvariantError};
use crate::inspect::OnceRuleCount as PublicOnceRuleCount;
use crate::rule::{OnceRuleCount, OnceRuleSlot, Rule, RuleAvailability};

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

/// Linear commit action for a matched rule.
#[derive(Debug)]
pub(super) enum MatchedRuleCommit {
    /// Rule has no once-state side effect.
    Always,
    /// Rule owns the unique permit to consume its once state.
    Once(OnceMatchPermit),
}

/// Availability of a parsed rule together with the only valid commit path.
#[derive(Debug)]
pub(super) enum OnceRuleReadiness {
    /// Rule is available and carries the commit action for a later successful application.
    Available(MatchedRuleCommit),
    /// Rule has already committed during this runtime invocation.
    Consumed,
}

/// Private permit that consumes one fresh once-rule state on commit.
#[derive(Debug)]
pub(super) struct OnceMatchPermit {
    /// Parsed rule owning this once slot.
    rule: crate::inspect::RulePosition,
    /// Fresh once-state slot reserved for the matched rule.
    slot: OnceRuleSlot,
}

impl OnceMatchPermit {
    /// Creates the commit permit after availability has been checked.
    const fn new(rule: crate::inspect::RulePosition, slot: OnceRuleSlot) -> Self {
        Self { rule, slot }
    }
}

impl MatchedRuleCommit {
    /// Applies the rule's once-state side effect after rewrite success.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if the permit points at a missing runtime once-state
    /// slot.
    pub(super) fn commit(self, once_states: &mut OnceStateSet) -> Result<(), RunError> {
        match self {
            Self::Always => Ok(()),
            Self::Once(commit) => once_states.commit_once(commit),
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

    /// Returns this rule's current per-run readiness and commit action.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if a parsed `(once)` rule references a runtime slot
    /// missing from this run's once-state table.
    pub(super) fn readiness_for_rule(&self, rule: &Rule) -> Result<OnceRuleReadiness, RunError> {
        match rule.availability() {
            RuleAvailability::Always => Ok(OnceRuleReadiness::Available(MatchedRuleCommit::Always)),
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
                    OnceRuleState::Fresh => Ok(OnceRuleReadiness::Available(
                        MatchedRuleCommit::Once(OnceMatchPermit::new(rule.position(), slot)),
                    )),
                    OnceRuleState::Consumed => Ok(OnceRuleReadiness::Consumed),
                }
            }
        }
    }

    /// Commits a previously minted once-rule permit.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if the permit points at a missing runtime once-state
    /// slot.
    fn commit_once(&mut self, permit: OnceMatchPermit) -> Result<(), RunError> {
        let available_slots = PublicOnceRuleCount::new(self.states.len());
        let Some(state) = self.states.get_mut(permit.slot.get()) else {
            return Err(RunInvariantError::MissingOnceRuleState {
                rule: permit.rule,
                available_slots,
            }
            .into());
        };

        *state = OnceRuleState::Consumed;
        Ok(())
    }
}
