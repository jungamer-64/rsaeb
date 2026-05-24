use alloc::vec::Vec;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::rule::Rule;

/// Per-run execution state aligned with the parsed rule table.
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
}
