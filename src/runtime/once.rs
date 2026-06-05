use alloc::{collections::VecDeque, vec::Vec};

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::inspect::{
    AlwaysReturnRuleView, AlwaysRewriteRuleView, OnceReturnRuleView, OnceRewriteRuleView,
};
use crate::program::{ExecutableProgram, RuleScan, RuntimeStoredRule, StoredRuleRef};
use crate::runtime::matcher::{
    AvailableRuleAttempt, MatchedRuleApplication, RuleAttempt, RuleAttemptMiss,
    attempt_available_rule,
};
use crate::runtime::state::State;

/// Per-run ordinary execution table with parsed rules and runtime availability paired.
#[derive(Debug)]
pub(crate) struct RuntimeRuleTable<'program> {
    /// First runtime rule cell, preserving executable non-emptiness.
    first: RuntimeRuleCell<'program>,
    /// Remaining runtime rule cells in parser execution order.
    remaining: Vec<RuntimeRuleCell<'program>>,
}

/// Outcome of scanning the ordinary runtime rule table.
#[derive(Debug)]
pub(crate) enum RuntimeRuleSearch<'program, 'state, 'once> {
    /// A rule matched and carries the commit permit needed after success.
    Matched(MatchedRuleApplication<'program, 'state, 'once>),
    /// No currently available rule matched the runtime state.
    Stable,
}

/// Rule-attempt pass whose history and tail shape are selected by type.
#[derive(Debug)]
pub(crate) struct RuntimeRulePass<'program, History, Tail> {
    /// Current executable rule attempt target.
    current: RuntimeRuleCell<'program>,
    /// Rules already missed in this pass.
    history: History,
    /// Rule targets after the current target.
    tail: Tail,
}

/// Rule-attempt pass at the start of a scan.
#[derive(Debug)]
pub(crate) enum StartedRuntimeRulePass<'program> {
    /// Started with a current rule that has successors.
    Continuing(FirstContinuingRulePass<'program>),
    /// Started with the final rule in the pass.
    Final(FirstFinalRulePass<'program>),
}

/// Rule-attempt pass after at least one miss has committed.
#[derive(Debug)]
pub(crate) enum AfterMissRuntimeRulePass<'program> {
    /// Current rule has successors and at least one earlier miss exists.
    Continuing(AfterMissContinuingRulePass<'program>),
    /// Current rule exhausts the pass and at least one earlier miss exists.
    Final(AfterMissFinalRulePass<'program>),
}

/// Newly started rule-attempt pass paired with its per-run once-state table.
#[derive(Debug)]
pub(crate) struct StartedRuntimeRuleTable<'program> {
    /// Rule-attempt pass classified by current-tail shape.
    pass: StartedRuntimeRulePass<'program>,
}

/// Continuing pass whose current target is still the first rule in the scan.
pub(crate) type FirstContinuingRulePass<'program> =
    RuntimeRulePass<'program, NoMissedRules<'program>, ContinuingRuleTail<'program>>;

/// Continuing pass after one or more rules have missed.
pub(crate) type AfterMissContinuingRulePass<'program> =
    RuntimeRulePass<'program, MissedRuntimeRules<'program>, ContinuingRuleTail<'program>>;

/// Final pass whose current target is still the first rule in the scan.
pub(crate) type FirstFinalRulePass<'program> =
    RuntimeRulePass<'program, NoMissedRules<'program>, FinalRuleTail<'program>>;

/// Final pass after one or more rules have missed.
pub(crate) type AfterMissFinalRulePass<'program> =
    RuntimeRulePass<'program, MissedRuntimeRules<'program>, FinalRuleTail<'program>>;

/// Sealed boundary for the four valid runtime rule-attempt pass shapes.
pub(crate) trait RuntimeRulePassState<'program>: pass_state::Sealed {}

/// Private sealing traits for runtime pass states.
pub(crate) mod pass_state {
    /// Marker implemented only by valid rule-attempt pass shapes.
    pub(crate) trait Sealed {}
}

/// Empty pass history with a pre-reserved buffer for future misses.
#[derive(Debug)]
pub(crate) struct NoMissedRules<'program> {
    /// Empty buffer reused if the first miss commits.
    attempted: VecDeque<RuntimeRuleCell<'program>>,
}

