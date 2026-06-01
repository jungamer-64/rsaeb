use alloc::vec::Vec;
use core::slice;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::inspect::OnceRuleCount;
use crate::program::{ActiveRuleCursor, RuleCursorAfterMiss, RuleScan};
use crate::rule::{Rule, RuleAvailability};

/// Per-run execution state for parsed `(once)` slots.
#[derive(Debug)]
pub(crate) struct OnceStateSet {
    /// One runtime state cell for each parser-assigned `(once)` slot.
    states: Vec<OnceRuleRuntimeState>,
}

/// Runtime state for one parsed `(once)` rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OnceRuleRuntimeState {
    /// Rule has not committed during this run.
    Fresh,
    /// Rule has already committed during this run.
    Committed,
}

/// Linear commit action for a matched rule.
#[derive(Debug)]
pub(super) enum MatchedRuleCommit<'state> {
    /// Rule has no once-state side effect.
    Always,
    /// Rule owns the unique permit to consume its once state.
    Once(OnceMatchPermit<'state>),
}

/// Private permit that consumes one fresh once-rule state on commit.
#[derive(Debug)]
pub(super) struct OnceMatchPermit<'state> {
    /// Fresh per-rule state reserved for the matched rule.
    state: &'state mut OnceRuleRuntimeState,
    /// Non-copy token that keeps the permit linear even though its witnesses are copyable.
    linearity: OnceMatchPermitLinearity,
}

/// Non-copy marker carried by once-rule commit permits.
#[derive(Debug)]
struct OnceMatchPermitLinearity;

/// Parsed rule paired with its runtime availability state.
#[derive(Debug)]
pub(crate) struct RuntimeRule<'program, 'once> {
    /// Parsed executable rule.
    rule: &'program Rule,
    /// Runtime availability selected by this rule's parsed shape.
    availability: RuntimeRuleAvailability<'once>,
}

/// Iterator pairing parsed rules with only the `(once)` states they require.
pub(crate) struct RuntimeRulesMut<'program, 'once> {
    /// Parsed executable rules in execution order.
    rules: slice::Iter<'program, Rule>,
    /// Runtime state cells for parser-assigned `(once)` slots.
    once_states: slice::IterMut<'once, OnceRuleRuntimeState>,
}

/// Runtime availability paired with one parsed rule.
#[derive(Debug)]
enum RuntimeRuleAvailability<'once> {
    /// Rule has no per-run state.
    Always,
    /// Rule owns the state cell at its parser-assigned `(once)` slot.
    Once(&'once mut OnceRuleRuntimeState),
}

/// Availability of a parsed rule together with the only valid commit path.
#[derive(Debug)]
pub(super) enum RuntimeRuleReadiness<'once> {
    /// Rule is available and carries the seed for a later successful application.
    Available(RuntimeRuleCommitSeed<'once>),
    /// Rule has already committed during this runtime invocation.
    Consumed,
}

/// Data that can mint the linear commit action after a rule match is known.
#[derive(Debug)]
pub(super) enum RuntimeRuleCommitSeed<'once> {
    /// Rule has no once-state side effect.
    Always,
    /// Rule owns this fresh per-rule runtime state.
    Once {
        /// Fresh per-rule runtime state for the matched rule.
        state: &'once mut OnceRuleRuntimeState,
    },
}

/// Checked rule-attempt selection produced by a cursor and parser-assigned once slots.
pub(crate) struct RuntimeRuleAttemptTarget<'program, 'once> {
    /// Cursor movement allowed if this target misses.
    after_miss: RuleCursorAfterMiss<'program>,
    /// Parsed rule selected with its runtime state.
    target: RuntimeRule<'program, 'once>,
}

impl OnceMatchPermitLinearity {
    /// Creates the linearity marker for one permit.
    const fn new() -> Self {
        Self
    }
}

impl OnceStateSet {
    /// Builds per-execution once-state from the parser-assigned once-rule count.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the per-execution once-state table cannot be
    /// allocated.
    pub(crate) fn new(once_count: OnceRuleCount) -> Result<Self, AllocationError> {
        let mut states = Vec::new();
        try_reserve_total_exact(
            &mut states,
            RequestedCapacity::new(once_count.get()),
            AllocationContext::OnceRuleState,
        )?;
        for _ in 0..once_count.get() {
            try_push(
                &mut states,
                OnceRuleRuntimeState::Fresh,
                AllocationContext::OnceRuleState,
            )?;
        }

        Ok(Self { states })
    }

