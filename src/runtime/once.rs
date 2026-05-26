use alloc::vec::Vec;
use core::slice;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::error::RuleRuntimeStateError;
use crate::inspect::OnceRuleCount;
use crate::rule::{Rule, RuleAvailability};

/// Per-run execution state for parsed `(once)` slots.
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

/// Parsed rule paired with its runtime availability state.
#[derive(Debug)]
pub(crate) struct RuntimeRule<'program, 'once> {
    /// Parsed executable rule.
    rule: &'program Rule,
    /// Runtime availability selected by this rule's parsed shape.
    availability: RuntimeRuleAvailability<'once>,
}

/// Iterator pairing parsed rules with only the `(once)` states they require.
pub(super) struct RuntimeRulesMut<'program, 'once> {
    /// Parsed executable rules in execution order.
    rules: slice::Iter<'program, Rule>,
    /// Runtime state cells for parser-assigned `(once)` slots.
    once_states: slice::IterMut<'once, OnceRuleState>,
}

/// Runtime availability paired with one parsed rule.
#[derive(Debug)]
enum RuntimeRuleAvailability<'once> {
    /// Rule has no per-run state.
    Always,
    /// Rule owns the state cell at its parser-assigned `(once)` slot.
    Once(&'once mut OnceRuleState),
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
    pub(crate) fn new(once_count: OnceRuleCount) -> Result<Self, AllocationError> {
        let mut states = Vec::new();
        try_reserve_total_exact(
            &mut states,
            RequestedCapacity::from_once_rule_count(once_count),
            AllocationContext::RuntimeOnceRuleState,
        )?;

        for _ in 0..once_count.get() {
            try_push(
                &mut states,
                OnceRuleState::Fresh,
                AllocationContext::RuntimeOnceRuleState,
            )?;
        }

        Ok(Self { states })
    }

    /// Pairs one parsed rule with its parser-assigned runtime availability.
    pub(super) fn runtime_rule_mut<'program, 'once>(
        &'once mut self,
        rule: &'program Rule,
    ) -> Result<RuntimeRule<'program, 'once>, RuleRuntimeStateError> {
        let availability = match rule.availability() {
            RuleAvailability::Always => RuntimeRuleAvailability::Always,
            RuleAvailability::Once(slot) => {
                let state = self
                    .states
                    .get_mut(slot.index())
                    .ok_or_else(|| RuleRuntimeStateError::missing_once_rule_state(rule.position()))?;
                RuntimeRuleAvailability::Once(state)
            }
        };

        Ok(RuntimeRule { rule, availability })
    }

    /// Iterates parsed rules with runtime availability without row-aligned state.
    pub(super) fn runtime_rules_mut<'program, 'once>(
        &'once mut self,
        rules: &'program [Rule],
    ) -> RuntimeRulesMut<'program, 'once> {
        RuntimeRulesMut {
            rules: rules.iter(),
            once_states: self.states.iter_mut(),
        }
    }
}

impl<'program, 'once> Iterator for RuntimeRulesMut<'program, 'once> {
    type Item = Result<RuntimeRule<'program, 'once>, RuleRuntimeStateError>;

    fn next(&mut self) -> Option<Self::Item> {
        let rule = self.rules.next()?;
        let availability = match rule.availability() {
            RuleAvailability::Always => RuntimeRuleAvailability::Always,
            RuleAvailability::Once(_) => match self.once_states.next() {
                Some(state) => RuntimeRuleAvailability::Once(state),
                None => {
                    return Some(Err(RuleRuntimeStateError::missing_once_rule_state(
                        rule.position(),
                    )));
                }
            },
        };

        Some(Ok(RuntimeRule { rule, availability }))
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
                OnceRuleReadiness::Available(MatchedRuleCommit::Always)
            }
            RuntimeRuleAvailability::Once(state) => match state {
                OnceRuleState::Fresh => OnceRuleReadiness::Available(MatchedRuleCommit::Once(
                    OnceMatchPermit::new(state),
                )),
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
