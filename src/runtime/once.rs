use alloc::vec::Vec;
use core::slice;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::program::{RuleScan, RuleScanIter};
use crate::rule::{Rule, RuleAvailability};

/// Per-run execution state for executable rule availability.
#[derive(Debug)]
pub(crate) struct RuntimeRuleStates {
    /// One runtime availability cell for each executable rule.
    states: Vec<RuntimeRuleAvailabilityState>,
}

/// Per-run rule-attempt pass over executable rules and their availability cells.
#[derive(Debug)]
pub(crate) struct RuntimeRulePass<'program> {
    /// Current executable rule attempt target.
    current: RuntimeRuleCell<'program>,
    /// Remaining executable rules after the current target, in attempt order.
    remaining: Vec<RuntimeRuleCell<'program>>,
    /// Targets left in the current pass before stability.
    remaining_attempts: usize,
    /// Total executable rules in the pass.
    total_rules: usize,
}

/// One executable rule paired with its run-local availability state.
#[derive(Debug)]
struct RuntimeRuleCell<'program> {
    /// Parsed executable rule.
    rule: &'program Rule,
    /// Run-local availability for the parsed rule.
    state: RuntimeRuleAvailabilityState,
}

/// Runtime availability state for one parsed executable rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuntimeRuleAvailabilityState {
    /// Rule has no per-run once-state side effect.
    Always,
    /// Rule has not committed during this run.
    FreshOnce,
    /// Rule has already committed during this run.
    CommittedOnce,
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
    state: &'state mut RuntimeRuleAvailabilityState,
    /// Non-copy token that keeps the permit linear even though its witnesses are copyable.
    linearity: OnceMatchPermitLinearity,
}

/// Non-copy marker carried by once-rule commit permits.
#[derive(Debug)]
struct OnceMatchPermitLinearity;

/// Parsed rule paired with its runtime availability state.
#[derive(Debug)]
pub(crate) struct RuntimeRule<'program, 'state> {
    /// Parsed executable rule.
    rule: &'program Rule,
    /// Runtime availability selected by this rule's parsed shape.
    availability: RuntimeRuleAvailability<'state>,
}

/// Iterator pairing parsed rules with their per-run runtime availability states.
pub(crate) struct RuntimeRulesMut<'program, 'state> {
    /// Parsed executable rules in execution order.
    rules: RuleScanIter<'program>,
    /// Runtime state cells for executable rules.
    states: slice::IterMut<'state, RuntimeRuleAvailabilityState>,
}

/// Cursor movement after a non-applying rule-attempt line has been consumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeRuleMissProgress {
    /// Cursor advanced to the next executable rule.
    Advanced,
    /// The consumed miss was the final executable rule.
    Exhausted,
}

/// Runtime availability paired with one parsed rule.
#[derive(Debug)]
enum RuntimeRuleAvailability<'state> {
    /// Rule has no per-run state.
    Always,
    /// Rule owns this per-run state cell.
    Once(&'state mut RuntimeRuleAvailabilityState),
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
        state: &'state mut RuntimeRuleAvailabilityState,
    },
}

/// Checked rule-attempt selection produced by a cursor and runtime rule states.
pub(crate) struct RuntimeRuleAttemptTarget<'program, 'state> {
    /// Cursor movement allowed if this target misses.
    after_miss: RuntimeRuleMissProgress,
    /// Parsed rule selected with its runtime state.
    target: RuntimeRule<'program, 'state>,
}

impl OnceMatchPermitLinearity {
    /// Creates the linearity marker for one permit.
    const fn new() -> Self {
        Self
    }
}

impl RuntimeRuleStates {
    /// Builds per-execution rule state from the executable rule table.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the per-execution rule-state table cannot be
    /// allocated.
    pub(crate) fn new(rules: RuleScan<'_>) -> Result<Self, AllocationError> {
        let rule_count = rules.iter().count();
        let mut states = Vec::new();
        try_reserve_total_exact(
            &mut states,
            RequestedCapacity::new(rule_count),
            AllocationContext::RuntimeRuleAvailability,
        )?;
        for rule in rules.iter() {
            try_push(
                &mut states,
                RuntimeRuleAvailabilityState::from_rule(rule),
                AllocationContext::RuntimeRuleAvailability,
            )?;
        }

        Ok(Self { states })
    }

    /// Starts scanning parsed rules together with their runtime states.
    pub(crate) fn scan<'program, 'state>(
        &'state mut self,
        rules: RuleScan<'program>,
    ) -> RuntimeRulesMut<'program, 'state> {
        RuntimeRulesMut {
            rules: rules.iter(),
            states: self.states.iter_mut(),
        }
    }
}

impl<'program> RuntimeRulePass<'program> {
    /// Builds a rule-attempt pass from the executable rule table.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the per-execution rule-attempt table cannot
    /// be allocated.
    pub(crate) fn new(rules: RuleScan<'program>) -> Result<Self, AllocationError> {
        let (first, remaining_rules) = rules.split_first();
        let mut remaining = Vec::new();
        try_reserve_total_exact(
            &mut remaining,
            RequestedCapacity::new(remaining_rules.len()),
            AllocationContext::RuntimeRuleAvailability,
        )?;
        for rule in remaining_rules {
            try_push(
                &mut remaining,
                RuntimeRuleCell::from_rule(rule),
                AllocationContext::RuntimeRuleAvailability,
            )?;
        }
        let total_rules = remaining_rules.len().saturating_add(1);
        Ok(Self {
            current: RuntimeRuleCell::from_rule(first),
            remaining,
            remaining_attempts: total_rules,
            total_rules,
        })
    }