/// Non-empty pass history after at least one rule has missed.
#[derive(Debug)]
pub(crate) struct MissedRuntimeRules<'program> {
    /// First rule missed in the current pass.
    first: RuntimeRuleCell<'program>,
    /// Later missed rules in original rule order.
    remaining: VecDeque<RuntimeRuleCell<'program>>,
}

/// Non-empty tail of unattempted rules after a continuing current target.
#[derive(Debug)]
pub(crate) struct ContinuingRuleTail<'program> {
    /// Next rule after the current target.
    next: RuntimeRuleCell<'program>,
    /// Remaining rules after `next`, in original rule order.
    remaining: VecDeque<RuntimeRuleCell<'program>>,
}

/// Empty tail for a final current target.
#[derive(Debug)]
pub(crate) struct FinalRuleTail<'program> {
    /// Empty pre-reserved buffer reused if a later rewrite resets the pass.
    spare: VecDeque<RuntimeRuleCell<'program>>,
}

/// Result of advancing a continuing tail.
#[derive(Debug)]
enum AdvancedRuleTail<'program> {
    /// Another target remains after the new current target.
    Continuing(ContinuingRuleTail<'program>),
    /// The new current target is final.
    Final(FinalRuleTail<'program>),
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

/// One executable rule classified by its run-local availability shape.
#[derive(Debug)]
enum RuntimeRuleCell<'program> {
    /// Reusable non-terminal rewrite rule.
    AlwaysRewrite(AlwaysRewriteRuntimeRuleCell<'program>),
    /// Once-only non-terminal rewrite rule.
    OnceRewrite(OnceRewriteRuntimeRuleCell<'program>),
    /// Reusable terminal return rule.
    AlwaysReturn(AlwaysReturnRuntimeRuleCell<'program>),
    /// Once-only terminal return rule.
    OnceReturn(OnceReturnRuntimeRuleCell<'program>),
}

/// Runtime cell for a reusable rewrite rule.
#[derive(Debug)]
struct AlwaysRewriteRuntimeRuleCell<'program> {
    /// Position-bearing parsed executable rule.
    rule: AlwaysRewriteRuleView<'program>,
}

/// Runtime cell for a once-only rewrite rule.
#[derive(Debug)]
struct OnceRewriteRuntimeRuleCell<'program> {
    /// Position-bearing parsed executable rule.
    rule: OnceRewriteRuleView<'program>,
    /// Runtime-local availability for this parsed rule.
    availability: RuntimeOnceAvailability,
}

/// Runtime cell for a reusable return rule.
#[derive(Debug)]
struct AlwaysReturnRuntimeRuleCell<'program> {
    /// Position-bearing parsed executable rule.
    rule: AlwaysReturnRuleView<'program>,
}

/// Runtime cell for a once-only return rule.
#[derive(Debug)]
struct OnceReturnRuntimeRuleCell<'program> {
    /// Position-bearing parsed executable rule.
    rule: OnceReturnRuleView<'program>,
    /// Runtime-local availability for this parsed rule.
    availability: RuntimeOnceAvailability,
}

/// Runtime availability state for one parsed `(once)` executable rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeOnceAvailability {
    /// Rule has not committed during this run.
    Fresh,
    /// Rule has already committed during this run.
    Committed,
}

/// Private permit that consumes one fresh once-rule state on commit.
#[derive(Debug)]
pub(crate) struct OnceMatchPermit<'state> {
    /// Fresh per-rule state reserved for the matched rule.
    state: &'state mut RuntimeOnceAvailability,
    /// Non-copy token that keeps the permit linear even though its witnesses are copyable.
    linearity: OnceMatchPermitLinearity,
}

/// Non-copy marker carried by once-rule commit permits.
#[derive(Debug)]
struct OnceMatchPermitLinearity;

