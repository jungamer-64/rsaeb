use alloc::vec::Vec;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::program::{ActiveRuleCursor, RuleCursorAfterMiss, RuleScan};
use crate::rule::{Rule, RuleRepeatBehavior};

/// Per-run execution state aligned with the parsed rule table.
#[derive(Debug)]
pub(crate) struct RuntimeRuleStates {
    /// Runtime state for each executable rule in parser-owned order.
    states: Vec<RuntimeRuleState>,
}

/// Runtime state for one executable rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeRuleState {
    /// Rule has no per-run repeat state.
    Always,
    /// Rule has per-run `(once)` state.
    Once(OnceRuleRuntimeState),
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

/// Availability of a parsed rule together with the only valid commit path.
#[derive(Debug)]
pub(super) enum RuntimeRuleReadiness<'state> {
    /// Rule is available and carries the seed for a later successful application.
    Available(RuntimeRuleCommitSeed<'state>),
    /// Rule has already committed during this runtime invocation.
    Consumed,
}

/// Data that can mint the linear commit action after a rule match is known.
#[derive(Debug)]
pub(super) enum RuntimeRuleCommitSeed<'state> {
    /// Rule has no once-state side effect.
    Always,
    /// Rule owns this fresh per-rule runtime state.
    Once {
        /// Fresh per-rule runtime state for the matched rule.
        state: &'state mut OnceRuleRuntimeState,
    },
}

/// Private permit that consumes one fresh once-rule state on commit.
#[derive(Debug)]
pub(super) struct OnceMatchPermit<'state> {
    /// Fresh per-rule state reserved for the matched rule.
    state: &'state mut OnceRuleRuntimeState,
    /// Non-copy token that keeps the permit linear even though its witnesses are copyable.
    linearity: OnceMatchPermitLinearity,
}

/// Parsed rule paired with its aligned runtime repeat state.
#[derive(Debug)]
pub(crate) struct RuntimeRule<'program, 'state> {
    /// Parsed executable rule.
    rule: &'program Rule,
    /// Runtime repeat state selected by this rule's parsed shape.
    state: RuntimeRuleStateRef<'state>,
}

/// Runtime repeat state borrowed for one parsed rule.
#[derive(Debug)]
enum RuntimeRuleStateRef<'state> {
    /// Rule has no per-run state.
    Always,
    /// Rule owns this aligned `(once)` state.
    Once(&'state mut OnceRuleRuntimeState),
}

/// Runtime scan that advances parsed rules and runtime states together.
pub(crate) struct RuntimeRuleScan<'program, 'state> {
    /// Parsed executable rules in execution order.
    rules: core::slice::Iter<'program, Rule>,
    /// Runtime states in the same execution order.
    states: core::slice::IterMut<'state, RuntimeRuleState>,
}

/// Checked rule-attempt selection produced by a cursor and aligned runtime states.
pub(crate) struct RuntimeRuleAttemptTarget<'program, 'state> {
    /// Cursor movement allowed if this target misses.
    after_miss: RuleCursorAfterMiss,
    /// Parsed rule selected with its aligned runtime state.
    target: RuntimeRule<'program, 'state>,
}

/// Non-copy marker carried by once-rule commit permits.
#[derive(Debug)]
struct OnceMatchPermitLinearity;

impl RuntimeRuleState {
    /// Builds runtime state for one parsed rule.
    const fn from_rule(rule: &Rule) -> Self {
        match rule.repeat_behavior() {
            RuleRepeatBehavior::Always => Self::Always,
            RuleRepeatBehavior::Once => Self::Once(OnceRuleRuntimeState::Fresh),
        }
    }
}

impl OnceMatchPermitLinearity {
    /// Creates the linearity marker for one permit.
    const fn new() -> Self {
        Self
    }
}

