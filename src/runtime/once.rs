use alloc::{collections::VecDeque, vec::Vec};

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::policy::ParsePolicy;
use crate::program::{ExecutableProgram, RuleScan};
use crate::rule::{Rule, RuleAvailability};
use crate::runtime::matcher::{MatchedRuleApplication, RuleAttempt, attempt_rule};
use crate::runtime::state::State;

/// Per-run ordinary execution table with parsed rules and runtime availability paired.
#[derive(Debug)]
pub(crate) struct RuntimeRuleTable<'program> {
    /// Runtime rule cells in parser execution order.
    cells: Vec<RuntimeRuleCell<'program>>,
}

/// Outcome of scanning the ordinary runtime rule table.
#[derive(Debug)]
pub(crate) enum RuntimeRuleSearch<'program, 'state, 'once> {
    /// A rule matched and carries the commit permit needed after success.
    Matched(MatchedRuleApplication<'program, 'state, 'once>),
    /// No currently available rule matched the runtime state.
    Stable,
}

/// Per-run rule-attempt pass over executable rules and their availability cells.
#[derive(Debug)]
pub(crate) enum RuntimeRulePass<'program> {
    /// The current rule is not the final target in this pass.
    Continuing(ContinuingRuntimeRulePass<'program>),
    /// The current rule is the final target in this pass.
    Final(FinalRuntimeRulePass<'program>),
}

/// Rule-attempt pass state whose current target has at least one successor.
#[derive(Debug)]
pub(crate) struct ContinuingRuntimeRulePass<'program> {
    /// Current executable rule attempt target.
    current: RuntimeRuleCell<'program>,
    /// Non-empty tail after the current target.
    pending: PendingRuntimeRules<'program>,
    /// Rules already missed in this pass, in original rule order.
    attempted: VecDeque<RuntimeRuleCell<'program>>,
}

/// Rule-attempt pass state whose current target exhausts the pass.
#[derive(Debug)]
pub(crate) struct FinalRuntimeRulePass<'program> {
    /// Current executable rule attempt target.
    current: RuntimeRuleCell<'program>,
    /// Rules already missed in this pass, in original rule order.
    attempted: VecDeque<RuntimeRuleCell<'program>>,
    /// Empty pre-reserved buffer used when a later rewrite resets the pass.
    spare: VecDeque<RuntimeRuleCell<'program>>,
}

/// Non-empty tail of unattempted rules after a continuing current target.
#[derive(Debug)]
struct PendingRuntimeRules<'program> {
    /// Next rule after the current target.
    next: RuntimeRuleCell<'program>,
    /// Remaining rules after `next`, in original rule order.
    remaining: VecDeque<RuntimeRuleCell<'program>>,
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

impl OnceMatchPermitLinearity {
    /// Creates the linearity marker for one permit.
    const fn new() -> Self {
        Self
    }
}

impl<'program> RuntimeRuleTable<'program> {
    /// Builds a run-local ordinary execution table from an executable program.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the per-execution rule table cannot be
    /// allocated.
    pub(crate) fn from_program<P: ParsePolicy>(
        program: &'program ExecutableProgram<P>,
    ) -> Result<Self, AllocationError> {
        Self::from_rule_scan(program.rule_scan())
    }

    /// Builds a run-local ordinary execution table from the executable rule table.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the per-execution rule table cannot be
    /// allocated.
    fn from_rule_scan(rules: RuleScan<'program>) -> Result<Self, AllocationError> {
        let rule_count = rules.iter().count();
        let mut cells = Vec::new();
        try_reserve_total_exact(
            &mut cells,
            RequestedCapacity::new(rule_count),
            AllocationContext::RuntimeRuleAvailability,
        )?;
        for rule in rules.iter() {
            try_push(
                &mut cells,
                RuntimeRuleCell::from_rule(rule),
                AllocationContext::RuntimeRuleAvailability,
            )?;
        }

        Ok(Self { cells })
    }