/// Runtime cell classification before state matching.
#[derive(Debug)]
enum RuntimeRuleTarget<'program, 'once> {
    /// The rule can be evaluated against the current runtime state.
    Available(AvailableRuntimeRule<'program, 'once>),
    /// The rule has already committed during this runtime invocation.
    Consumed(ConsumedRuntimeRule<'program>),
}

impl<'program> pass_state::Sealed for FirstContinuingRulePass<'program> {}
impl<'program> RuntimeRulePassState<'program> for FirstContinuingRulePass<'program> {}

impl<'program> pass_state::Sealed for AfterMissContinuingRulePass<'program> {}
impl<'program> RuntimeRulePassState<'program> for AfterMissContinuingRulePass<'program> {}

impl<'program> pass_state::Sealed for FirstFinalRulePass<'program> {}
impl<'program> RuntimeRulePassState<'program> for FirstFinalRulePass<'program> {}

impl<'program> pass_state::Sealed for AfterMissFinalRulePass<'program> {}
impl<'program> RuntimeRulePassState<'program> for AfterMissFinalRulePass<'program> {}

/// Parsed once-only rule that is already consumed in the current run.
#[derive(Debug)]
enum ConsumedRuntimeRule<'program> {
    /// Consumed once-only rewrite rule.
    OnceRewrite(OnceRewriteRuleView<'program>),
    /// Consumed once-only return rule.
    OnceReturn(OnceReturnRuleView<'program>),
}

/// Parsed rule proven available for runtime-state matching.
#[derive(Debug)]
pub(crate) enum AvailableRuntimeRule<'program, 'once> {
    /// Reusable rewrite rule.
    AlwaysRewrite(AvailableAlwaysRewriteRuntimeRule<'program>),
    /// Fresh once-only rewrite rule paired with its linear commit permit.
    OnceRewrite(AvailableOnceRewriteRuntimeRule<'program, 'once>),
    /// Reusable return rule.
    AlwaysReturn(AvailableAlwaysReturnRuntimeRule<'program>),
    /// Fresh once-only return rule paired with its linear commit permit.
    OnceReturn(AvailableOnceReturnRuntimeRule<'program, 'once>),
}

/// Reusable rewrite rule proven available for runtime-state matching.
#[derive(Debug)]
pub(crate) struct AvailableAlwaysRewriteRuntimeRule<'program> {
    /// Position-bearing parsed executable rule.
    rule: AlwaysRewriteRuleView<'program>,
}

/// Fresh once-only rewrite rule paired with the permit that can consume it after a match commits.
#[derive(Debug)]
pub(crate) struct AvailableOnceRewriteRuntimeRule<'program, 'once> {
    /// Position-bearing parsed executable rule.
    rule: OnceRewriteRuleView<'program>,
    /// Linear once-state commit permit.
    commit: OnceMatchPermit<'once>,
}

/// Reusable return rule proven available for runtime-state matching.
#[derive(Debug)]
pub(crate) struct AvailableAlwaysReturnRuntimeRule<'program> {
    /// Position-bearing parsed executable rule.
    rule: AlwaysReturnRuleView<'program>,
}

/// Fresh once-only return rule paired with the permit that can consume it after a match commits.
#[derive(Debug)]
pub(crate) struct AvailableOnceReturnRuntimeRule<'program, 'once> {
    /// Position-bearing parsed executable rule.
    rule: OnceReturnRuleView<'program>,
    /// Linear once-state commit permit.
    commit: OnceMatchPermit<'once>,
}

impl OnceMatchPermitLinearity {
    /// Creates the linearity marker for one permit.
    const fn new() -> Self {
        Self
    }
}

impl<'program> StartedRuntimeRuleTable<'program> {
    /// Moves out the typed started pass.
    pub(crate) fn into_pass(self) -> StartedRuntimeRulePass<'program> {
        self.pass
    }
}

impl<'program> RuntimeRuleTable<'program> {
    /// Builds a run-local ordinary execution table from an executable program.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the per-execution rule table cannot be
    /// allocated.
    pub(crate) fn from_program(
        program: &'program ExecutableProgram,
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
        let (first, remaining_rules) = rules.split_first();
        let mut remaining = Vec::new();
        try_reserve_total_exact(
            &mut remaining,
            RequestedCapacity::new(remaining_rules.len()),
            AllocationContext::RuntimeRuleCell,
        )?;
        for rule in remaining_rules {
            try_push(
                &mut remaining,
                RuntimeRuleCell::new(rule),
                AllocationContext::RuntimeRuleCell,
            )?;
        }

        Ok(Self {
            first: RuntimeRuleCell::new(first),
            remaining,
        })
    }

    /// Finds the first currently available rule that matches `state`.
    pub(crate) fn find_next_match<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuntimeRuleSearch<'program, 'state, 'once> {
        if let Some(matched) = self.first.find_match(state) {
            return RuntimeRuleSearch::Matched(matched);
        }

        for cell in &mut self.remaining {
            if let Some(matched) = cell.find_match(state) {
                return RuntimeRuleSearch::Matched(matched);
            }
        }