    /// Selects the current rule-attempt target.
    pub(crate) fn attempt_target(&mut self) -> RuntimeRuleAttemptTarget<'program, '_> {
        let after_miss = if self.remaining_attempts == 1 {
            RuntimeRuleMissProgress::Exhausted
        } else {
            RuntimeRuleMissProgress::Advanced
        };
        RuntimeRuleAttemptTarget {
            after_miss,
            target: self.current.as_runtime_rule(),
        }
    }

    /// Commits a non-applying attempt and advances to the next target when one exists.
    pub(crate) fn commit_miss(&mut self, after_miss: RuntimeRuleMissProgress) {
        if matches!(after_miss, RuntimeRuleMissProgress::Advanced) {
            self.advance_current_to_back();
            self.remaining_attempts = self.remaining_attempts.saturating_sub(1);
        }
    }

    /// Resets the attempt pass to the first executable rule after a rewrite.
    pub(crate) fn reset_after_rewrite(&mut self) {
        if self.remaining_attempts != self.total_rules {
            for _ in 0..self.remaining_attempts {
                self.advance_current_to_back();
            }
            self.remaining_attempts = self.total_rules;
        }
    }

    /// Moves the current cell to the back of the cyclic pass.
    fn advance_current_to_back(&mut self) {
        let next = self.remaining.remove(0);
        let consumed = core::mem::replace(&mut self.current, next);
        self.remaining.push(consumed);
    }
}

impl<'program> RuntimeRuleCell<'program> {
    /// Builds a runtime rule cell from parsed rule data.
    fn from_rule(rule: &'program Rule) -> Self {
        Self {
            rule,
            state: RuntimeRuleAvailabilityState::from_rule(rule),
        }
    }

    /// Borrows this cell as a rule target with availability state.
    fn as_runtime_rule(&mut self) -> RuntimeRule<'program, '_> {
        RuntimeRule::new(self.rule, RuntimeRuleAvailability::new(&mut self.state))
    }
}

impl RuntimeRuleAvailabilityState {
    /// Builds runtime availability state for one parsed rule.
    const fn from_rule(rule: &Rule) -> Self {
        match rule.availability() {
            RuleAvailability::Always => Self::Always,
            RuleAvailability::Once => Self::FreshOnce,
        }
    }
}

impl<'program, 'state> Iterator for RuntimeRulesMut<'program, 'state> {
    type Item = RuntimeRule<'program, 'state>;

    fn next(&mut self) -> Option<Self::Item> {
        let rule = self.rules.next()?;
        let state = self.states.next()?;
        Some(RuntimeRule::new(rule, RuntimeRuleAvailability::new(state)))
    }
}

impl<'state> RuntimeRuleAvailability<'state> {
    /// Builds runtime availability from a per-rule state cell.
    fn new(state: &'state mut RuntimeRuleAvailabilityState) -> Self {
        match state {
            RuntimeRuleAvailabilityState::Always => Self::Always,
            RuntimeRuleAvailabilityState::FreshOnce
            | RuntimeRuleAvailabilityState::CommittedOnce => Self::Once(state),
        }
    }
}

impl<'program, 'state> RuntimeRule<'program, 'state> {
    /// Pairs a parsed rule with its runtime availability state.
    fn new(rule: &'program Rule, availability: RuntimeRuleAvailability<'state>) -> Self {
        Self { rule, availability }
    }

    /// Parsed rule selected with its runtime state.
    pub(super) const fn rule(&self) -> &'program Rule {
        self.rule
    }

    /// Returns this rule's current per-run readiness and commit action.
    pub(super) fn readiness(self) -> RuntimeRuleReadiness<'state> {
        match self.availability {
            RuntimeRuleAvailability::Always => {
                RuntimeRuleReadiness::Available(RuntimeRuleCommitSeed::Always)
            }
            RuntimeRuleAvailability::Once(state) => match *state {
                RuntimeRuleAvailabilityState::FreshOnce => {
                    RuntimeRuleReadiness::Available(RuntimeRuleCommitSeed::Once { state })
                }
                RuntimeRuleAvailabilityState::CommittedOnce
                | RuntimeRuleAvailabilityState::Always => RuntimeRuleReadiness::Consumed,
            },
        }
    }
}

impl<'state> OnceMatchPermit<'state> {
    /// Creates the commit permit after availability has been checked.
    fn new(state: &'state mut RuntimeRuleAvailabilityState) -> Self {
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
        *state = RuntimeRuleAvailabilityState::CommittedOnce;
    }
}

impl<'program, 'state> RuntimeRuleAttemptTarget<'program, 'state> {
    /// Splits the checked target into cursor progress and selected rule state.
    pub(crate) fn into_parts(self) -> (RuntimeRuleMissProgress, RuntimeRule<'program, 'state>) {
        (self.after_miss, self.target)
    }
}