impl<'state> OnceMatchPermit<'state> {
    /// Creates the commit permit after availability has been checked.
    fn new(state: &'state mut OnceRuleRuntimeState) -> Self {
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

impl RuntimeRuleStates {
    /// Builds per-execution rule availability state from the parsed rule scan.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the per-execution rule-state table cannot
    /// be allocated.
    pub(crate) fn new(rules: RuleScan<'_>) -> Result<Self, AllocationError> {
        let mut states = Vec::new();
        try_reserve_total_exact(
            &mut states,
            RequestedCapacity::from_rule_count(rules.rule_count()),
            AllocationContext::RuntimeRuleState,
        )?;
        for rule in rules.iter() {
            try_push(
                &mut states,
                RuntimeRuleState::from_rule(rule),
                AllocationContext::RuntimeRuleState,
            )?;
        }

        Ok(Self { states })
    }

    /// Starts scanning parsed rules together with their aligned runtime states.
    pub(crate) fn scan<'program, 'state>(
        &'state mut self,
        rules: RuleScan<'program>,
    ) -> RuntimeRuleScan<'program, 'state> {
        RuntimeRuleScan {
            rules: rules.iter(),
            states: self.states.iter_mut(),
        }
    }

    /// Selects the next rule-attempt target from a cursor and aligned rule state.
    pub(crate) fn select_attempt_target<'program, 'state>(
        &'state mut self,
        rules: RuleScan<'program>,
        cursor: ActiveRuleCursor,
    ) -> RuntimeRuleAttemptTarget<'program, 'state> {
        let rule = rules.rule_at_cursor(cursor);
        #[expect(
            clippy::indexing_slicing,
            reason = "RuntimeRuleStates is allocated from the same RuleScan that minted the active cursor"
        )]
        let state = &mut self.states[cursor.next_rule_index()];

        RuntimeRuleAttemptTarget {
            after_miss: rules.after_miss(cursor),
            target: RuntimeRule::new(rule, state),
        }
    }
}

impl<'program, 'state> Iterator for RuntimeRuleScan<'program, 'state> {
    type Item = RuntimeRule<'program, 'state>;

    fn next(&mut self) -> Option<Self::Item> {
        let rule = self.rules.next()?;
        let state = self.states.next()?;
        Some(RuntimeRule::new(rule, state))
    }
}

impl<'program, 'state> RuntimeRule<'program, 'state> {
    /// Pairs a parsed rule with its aligned runtime state.
    fn new(rule: &'program Rule, state: &'state mut RuntimeRuleState) -> Self {
        let state = match state {
            RuntimeRuleState::Always => RuntimeRuleStateRef::Always,
            RuntimeRuleState::Once(state) => RuntimeRuleStateRef::Once(state),
        };
        Self { rule, state }
    }

    /// Parsed rule selected with its runtime state.
    pub(super) const fn rule(&self) -> &'program Rule {
        self.rule
    }

    /// Returns this rule's current per-run readiness and commit action.
    pub(super) fn readiness(self) -> RuntimeRuleReadiness<'state> {
        match self.state {
            RuntimeRuleStateRef::Always => {
                RuntimeRuleReadiness::Available(RuntimeRuleCommitSeed::Always)
            }
            RuntimeRuleStateRef::Once(state) => match *state {
                OnceRuleRuntimeState::Fresh => {
                    RuntimeRuleReadiness::Available(RuntimeRuleCommitSeed::Once { state })
                }
                OnceRuleRuntimeState::Committed => RuntimeRuleReadiness::Consumed,
            },
        }
    }
}

impl<'state> RuntimeRuleCommitSeed<'state> {
    /// Mints the linear commit action for a rule that has already matched.
    pub(super) fn into_matched_commit(self) -> MatchedRuleCommit<'state> {
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

impl<'program, 'state> RuntimeRuleAttemptTarget<'program, 'state> {
    /// Splits the checked target into cursor progress and selected rule state.
    pub(crate) fn into_parts(self) -> (RuleCursorAfterMiss, RuntimeRule<'program, 'state>) {
        (self.after_miss, self.target)
    }
}