        RuntimeRuleSearch::Stable
    }
}

impl<'program> StartedRuntimeRulePass<'program> {
    /// Builds a rule-attempt pass from an executable program.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the per-execution rule-attempt table cannot
    /// be allocated.
    pub(crate) fn from_program(
        program: &'program ExecutableProgram,
    ) -> Result<StartedRuntimeRuleTable<'program>, AllocationError> {
        Ok(StartedRuntimeRuleTable {
            pass: Self::from_rule_scan(program.rule_scan())?,
        })
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
            AllocationContext::RuntimeRuleCell,
        )?;
        for rule in remaining_rules {
            remaining.push_back(RuntimeRuleCell::new(rule));
        }
        let mut attempted = VecDeque::new();
        try_reserve_rule_queue(
            &mut attempted,
            remaining_capacity,
            AllocationContext::RuntimeRuleCell,
        )?;
        Ok(RuntimeRulePassParts {
            current: RuntimeRuleCell::new(first),
            remaining,
            attempted,
        }
        .into_started_pass())
    }
}

impl<'program, History, Tail> RuntimeRulePass<'program, History, Tail> {
    /// Attempts the current target against the current runtime state.
    pub(crate) fn attempt_current<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttempt<'program, 'state, 'once> {
        self.current.attempt(state)
    }
}

impl<'program> RuntimeRulePass<'program, NoMissedRules<'program>, ContinuingRuleTail<'program>> {
    /// Commits the first miss in a continuing pass.
    pub(crate) fn commit_miss(self) -> AfterMissRuntimeRulePass<'program> {
        let Self {
            current,
            history,
            tail,
        } = self;
        let history = history.into_missed(current);
        advance_after_miss(history, tail)
    }

    /// Resets a first-rule continuing pass after a rewrite.
    pub(crate) fn reset_after_rewrite(self) -> StartedRuntimeRulePass<'program> {
        StartedRuntimeRulePass::Continuing(self)
    }
}

impl<'program>
    RuntimeRulePass<'program, MissedRuntimeRules<'program>, ContinuingRuleTail<'program>>
{
    /// Commits another miss in a continuing pass.
    pub(crate) fn commit_miss(self) -> AfterMissRuntimeRulePass<'program> {
        let Self {
            current,
            history,
            tail,
        } = self;
        let history = history.push_missed(current);
        advance_after_miss(history, tail)
    }

    /// Resets a continuing pass with non-empty miss history after a rewrite.
    pub(crate) fn reset_after_rewrite(self) -> StartedRuntimeRulePass<'program> {
        let Self {
            current,
            history,
            tail,
        } = self;
        let (first, mut remaining) = history.into_parts();
        remaining.push_back(current);
        let attempted = tail.append_to(&mut remaining);
        RuntimeRulePassParts {
            current: first,
            remaining,
            attempted,
        }
        .into_started_pass()
    }
}

impl<'program> RuntimeRulePass<'program, NoMissedRules<'program>, FinalRuleTail<'program>> {
    /// Resets a first-rule final pass after a rewrite.
    pub(crate) fn reset_after_rewrite(self) -> StartedRuntimeRulePass<'program> {
        StartedRuntimeRulePass::Final(self)
    }
}

impl<'program> RuntimeRulePass<'program, MissedRuntimeRules<'program>, FinalRuleTail<'program>> {
    /// Resets a final pass with non-empty miss history after a rewrite.
    pub(crate) fn reset_after_rewrite(self) -> StartedRuntimeRulePass<'program> {
        let Self {
            current,
            history,
            tail,
        } = self;
        let (first, mut remaining) = history.into_parts();
        remaining.push_back(current);
        RuntimeRulePassParts {
            current: first,
            remaining,
            attempted: tail.into_spare(),
        }
        .into_started_pass()
    }
}

impl<'program> NoMissedRules<'program> {
    /// Builds empty pass history from a pre-reserved attempted-rule buffer.
    fn new(attempted: VecDeque<RuntimeRuleCell<'program>>) -> Self {
        Self { attempted }
    }

    /// Promotes empty history into a non-empty miss history.
    fn into_missed(self, first: RuntimeRuleCell<'program>) -> MissedRuntimeRules<'program> {
        MissedRuntimeRules {
            first,
            remaining: self.attempted,
        }
    }
}

impl<'program> MissedRuntimeRules<'program> {
    /// Appends a later miss to this non-empty history.
    fn push_missed(mut self, rule: RuntimeRuleCell<'program>) -> Self {
        self.remaining.push_back(rule);
        self
    }