    /// Starts scanning parsed rules together with the once-state slots they require.
    pub(crate) fn scan<'program, 'once>(
        &'once mut self,
        rules: RuleScan<'program>,
    ) -> RuntimeRulesMut<'program, 'once> {
        RuntimeRulesMut {
            rules: rules.iter(),
            once_states: self.states.iter_mut(),
        }
    }

    /// Selects the next rule-attempt target from a cursor and parser-assigned once slots.
    pub(crate) fn attempt_target<'program, 'once>(
        &'once mut self,
        rules: RuleScan<'program>,
        cursor: ActiveRuleCursor<'program>,
    ) -> RuntimeRuleAttemptTarget<'program, 'once> {
        let (rule, after_miss) = rules.consume_cursor(cursor);
        let target = RuntimeRule::new(rule, self.runtime_state_for(rule));

        RuntimeRuleAttemptTarget { after_miss, target }
    }

    /// Runtime availability state for one parsed rule.
    fn runtime_state_for(&mut self, rule: &Rule) -> RuntimeRuleAvailability<'_> {
        match rule.availability() {
            RuleAvailability::Always => RuntimeRuleAvailability::Always,
            RuleAvailability::Once(slot) => self
                .states
                .get_mut(slot.index())
                .map(RuntimeRuleAvailability::Once)
                .unwrap_or(RuntimeRuleAvailability::Always),
        }
    }
}

impl<'program, 'once> Iterator for RuntimeRulesMut<'program, 'once> {
    type Item = RuntimeRule<'program, 'once>;

    fn next(&mut self) -> Option<Self::Item> {
        let rule = self.rules.next()?;
        let availability = match rule.availability() {
            RuleAvailability::Always => RuntimeRuleAvailability::Always,
            RuleAvailability::Once(_) => RuntimeRuleAvailability::Once(self.once_states.next()?),
        };
        Some(RuntimeRule::new(rule, availability))
    }
}

impl<'program, 'once> RuntimeRule<'program, 'once> {
    /// Pairs a parsed rule with its runtime availability state.
    fn new(rule: &'program Rule, availability: RuntimeRuleAvailability<'once>) -> Self {
        Self { rule, availability }
    }

    /// Parsed rule selected with its runtime state.
    pub(super) const fn rule(&self) -> &'program Rule {
        self.rule
    }

    /// Returns this rule's current per-run readiness and commit action.
    pub(super) fn readiness(self) -> RuntimeRuleReadiness<'once> {
        match self.availability {
            RuntimeRuleAvailability::Always => {
                RuntimeRuleReadiness::Available(RuntimeRuleCommitSeed::Always)
            }
            RuntimeRuleAvailability::Once(state) => match *state {
                OnceRuleRuntimeState::Fresh => {
                    RuntimeRuleReadiness::Available(RuntimeRuleCommitSeed::Once { state })
                }
                OnceRuleRuntimeState::Committed => RuntimeRuleReadiness::Consumed,
            },
        }
    }
}

impl<'state> OnceMatchPermit<'state> {
    /// Creates the commit permit after availability has been checked.
    pub(super) fn new(state: &'state mut OnceRuleRuntimeState) -> Self {
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

impl<'once> RuntimeRuleCommitSeed<'once> {
    /// Mints the linear commit action for a rule that has already matched.
    pub(super) fn into_matched_commit(self) -> MatchedRuleCommit<'once> {
        match self {
            Self::Always => MatchedRuleCommit::Always,
            Self::Once { state } => MatchedRuleCommit::Once(OnceMatchPermit::new(state)),
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
        *state = OnceRuleRuntimeState::Committed;
    }
}

impl<'program, 'once> RuntimeRuleAttemptTarget<'program, 'once> {
    /// Splits the checked target into cursor progress and selected rule state.
    pub(crate) fn into_parts(
        self,
    ) -> (RuleCursorAfterMiss<'program>, RuntimeRule<'program, 'once>) {
        (self.after_miss, self.target)
    }
}
