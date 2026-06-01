use alloc::vec::Vec;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::error::RuleRuntimeStateError;
use crate::inspect::OnceRuleCount;
use crate::rule::{OnceRuleSlot, Rule, RuleAvailability};

/// Per-run execution state for parsed `(once)` slots.
#[derive(Debug)]
pub(crate) struct OnceStateSet {
    /// Runtime state for each parser-assigned `(once)` slot.
    slot_states: Vec<OnceSlotState>,
}

/// Runtime state for one parser-assigned `(once)` slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OnceSlotState {
    /// Slot has not committed during this run.
    Fresh,
    /// Slot has already committed during this run.
    Committed,
}

/// Linear commit action for a matched rule.
#[derive(Debug)]
pub(super) enum MatchedRuleCommit<'once> {
    /// Rule has no once-state side effect.
    Always,
    /// Rule owns the unique permit to consume its once state.
    Once(OnceMatchPermit<'once>),
}

/// Availability of a parsed rule together with the only valid commit path.
#[derive(Debug)]
pub(super) enum OnceRuleReadiness<'once> {
    /// Rule is available and carries the seed for a later successful application.
    Available(OnceRuleCommitSeed<'once>),
    /// Rule has already committed during this runtime invocation.
    Consumed,
}

/// Data that can mint the linear commit action after a rule match is known.
#[derive(Debug)]
pub(super) enum OnceRuleCommitSeed<'once> {
    /// Rule has no once-state side effect.
    Always,
    /// Rule owns this fresh parser-assigned `(once)` slot.
    Once {
        /// Fresh parser-assigned slot state for the matched rule.
        slot_state: &'once mut OnceSlotState,
    },
}

/// Availability of a scanned parsed rule without reserving mutable once state yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScannedRuleReadiness {
    /// Rule is available and carries the data needed to mint its commit action.
    Available(ScannedRuleCommit),
    /// Rule has already committed during this runtime invocation.
    Consumed,
}

/// Commit seed for a scanned rule already proven available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScannedRuleCommit {
    /// Rule has no once-state side effect.
    Always,
    /// Rule owns this fresh parser-assigned `(once)` slot.
    Once(OnceRuleSlot),
}

/// Private permit that consumes one fresh once-rule state on commit.
#[derive(Debug)]
pub(super) struct OnceMatchPermit<'once> {
    /// Fresh parser-assigned slot reserved for the matched rule.
    slot_state: &'once mut OnceSlotState,
    /// Non-copy token that keeps the permit linear even though its witnesses are copyable.
    linearity: OnceMatchPermitLinearity,
}

/// Parsed rule paired with its runtime availability state.
#[derive(Debug)]
pub(crate) struct RuntimeRule<'program, 'once> {
    /// Parsed executable rule.
    rule: &'program Rule,
    /// Runtime availability selected by this rule's parsed shape.
    availability: RuntimeRuleAvailability<'once>,
}

/// Runtime availability paired with one parsed rule.
#[derive(Debug)]
enum RuntimeRuleAvailability<'once> {
    /// Rule has no per-run state.
    Always,
    /// Rule owns this parser-assigned `(once)` slot.
    Once {
        /// Parser-assigned once slot state for this rule.
        slot_state: &'once mut OnceSlotState,
    },
}

/// Non-copy marker carried by once-rule commit permits.
#[derive(Debug)]
struct OnceMatchPermitLinearity;

impl OnceMatchPermitLinearity {
    /// Creates the linearity marker for one permit.
    const fn new() -> Self {
        Self
    }
}