    /// Finds the first currently available rule that matches `state`.
    pub(crate) fn find_next_match<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuntimeRuleSearch<'program, 'state, 'once> {
        for cell in &mut self.cells {
            match attempt_rule(cell.as_runtime_rule(), state) {
                RuleAttempt::Matched(matched) => return RuntimeRuleSearch::Matched(matched),
                RuleAttempt::Missed(_missed) => {}
            }
        }

        RuntimeRuleSearch::Stable
    }
}

impl<'program> RuntimeRulePass<'program> {
    /// Builds a rule-attempt pass from an executable program.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the per-execution rule-attempt table cannot
    /// be allocated.
    pub(crate) fn from_program<P: ParsePolicy>(
        program: &'program ExecutableProgram<P>,
    ) -> Result<Self, AllocationError> {
        Self::from_rule_scan(program.rule_scan())
    }

    /// Builds a rule-attempt pass from the executable rule table.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the per-execution rule-attempt table cannot
    /// be allocated.
    fn from_rule_scan(rules: RuleScan<'program>) -> Result<Self, AllocationError> {
        let (first, remaining_rules) = rules.split_first();
        let mut remaining = VecDeque::new();
        let remaining_capacity = RequestedCapacity::new(remaining_rules.len());
        try_reserve_rule_queue(
            &mut remaining,
            remaining_capacity,
            AllocationContext::RuntimeRuleAvailability,
        )?;
        for rule in remaining_rules {
            remaining.push_back(RuntimeRuleCell::from_rule(rule));
        }
        let mut attempted = VecDeque::new();
        try_reserve_rule_queue(
            &mut attempted,
            remaining_capacity,
            AllocationContext::RuntimeRuleAvailability,
        )?;
        Ok(RuntimeRulePassParts {
            current: RuntimeRuleCell::from_rule(first),
            remaining,
            attempted,
        }
        .into_pass())
    }

    /// Resets this pass to the first executable rule after a rewrite.
    pub(crate) fn reset_after_rewrite(self) -> Self {
        match self {
            Self::Continuing(pass) => pass.reset_after_rewrite(),
            Self::Final(pass) => pass.reset_after_rewrite(),
        }
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

impl<'program> PendingRuntimeRules<'program> {
    /// Builds a non-empty pending tail from its head and remaining rules.
    fn new(
        next: RuntimeRuleCell<'program>,
        remaining: VecDeque<RuntimeRuleCell<'program>>,
    ) -> Self {
        Self { next, remaining }
    }

    /// Moves the pending tail to the current target after a non-final miss.
    fn advance(
        mut self,
    ) -> (
        RuntimeRuleCell<'program>,
        AdvancedPendingRuntimeRules<'program>,
    ) {
        let current = self.next;
        let advanced = match self.remaining.pop_front() {
            Some(next) => AdvancedPendingRuntimeRules::Continuing(Self::new(next, self.remaining)),
            None => AdvancedPendingRuntimeRules::Final {
                spare: self.remaining,
            },
        };
        (current, advanced)
    }

    /// Appends pending rules to `output` in executable order.
    fn append_to(
        self,
        output: &mut VecDeque<RuntimeRuleCell<'program>>,
    ) -> VecDeque<RuntimeRuleCell<'program>> {
        output.push_back(self.next);
        let mut remaining = self.remaining;
        while let Some(rule) = remaining.pop_front() {
            output.push_back(rule);
        }
        remaining
    }
}

/// Result of advancing a non-empty pending tail.
#[derive(Debug)]
enum AdvancedPendingRuntimeRules<'program> {
    /// Another target remains after the new current target.
    Continuing(PendingRuntimeRules<'program>),
    /// The new current target is final.
    Final {
        /// Empty pre-reserved buffer retained for a later rewrite reset.
        spare: VecDeque<RuntimeRuleCell<'program>>,
    },
}

/// Rule-attempt pass under construction from a current target and ordered tail.
struct RuntimeRulePassParts<'program> {
    /// Current executable rule attempt target.
    current: RuntimeRuleCell<'program>,
    /// Remaining executable rules after the current target, in attempt order.
    remaining: VecDeque<RuntimeRuleCell<'program>>,
    /// Empty pre-reserved buffer for missed rules in the current pass.
    attempted: VecDeque<RuntimeRuleCell<'program>>,
}

