use alloc::vec::Vec;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::inspect::RuleCount;
use crate::rule::{Rule, RuleRepeatState};

/// Per-run execution state aligned with the parsed rule table.
#[derive(Debug)]
pub(crate) struct OnceStateSet {
    /// One runtime row for each parsed rule.
    states: Vec<RuleExecutionState>,
}

/// Runtime state for one parsed rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuleExecutionState {
    /// Rule has no once-state side effect.
    Always,
    /// Rule has a once-state cell for this run.
    Once(OnceRuleState),
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
    /// Builds per-execution rule state directly from the parsed program rules.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the per-execution rule-state table cannot
    /// be allocated.
    pub(crate) fn new(rules: &[Rule]) -> Result<Self, AllocationError> {
        let mut states = Vec::new();
        try_reserve_total_exact(
            &mut states,
            RequestedCapacity::from_rule_count(RuleCount::new(rules.len())),
            AllocationContext::RuntimeOnceRuleState,
        )?;

        for rule in rules {
            try_push(
                &mut states,
                RuleExecutionState::from_rule(rule),
                AllocationContext::RuntimeOnceRuleState,
            )?;
        }

        Ok(Self { states })
    }

    /// Iterates over rule-aligned runtime state rows.
    pub(super) fn rows_mut(&mut self) -> impl Iterator<Item = &mut RuleExecutionState> {
        self.states.iter_mut()
    }

    /// Number of rule-aligned runtime state rows.
    pub(super) fn row_count(&self) -> RuleCount {
        RuleCount::new(self.states.len())
    }
}

impl RuleExecutionState {
    /// Builds the runtime row corresponding to one parsed rule.
    fn from_rule(rule: &Rule) -> Self {
        match rule.repeat_state() {
            RuleRepeatState::Always => Self::Always,
            RuleRepeatState::Once => Self::Once(OnceRuleState::Fresh),
        }
    }

    /// Reserves the commit side effect for a rule that may currently run.
    pub(super) fn reserve_commit(&mut self) -> Option<MatchedRuleCommit<'_>> {
        match self {
            Self::Always => Some(MatchedRuleCommit::Always),
            Self::Once(state @ OnceRuleState::Fresh) => {
                Some(MatchedRuleCommit::Once(OnceMatchPermit::new(state)))
            }
            Self::Once(OnceRuleState::Consumed) => None,
        }
    }
}
