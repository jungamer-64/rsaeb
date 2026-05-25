use alloc::vec::Vec;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::inspect::OnceRuleCount;
use crate::rule::{Rule, RuleAvailability};

/// Per-run execution state aligned one-to-one with parsed executable rules.
#[derive(Debug)]
pub(crate) struct OnceStateSet {
    /// One runtime state cell for each parsed `(once)` slot.
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

/// Availability of a parsed rule together with the only valid commit path.
#[derive(Debug)]
pub(super) enum OnceRuleReadiness<'once> {
    /// Rule is available and carries the commit action for a later successful application.
    Available(MatchedRuleCommit<'once>),
    /// Rule has already committed during this runtime invocation.
    Consumed,
}

/// Private permit that consumes one fresh once-rule state on commit.
#[derive(Debug)]
pub(super) struct OnceMatchPermit<'once> {
    /// Fresh once-state reserved for the matched rule.
    state: &'once mut OnceRuleState,
    /// Non-copy token that keeps the permit linear even though its witnesses are copyable.
    linearity: OnceMatchPermitLinearity,
}

/// Parsed rule paired with its aligned runtime availability state.
#[derive(Debug)]
pub(crate) struct RuntimeRule<'program, 'once> {
    /// Parsed executable rule.
    rule: &'program Rule,
    /// Runtime state cell for this rule's parsed once slot.
    state: &'once mut OnceRuleState,
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
    fn new(state: &'once mut OnceRuleState) -> Self {
        Self {
            state,
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
    pub(crate) fn new(rules: &[Rule]) -> Result<Self, AllocationError> {
        let once_count = OnceRuleCount::new(
            rules
                .iter()
                .filter(|rule| rule.availability().is_once())
                .count(),
        );
        let mut states = Vec::new();
        try_reserve_total_exact(
            &mut states,
            RequestedCapacity::from_once_rule_count(once_count),
            AllocationContext::RuntimeOnceRuleState,
        )?;

        for rule in rules {
            if rule.availability().is_once() {
                try_push(
                    &mut states,
                    OnceRuleState::Fresh,
                    AllocationContext::RuntimeOnceRuleState,
                )?;
            }
        }

        Ok(Self { states })
    }
}

impl<'program, 'once> RuntimeRule<'program, 'once> {
    /// Parsed rule selected with its runtime state.
    pub(super) const fn rule(&self) -> &'program Rule {
        self.rule
    }

    /// Returns this rule's current per-run readiness and commit action.
    pub(super) fn readiness(self) -> OnceRuleReadiness<'once> {
        match self.rule.availability() {
            RuleAvailability::Always => OnceRuleReadiness::Available(MatchedRuleCommit::Always),
            RuleAvailability::Once => match self.state {
                OnceRuleState::Fresh => {
                    OnceRuleReadiness::Available(MatchedRuleCommit::Once(OnceMatchPermit::new(
                        self.state,
                    )))
                }
                OnceRuleState::Consumed => OnceRuleReadiness::Consumed,
            },
        }
    }
}

impl OnceMatchPermit<'_> {
    /// Consumes this permit and marks the owning once-rule state as consumed.
    fn commit(self) {
        let Self {
            state,
            linearity: _linearity,
        } = self;
        *state = OnceRuleState::Consumed;
    }
}