impl<'once> OnceMatchPermit<'once> {
    /// Creates the commit permit after availability has been checked.
    fn new(slot_state: &'once mut OnceSlotState) -> Self {
        Self {
            slot_state,
            linearity: OnceMatchPermitLinearity::new(),
        }
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
    /// Builds per-execution rule availability state from the parsed rule table.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the per-execution rule-state table cannot
    /// be allocated.
    pub(crate) fn new(once_count: OnceRuleCount) -> Result<Self, AllocationError> {
        let mut slot_states = Vec::new();
        try_reserve_total_exact(
            &mut slot_states,
            RequestedCapacity::from_once_rule_count(once_count),
            AllocationContext::RuntimeOnceRuleState,
        )?;
        for _ in 0..once_count.get() {
            try_push(
                &mut slot_states,
                OnceSlotState::Fresh,
                AllocationContext::RuntimeOnceRuleState,
            )?;
        }

        Ok(Self { slot_states })
    }

    /// Pairs one parsed rule with its parser-assigned runtime availability.
    ///
    /// # Errors
    ///
    /// Returns `RuleRuntimeStateError` if this run-local once-state table was
    /// not constructed from the same parsed program as `rule`.
    pub(super) fn runtime_rule_mut<'program, 'once>(
        &'once mut self,
        rule: &'program Rule,
    ) -> Result<RuntimeRule<'program, 'once>, RuleRuntimeStateError> {
        let availability = match rule.availability() {
            RuleAvailability::Always => RuntimeRuleAvailability::Always,
            RuleAvailability::Once(slot) => RuntimeRuleAvailability::Once {
                slot_state: self.slot_state_mut(slot)?,
            },
        };

        Ok(RuntimeRule { rule, availability })
    }

    /// Returns the scanned rule's readiness without reserving mutable once state.
    ///
    /// # Errors
    ///
    /// Returns `RuleRuntimeStateError` if this run-local once-state table was
    /// not constructed from the same parsed program as `rule`.
    pub(super) fn scanned_rule_readiness(
        &self,
        rule: &Rule,
    ) -> Result<ScannedRuleReadiness, RuleRuntimeStateError> {
        match rule.availability() {
            RuleAvailability::Always => {
                Ok(ScannedRuleReadiness::Available(ScannedRuleCommit::Always))
            }
            RuleAvailability::Once(slot) => match self.slot_state(slot) {
                Ok(OnceSlotState::Fresh) => Ok(ScannedRuleReadiness::Available(
                    ScannedRuleCommit::Once(slot),
                )),
                Ok(OnceSlotState::Committed) => Ok(ScannedRuleReadiness::Consumed),
                Err(error) => Err(error),
            },
        }
    }

    /// Returns the runtime state for one parser-assigned `(once)` slot.
    ///
    /// # Errors
    ///
    /// Returns `RuleRuntimeStateError` if a parsed rule from a different program is paired
    /// with this run-local once-state table.
    fn slot_state(&self, slot: OnceRuleSlot) -> Result<OnceSlotState, RuleRuntimeStateError> {
        self.slot_states
            .get(slot.index())
            .copied()
            .ok_or_else(|| self.slot_error(slot))
    }

    /// Returns mutable runtime state for one parser-assigned `(once)` slot.
    ///
    /// # Errors
    ///
    /// Returns `RuleRuntimeStateError` if a parsed rule from a different program is paired
    /// with this run-local once-state table.
    fn slot_state_mut(
        &mut self,
        slot: OnceRuleSlot,
    ) -> Result<&mut OnceSlotState, RuleRuntimeStateError> {
        let slot_count = OnceRuleCount::new(self.slot_states.len());
        self.slot_states
            .get_mut(slot.index())
            .ok_or_else(|| RuleRuntimeStateError::once_slot_out_of_range(slot.index(), slot_count))
    }

    /// Builds an out-of-range slot error for this run-local table.
    fn slot_error(&self, slot: OnceRuleSlot) -> RuleRuntimeStateError {
        RuleRuntimeStateError::once_slot_out_of_range(
            slot.index(),
            OnceRuleCount::new(self.slot_states.len()),
        )
    }
}

impl<'program, 'once> RuntimeRule<'program, 'once> {
    /// Parsed rule selected with its runtime state.
    pub(super) const fn rule(&self) -> &'program Rule {
        self.rule
    }

    /// Returns this rule's current per-run readiness and commit action.
    pub(super) fn readiness(self) -> OnceRuleReadiness<'once> {
        match self.availability {
            RuntimeRuleAvailability::Always => {
                OnceRuleReadiness::Available(OnceRuleCommitSeed::Always)
            }
            RuntimeRuleAvailability::Once { slot_state } => match *slot_state {
                OnceSlotState::Fresh => {
                    OnceRuleReadiness::Available(OnceRuleCommitSeed::Once { slot_state })
                }
                OnceSlotState::Committed => OnceRuleReadiness::Consumed,
            },
        }
    }
}

impl<'once> OnceRuleCommitSeed<'once> {
    /// Mints the linear commit action for a rule that has already matched.
    pub(super) fn into_matched_commit(self) -> MatchedRuleCommit<'once> {
        match self {
            Self::Always => MatchedRuleCommit::Always,
            Self::Once { slot_state } => MatchedRuleCommit::Once(OnceMatchPermit::new(slot_state)),
        }
    }
}

impl ScannedRuleCommit {
    /// Mints the linear commit action for this already selected rule.
    ///
    /// # Errors
    ///
    /// Returns `RuleRuntimeStateError` if this scanned commit was paired with a
    /// different program's run-local once-state table.
    pub(super) fn into_matched_commit(
        self,
        table: &mut OnceStateSet,
    ) -> Result<MatchedRuleCommit<'_>, RuleRuntimeStateError> {
        match self {
            Self::Always => Ok(MatchedRuleCommit::Always),
            Self::Once(slot) => {
                let slot_state = table.slot_state_mut(slot)?;
                Ok(MatchedRuleCommit::Once(OnceMatchPermit::new(slot_state)))
            }
        }
    }
}

impl OnceMatchPermit<'_> {
    /// Consumes this permit and marks the owning once-rule state as consumed.
    fn commit(self) {
        let Self {
            slot_state,
            linearity: _linearity,
        } = self;
        *slot_state = OnceSlotState::Committed;
    }
}