impl<'program> RuntimeRulePassParts<'program> {
    /// Classifies the current target by whether the ordered tail is empty.
    fn into_pass(mut self) -> RuntimeRulePass<'program> {
        match self.remaining.pop_front() {
            Some(next) => RuntimeRulePass::Continuing(ContinuingRuntimeRulePass {
                current: self.current,
                pending: PendingRuntimeRules::new(next, self.remaining),
                attempted: self.attempted,
            }),
            None => RuntimeRulePass::Final(FinalRuntimeRulePass {
                current: self.current,
                attempted: self.attempted,
                spare: self.remaining,
            }),
        }
    }
}

impl<'program> ContinuingRuntimeRulePass<'program> {
    /// Borrows the current target as a runtime rule with availability state.
    pub(crate) fn current_rule(&mut self) -> RuntimeRule<'program, '_> {
        self.current.as_runtime_rule()
    }

    /// Commits a non-applying attempt and returns the next typed pass state.
    pub(crate) fn commit_miss(mut self) -> RuntimeRulePass<'program> {
        self.attempted.push_back(self.current);
        let (current, advanced) = self.pending.advance();
        match advanced {
            AdvancedPendingRuntimeRules::Continuing(pending) => RuntimeRulePass::Continuing(Self {
                current,
                pending,
                attempted: self.attempted,
            }),
            AdvancedPendingRuntimeRules::Final { spare } => {
                RuntimeRulePass::Final(FinalRuntimeRulePass {
                    current,
                    attempted: self.attempted,
                    spare,
                })
            }
        }
    }

    /// Resets this pass to its first executable rule after a rewrite.
    fn reset_after_rewrite(self) -> RuntimeRulePass<'program> {
        let Self {
            current,
            pending,
            mut attempted,
        } = self;
        let Some(first) = attempted.pop_front() else {
            return RuntimeRulePass::Continuing(Self {
                current,
                pending,
                attempted,
            });
        };

        let mut remaining = attempted;
        remaining.push_back(current);
        let attempted = pending.append_to(&mut remaining);
        RuntimeRulePassParts {
            current: first,
            remaining,
            attempted,
        }
        .into_pass()
    }
}

impl<'program> FinalRuntimeRulePass<'program> {
    /// Borrows the current target as a runtime rule with availability state.
    pub(crate) fn current_rule(&mut self) -> RuntimeRule<'program, '_> {
        self.current.as_runtime_rule()
    }

    /// Resets this final pass to its first executable rule after a rewrite.
    fn reset_after_rewrite(self) -> RuntimeRulePass<'program> {
        let Self {
            current,
            mut attempted,
            spare,
        } = self;
        let Some(first) = attempted.pop_front() else {
            return RuntimeRulePass::Final(Self {
                current,
                attempted,
                spare,
            });
        };

        let mut remaining = attempted;
        remaining.push_back(current);
        RuntimeRulePassParts {
            current: first,
            remaining,
            attempted: spare,
        }
        .into_pass()
    }
}

/// Reserves a rule queue through the runtime-rule allocation boundary.
///
/// # Errors
///
/// Returns `AllocationError` if the requested capacity cannot be represented
/// or if the allocator rejects the reservation.
fn try_reserve_rule_queue<T>(
    queue: &mut VecDeque<T>,
    total_capacity: RequestedCapacity,
    context: AllocationContext,
) -> Result<(), AllocationError> {
    if queue.capacity() >= total_capacity.get() {
        return Ok(());
    }

    let additional = total_capacity
        .get()
        .checked_sub(queue.len())
        .ok_or_else(|| AllocationError::capacity_overflow(context))?;

    queue
        .try_reserve_exact(additional)
        .map_err(|_| AllocationError::reservation_failed(context, total_capacity))
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