    /// Splits this non-empty history into the first missed rule and later misses.
    fn into_parts(
        self,
    ) -> (
        RuntimeRuleCell<'program>,
        VecDeque<RuntimeRuleCell<'program>>,
    ) {
        (self.first, self.remaining)
    }
}

impl<'program> ContinuingRuleTail<'program> {
    /// Builds a non-empty pending tail from its head and remaining rules.
    fn new(
        next: RuntimeRuleCell<'program>,
        remaining: VecDeque<RuntimeRuleCell<'program>>,
    ) -> Self {
        Self { next, remaining }
    }

    /// Moves the pending tail to the current target after a non-final miss.
    fn advance(mut self) -> (RuntimeRuleCell<'program>, AdvancedRuleTail<'program>) {
        let current = self.next;
        let advanced = match self.remaining.pop_front() {
            Some(next) => AdvancedRuleTail::Continuing(Self::new(next, self.remaining)),
            None => AdvancedRuleTail::Final(FinalRuleTail {
                spare: self.remaining,
            }),
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

impl<'program> FinalRuleTail<'program> {
    /// Consumes this final tail into its empty reset buffer.
    fn into_spare(self) -> VecDeque<RuntimeRuleCell<'program>> {
        self.spare
    }
}

impl<'program> RuntimeRulePassParts<'program> {
    /// Classifies the current target by whether the ordered tail is empty.
    fn into_started_pass(mut self) -> StartedRuntimeRulePass<'program> {
        let history = NoMissedRules::new(self.attempted);
        match self.remaining.pop_front() {
            Some(next) => StartedRuntimeRulePass::Continuing(RuntimeRulePass {
                current: self.current,
                history,
                tail: ContinuingRuleTail::new(next, self.remaining),
            }),
            None => StartedRuntimeRulePass::Final(RuntimeRulePass {
                current: self.current,
                history,
                tail: FinalRuleTail {
                    spare: self.remaining,
                },
            }),
        }
    }
}

/// Advances a continuing pass after the current target has become a miss.
fn advance_after_miss<'program>(
    history: MissedRuntimeRules<'program>,
    tail: ContinuingRuleTail<'program>,
) -> AfterMissRuntimeRulePass<'program> {
    let (current, tail) = tail.advance();
    match tail {
        AdvancedRuleTail::Continuing(tail) => {
            AfterMissRuntimeRulePass::Continuing(RuntimeRulePass {
                current,
                history,
                tail,
            })
        }
        AdvancedRuleTail::Final(tail) => AfterMissRuntimeRulePass::Final(RuntimeRulePass {
            current,
            history,
            tail,
        }),
    }
}

impl<'program> RuntimeRuleCell<'program> {
    /// Builds a runtime rule cell from typed parsed rule data.
    fn new(rule: StoredRuleRef<'program>) -> Self {
        match rule.runtime_rule() {
            RuntimeStoredRule::AlwaysRewrite(rule) => {
                Self::AlwaysRewrite(AlwaysRewriteRuntimeRuleCell { rule })
            }
            RuntimeStoredRule::OnceRewrite(rule) => Self::OnceRewrite(OnceRewriteRuntimeRuleCell {
                rule,
                availability: RuntimeOnceAvailability::Fresh,
            }),
            RuntimeStoredRule::AlwaysReturn(rule) => {
                Self::AlwaysReturn(AlwaysReturnRuntimeRuleCell { rule })
            }
            RuntimeStoredRule::OnceReturn(rule) => Self::OnceReturn(OnceReturnRuntimeRuleCell {
                rule,
                availability: RuntimeOnceAvailability::Fresh,
            }),
        }
    }

    /// Attempts this rule cell against the current runtime state.
    fn attempt<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttempt<'program, 'state, 'once> {
        match self.target() {
            RuntimeRuleTarget::Available(target) => match attempt_available_rule(target, state) {
                AvailableRuleAttempt::Matched(matched) => RuleAttempt::Matched(matched),
                AvailableRuleAttempt::StateMismatch(miss) => RuleAttempt::Missed(miss),
            },
            RuntimeRuleTarget::Consumed(rule) => RuleAttempt::Missed(rule.into_miss()),
        }
    }

    /// Classifies this cell before runtime-state matching.
    fn target<'once>(&'once mut self) -> RuntimeRuleTarget<'program, 'once> {
        match self {
            Self::AlwaysRewrite(cell) => {
                RuntimeRuleTarget::Available(AvailableRuntimeRule::AlwaysRewrite(
                    AvailableAlwaysRewriteRuntimeRule { rule: cell.rule },
                ))
            }
            Self::OnceRewrite(cell) => match cell.availability {
                RuntimeOnceAvailability::Fresh => RuntimeRuleTarget::Available(
                    AvailableRuntimeRule::OnceRewrite(AvailableOnceRewriteRuntimeRule {
                        rule: cell.rule,
                        commit: OnceMatchPermit::new(&mut cell.availability),
                    }),
                ),
                RuntimeOnceAvailability::Committed => {
                    RuntimeRuleTarget::Consumed(ConsumedRuntimeRule::OnceRewrite(cell.rule))
                }
            },
            Self::AlwaysReturn(cell) => {
                RuntimeRuleTarget::Available(AvailableRuntimeRule::AlwaysReturn(
                    AvailableAlwaysReturnRuntimeRule { rule: cell.rule },
                ))
            }
            Self::OnceReturn(cell) => match cell.availability {
                RuntimeOnceAvailability::Fresh => RuntimeRuleTarget::Available(
                    AvailableRuntimeRule::OnceReturn(AvailableOnceReturnRuntimeRule {
                        rule: cell.rule,
                        commit: OnceMatchPermit::new(&mut cell.availability),
                    }),
                ),
                RuntimeOnceAvailability::Committed => {
                    RuntimeRuleTarget::Consumed(ConsumedRuntimeRule::OnceReturn(cell.rule))
                }
            },
        }
    }

    /// Finds this rule as an ordinary execution match, skipping non-applying targets.
    fn find_match<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> Option<MatchedRuleApplication<'program, 'state, 'once>> {
        match self.target() {
            RuntimeRuleTarget::Available(target) => match attempt_available_rule(target, state) {
                AvailableRuleAttempt::Matched(matched) => Some(matched),
                AvailableRuleAttempt::StateMismatch(_) => None,
            },
            RuntimeRuleTarget::Consumed(_) => None,
        }
    }
}

impl<'program> ConsumedRuntimeRule<'program> {
    /// Projects this consumed once-only rule into a non-applying attempt.
    const fn into_miss(self) -> RuleAttemptMiss<'program> {
        match self {
            Self::OnceRewrite(rule) => RuleAttemptMiss::once_rewrite_consumed(rule),
            Self::OnceReturn(rule) => RuleAttemptMiss::once_return_consumed(rule),
        }
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

impl<'program> AvailableAlwaysRewriteRuntimeRule<'program> {
    /// Parsed reusable rewrite rule selected with no once state.
    pub(crate) const fn rule(&self) -> AlwaysRewriteRuleView<'program> {
        self.rule
    }
}

impl<'program, 'once> AvailableOnceRewriteRuntimeRule<'program, 'once> {
    /// Splits this available once target into its rule and linear commit permit.
    pub(crate) fn into_parts(self) -> (OnceRewriteRuleView<'program>, OnceMatchPermit<'once>) {
        (self.rule, self.commit)
    }
}

impl<'program> AvailableAlwaysReturnRuntimeRule<'program> {
    /// Parsed reusable return rule selected with no once state.
    pub(crate) const fn rule(&self) -> AlwaysReturnRuleView<'program> {
        self.rule
    }
}

impl<'program, 'once> AvailableOnceReturnRuntimeRule<'program, 'once> {
    /// Splits this available once target into its rule and linear commit permit.
    pub(crate) fn into_parts(self) -> (OnceReturnRuleView<'program>, OnceMatchPermit<'once>) {
        (self.rule, self.commit)
    }
}

impl<'state> OnceMatchPermit<'state> {
    /// Creates the commit permit after availability has been checked.
    fn new(state: &'state mut RuntimeOnceAvailability) -> Self {
        Self {
            state,
            linearity: OnceMatchPermitLinearity::new(),
        }
    }
}

impl OnceMatchPermit<'_> {
    /// Consumes this permit and marks the owning once-rule state as consumed.
    pub(crate) fn commit(self) {
        let Self {
            state,
            linearity: _linearity,
        } = self;
        *state = RuntimeOnceAvailability::Committed;
    }
}
