use alloc::{collections::VecDeque, vec::Vec};

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::inspect::{
    AlwaysReturnRuleView, AlwaysRewriteRuleView, OnceReturnRuleView, OnceRewriteRuleView,
};
use crate::program::{ExecutableProgram, RuleScan, RuntimeStoredRule, StoredRuleRef};
use crate::runtime::matcher::{
    EvaluatedRuleMiss, MatchedRuleApplication, RuleAttemptEvaluation, attempt_always_return_rule,
    attempt_always_rewrite_rule, attempt_once_return_rule, attempt_once_rewrite_rule,
};
use crate::runtime::state::State;

/// Per-run ordinary execution table with parsed rules and once-cell state.
#[derive(Debug)]
pub(crate) struct RuntimeRuleTable<'program> {
    /// First runtime rule cell, preserving executable non-emptiness.
    first: RuntimeRuleCell<'program>,
    /// Remaining runtime rule cells in parser execution order.
    remaining: Vec<RuntimeRuleCell<'program>>,
}

/// Outcome of scanning the ordinary runtime rule table.
#[derive(Debug)]
pub(crate) enum RuntimeRuleScan<'program, 'state, 'once> {
    /// A rule matched and carries the commit permit needed after success.
    Matched(MatchedRuleApplication<'program, 'state, 'once>),
    /// No executable rule matched the current runtime state.
    Unmatched,
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

/// Newly started rule-attempt pass paired with its per-run once-state table.
#[derive(Debug)]
pub(crate) struct StartedRuntimeRuleTable<'program> {
    /// Rule-attempt pass classified by current-tail shape.
    pass: FirstRuntimeRulePassCursor<'program>,
}

/// Runtime pass classified only by whether the current target has a successor.
#[derive(Debug)]
pub(crate) enum RuntimeRulePassCursor<'program, History> {
    /// Current rule has at least one successor in this pass.
    Continuing(RuntimeRulePass<'program, History, ContinuingRuleTail<'program>>),
    /// Current rule exhausts this pass.
    Final(RuntimeRulePass<'program, History, FinalRuleTail<'program>>),
}

/// Rule-attempt pass at the start of a scan.
pub(crate) type FirstRuntimeRulePassCursor<'program> =
    RuntimeRulePassCursor<'program, NoMissedRules<'program>>;

/// Rule-attempt pass after the history has become non-empty.
pub(crate) type MissedRuntimeRulePassCursor<'program> =
    RuntimeRulePassCursor<'program, MissedRuntimeRules<'program>>;

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

/// Shared rule-attempt pass behavior for every typed runtime pass shape.
pub(crate) trait RuleAttemptPass<'program>:
    RuntimeRulePassState<'program> + rule_attempt_pass::Sealed + Sized
{
    /// Attempts this pass's current target.
    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttemptEvaluation<'program, 'state, 'once>;

    /// Resets this pass after a committed rewrite.
    fn reset_attempt_after_rewrite(self) -> FirstRuntimeRulePassCursor<'program>;
}

/// Continuing rule-attempt pass behavior owned by the runtime pass state.
pub(crate) trait ContinuingRuleAttemptPass<'program>:
    RuleAttemptPass<'program> + continuing_pass::Sealed + Sized
{
    /// Commits a miss and advances to the next typed pass.
    fn commit_attempt_miss(self) -> MissedRuntimeRulePassCursor<'program>;
}

/// Final rule-attempt pass behavior owned by the runtime pass state.
pub(crate) trait FinalRuleAttemptPass<'program>:
    RuleAttemptPass<'program> + final_pass::Sealed + Sized
{
}

/// Boundary for tails that can rebuild a pass after non-empty miss history.
trait MissedRuntimeRuleTail<'program>: missed_tail::Sealed {
    /// Appends reset-time pending rules after `remaining` and returns the buffer
    /// reserved for future misses in the rebuilt pass.
    fn append_to_reset_remaining(
        self,
        remaining: &mut ResetRemainingRules<'program>,
    ) -> FutureMissBuffer<'program>;
}

/// Private sealing traits for runtime pass states.
pub(crate) mod pass_state {
    /// Marker implemented only by valid rule-attempt pass shapes.
    pub(crate) trait Sealed {}
}

/// Private sealing traits for shared runtime pass capabilities.
pub(crate) mod rule_attempt_pass {
    /// Marker implemented only by valid rule-attempt pass shapes.
    pub(crate) trait Sealed {}
}

/// Private sealing traits for continuing runtime pass capabilities.
pub(crate) mod continuing_pass {
    /// Marker implemented only by continuing pass states.
    pub(crate) trait Sealed {}
}

/// Private sealing traits for final runtime pass capabilities.
pub(crate) mod final_pass {
    /// Marker implemented only by final pass states.
    pub(crate) trait Sealed {}
}

/// Private sealing traits for reset-capable missed-pass tails.
mod missed_tail {
    /// Marker implemented only by valid missed-pass tail shapes.
    pub(super) trait Sealed {}
}

/// Empty pass history with a pre-reserved buffer for future misses.
#[derive(Debug)]
pub(crate) struct NoMissedRules<'program> {
    /// Empty buffer reused if the first miss commits.
    future_misses: FutureMissBuffer<'program>,
}

/// Non-empty pass history after at least one rule has missed.
#[derive(Debug)]
pub(crate) struct MissedRuntimeRules<'program> {
    /// First rule missed in the current pass.
    first: RuntimeRuleCell<'program>,
    /// Later missed rules in original rule order.
    tail: MissedRuleHistoryTail<'program>,
}

/// Non-empty tail of unattempted rules after a continuing current target.
#[derive(Debug)]
pub(crate) struct ContinuingRuleTail<'program> {
    /// Next rule after the current target.
    next: RuntimeRuleCell<'program>,
    /// Remaining rules after `next`, in original rule order.
    remaining: PendingRuleTail<'program>,
}

/// Empty tail for a final current target.
#[derive(Debug)]
pub(crate) struct FinalRuleTail<'program> {
    /// Empty pre-reserved buffer reused if a later rewrite resets the pass.
    future_misses: FutureMissBuffer<'program>,
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
    remaining: PendingRuleTail<'program>,
    /// Empty pre-reserved buffer for missed rules in the current pass.
    attempted: FutureMissBuffer<'program>,
}

/// Pending executable rules that have not yet been attempted in this pass.
#[derive(Debug)]
struct PendingRuleTail<'program> {
    /// Runtime rule cells waiting after the current target.
    rules: VecDeque<RuntimeRuleCell<'program>>,
}

/// Later missed rules after the first miss in a non-empty history.
#[derive(Debug)]
struct MissedRuleHistoryTail<'program> {
    /// Missed runtime rule cells in original rule order.
    rules: VecDeque<RuntimeRuleCell<'program>>,
}

/// Reserved buffer for future misses in the current pass.
#[derive(Debug)]
struct FutureMissBuffer<'program> {
    /// Spare runtime rule cell storage for miss history growth.
    rules: VecDeque<RuntimeRuleCell<'program>>,
}

/// Rebuilt pending rules after a rewrite resets a non-empty miss history.
struct ResetRemainingRules<'program> {
    /// Runtime rule cells that will become the rebuilt pending tail.
    rules: VecDeque<RuntimeRuleCell<'program>>,
}

/// One executable rule classified by its run-local availability shape.
#[derive(Debug)]
enum RuntimeRuleCell<'program> {
    /// Reusable non-terminal rewrite rule.
    AlwaysRewrite(AlwaysRewriteRuntimeRuleCell<'program>),
    /// Fresh once-only non-terminal rewrite rule.
    FreshOnceRewrite(FreshOnceRewriteRuntimeRuleCell<'program>),
    /// Consumed once-only non-terminal rewrite rule.
    ConsumedOnceRewrite(ConsumedOnceRewriteRuntimeRuleCell<'program>),
    /// Reusable terminal return rule.
    AlwaysReturn(AlwaysReturnRuntimeRuleCell<'program>),
    /// Fresh once-only terminal return rule.
    FreshOnceReturn(FreshOnceReturnRuntimeRuleCell<'program>),
}

/// Runtime cell for a reusable rewrite rule.
#[derive(Debug)]
struct AlwaysRewriteRuntimeRuleCell<'program> {
    /// Position-bearing parsed executable rule.
    rule: AlwaysRewriteRuleView<'program>,
}

/// Runtime cell for a fresh once-only rewrite rule.
#[derive(Debug)]
struct FreshOnceRewriteRuntimeRuleCell<'program> {
    /// Position-bearing parsed executable rule.
    rule: OnceRewriteRuleView<'program>,
}

/// Runtime cell for a consumed once-only rewrite rule.
#[derive(Debug)]
struct ConsumedOnceRewriteRuntimeRuleCell<'program> {
    /// Position-bearing parsed executable rule.
    rule: OnceRewriteRuleView<'program>,
}

/// Runtime cell for a reusable return rule.
#[derive(Debug)]
struct AlwaysReturnRuntimeRuleCell<'program> {
    /// Position-bearing parsed executable rule.
    rule: AlwaysReturnRuleView<'program>,
}

/// Runtime cell for a fresh once-only return rule.
#[derive(Debug)]
struct FreshOnceReturnRuntimeRuleCell<'program> {
    /// Position-bearing parsed executable rule.
    rule: OnceReturnRuleView<'program>,
}

/// Linear commit permit for a matched fresh once-only rewrite rule.
#[derive(Debug)]
pub(crate) struct OnceRewriteCommitPermit<'program, 'once> {
    /// Runtime cell to consume if the prepared rewrite commits.
    cell: &'once mut RuntimeRuleCell<'program>,
    /// Rule witness used to rebuild the consumed cell variant.
    rule: OnceRewriteRuleView<'program>,
    /// Non-copy token that keeps this permit linear.
    linearity: OnceRewriteCommitLinearity,
}

/// Non-copy marker carried by once-rewrite commit permits.
#[derive(Debug)]
struct OnceRewriteCommitLinearity;

impl<'program> pass_state::Sealed for FirstContinuingRulePass<'program> {}
impl<'program> RuntimeRulePassState<'program> for FirstContinuingRulePass<'program> {}
impl<'program> rule_attempt_pass::Sealed for FirstContinuingRulePass<'program> {}
impl<'program> RuleAttemptPass<'program> for FirstContinuingRulePass<'program> {
    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttemptEvaluation<'program, 'state, 'once> {
        self.attempt_current(state)
    }

    fn reset_attempt_after_rewrite(self) -> FirstRuntimeRulePassCursor<'program> {
        self.reset_after_rewrite()
    }
}

impl<'program> continuing_pass::Sealed for FirstContinuingRulePass<'program> {}
impl<'program> ContinuingRuleAttemptPass<'program> for FirstContinuingRulePass<'program> {
    fn commit_attempt_miss(self) -> MissedRuntimeRulePassCursor<'program> {
        self.commit_miss()
    }
}

impl<'program> pass_state::Sealed for AfterMissContinuingRulePass<'program> {}
impl<'program> RuntimeRulePassState<'program> for AfterMissContinuingRulePass<'program> {}
impl<'program> rule_attempt_pass::Sealed for AfterMissContinuingRulePass<'program> {}
impl<'program> RuleAttemptPass<'program> for AfterMissContinuingRulePass<'program> {
    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttemptEvaluation<'program, 'state, 'once> {
        self.attempt_current(state)
    }

    fn reset_attempt_after_rewrite(self) -> FirstRuntimeRulePassCursor<'program> {
        self.reset_after_rewrite()
    }
}

impl<'program> continuing_pass::Sealed for AfterMissContinuingRulePass<'program> {}
impl<'program> ContinuingRuleAttemptPass<'program> for AfterMissContinuingRulePass<'program> {
    fn commit_attempt_miss(self) -> MissedRuntimeRulePassCursor<'program> {
        self.commit_miss()
    }
}

impl<'program> pass_state::Sealed for FirstFinalRulePass<'program> {}
impl<'program> RuntimeRulePassState<'program> for FirstFinalRulePass<'program> {}
impl<'program> rule_attempt_pass::Sealed for FirstFinalRulePass<'program> {}
impl<'program> RuleAttemptPass<'program> for FirstFinalRulePass<'program> {
    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttemptEvaluation<'program, 'state, 'once> {
        self.attempt_current(state)
    }

    fn reset_attempt_after_rewrite(self) -> FirstRuntimeRulePassCursor<'program> {
        self.reset_after_rewrite()
    }
}

impl<'program> final_pass::Sealed for FirstFinalRulePass<'program> {}
impl<'program> FinalRuleAttemptPass<'program> for FirstFinalRulePass<'program> {}

impl<'program> pass_state::Sealed for AfterMissFinalRulePass<'program> {}
impl<'program> RuntimeRulePassState<'program> for AfterMissFinalRulePass<'program> {}
impl<'program> rule_attempt_pass::Sealed for AfterMissFinalRulePass<'program> {}
impl<'program> RuleAttemptPass<'program> for AfterMissFinalRulePass<'program> {
    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttemptEvaluation<'program, 'state, 'once> {
        self.attempt_current(state)
    }

    fn reset_attempt_after_rewrite(self) -> FirstRuntimeRulePassCursor<'program> {
        self.reset_after_rewrite()
    }
}

impl<'program> final_pass::Sealed for AfterMissFinalRulePass<'program> {}
impl<'program> FinalRuleAttemptPass<'program> for AfterMissFinalRulePass<'program> {}

impl<'program> missed_tail::Sealed for ContinuingRuleTail<'program> {}
impl<'program> MissedRuntimeRuleTail<'program> for ContinuingRuleTail<'program> {
    fn append_to_reset_remaining(
        self,
        output: &mut ResetRemainingRules<'program>,
    ) -> FutureMissBuffer<'program> {
        let Self {
            next,
            mut remaining,
        } = self;
        output.push_back(next);
        remaining.drain_into(output);
        remaining.into_future_miss_buffer()
    }
}

impl<'program> missed_tail::Sealed for FinalRuleTail<'program> {}
impl<'program> MissedRuntimeRuleTail<'program> for FinalRuleTail<'program> {
    fn append_to_reset_remaining(
        self,
        _output: &mut ResetRemainingRules<'program>,
    ) -> FutureMissBuffer<'program> {
        self.into_future_miss_buffer()
    }
}

impl<'program> StartedRuntimeRuleTable<'program> {
    /// Moves out the typed started pass.
    pub(crate) fn into_pass_cursor(self) -> FirstRuntimeRulePassCursor<'program> {
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

    /// Scans executable rules until one matches or every rule has a typed miss.
    pub(crate) fn scan_for_match<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuntimeRuleScan<'program, 'state, 'once> {
        match self.first.attempt(state) {
            RuleAttemptEvaluation::Matched(matched) => return RuntimeRuleScan::Matched(matched),
            RuleAttemptEvaluation::Miss(_) => {}
        };

        for cell in &mut self.remaining {
            match cell.attempt(state) {
                RuleAttemptEvaluation::Matched(matched) => {
                    return RuntimeRuleScan::Matched(matched);
                }
                RuleAttemptEvaluation::Miss(_) => {}
            }
        }

        RuntimeRuleScan::Unmatched
    }
}

impl<'program> StartedRuntimeRuleTable<'program> {
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
            pass: RuntimeRulePassParts::from_rule_scan(program.rule_scan())?,
        })
    }
}

impl<'program> RuntimeRulePassParts<'program> {
    /// Builds a rule-attempt pass from the executable rule table.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the per-execution rule-attempt table cannot
    /// be allocated.
    fn from_rule_scan(
        rules: RuleScan<'program>,
    ) -> Result<FirstRuntimeRulePassCursor<'program>, AllocationError> {
        let (first, remaining_rules) = rules.split_first();
        let remaining_capacity = RequestedCapacity::new(remaining_rules.len());
        let mut remaining = PendingRuleTail::with_capacity(remaining_capacity)?;
        for rule in remaining_rules {
            remaining.push_back(RuntimeRuleCell::new(rule));
        }
        let attempted = FutureMissBuffer::with_capacity(remaining_capacity)?;
        Ok(RuntimeRulePassParts {
            current: RuntimeRuleCell::new(first),
            remaining,
            attempted,
        }
        .into_first_pass_cursor())
    }
}

impl<'program, History, Tail> RuntimeRulePass<'program, History, Tail> {
    /// Attempts the current target against the current runtime state.
    pub(crate) fn attempt_current<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttemptEvaluation<'program, 'state, 'once> {
        self.current.attempt(state)
    }
}

impl<'program> RuntimeRulePass<'program, NoMissedRules<'program>, ContinuingRuleTail<'program>> {
    /// Commits the first miss in a continuing pass.
    pub(crate) fn commit_miss(self) -> MissedRuntimeRulePassCursor<'program> {
        let Self {
            current,
            history,
            tail,
        } = self;
        let history = history.into_missed(current);
        advance_after_miss(history, tail)
    }

    /// Resets a first-rule continuing pass after a rewrite.
    pub(crate) fn reset_after_rewrite(self) -> FirstRuntimeRulePassCursor<'program> {
        RuntimeRulePassCursor::Continuing(self)
    }
}

impl<'program>
    RuntimeRulePass<'program, MissedRuntimeRules<'program>, ContinuingRuleTail<'program>>
{
    /// Commits another miss in a continuing pass.
    pub(crate) fn commit_miss(self) -> MissedRuntimeRulePassCursor<'program> {
        let Self {
            current,
            history,
            tail,
        } = self;
        let history = history.push_missed(current);
        advance_after_miss(history, tail)
    }
}

impl<'program> RuntimeRulePass<'program, NoMissedRules<'program>, FinalRuleTail<'program>> {
    /// Resets a first-rule final pass after a rewrite.
    pub(crate) fn reset_after_rewrite(self) -> FirstRuntimeRulePassCursor<'program> {
        RuntimeRulePassCursor::Final(self)
    }
}

impl<'program>
    RuntimeRulePass<'program, MissedRuntimeRules<'program>, ContinuingRuleTail<'program>>
{
    /// Resets an after-miss continuing pass after a rewrite.
    pub(crate) fn reset_after_rewrite(self) -> FirstRuntimeRulePassCursor<'program> {
        reset_after_missed_rewrite(self)
    }
}

impl<'program> RuntimeRulePass<'program, MissedRuntimeRules<'program>, FinalRuleTail<'program>> {
    /// Resets an after-miss final pass after a rewrite.
    pub(crate) fn reset_after_rewrite(self) -> FirstRuntimeRulePassCursor<'program> {
        reset_after_missed_rewrite(self)
    }
}

impl<'program> NoMissedRules<'program> {
    /// Builds empty pass history from a pre-reserved attempted-rule buffer.
    fn new(future_misses: FutureMissBuffer<'program>) -> Self {
        Self { future_misses }
    }

    /// Promotes empty history into a non-empty miss history.
    fn into_missed(self, first: RuntimeRuleCell<'program>) -> MissedRuntimeRules<'program> {
        MissedRuntimeRules {
            first,
            tail: self.future_misses.into_missed_history_tail(),
        }
    }
}

impl<'program> MissedRuntimeRules<'program> {
    /// Appends a later miss to this non-empty history.
    fn push_missed(mut self, rule: RuntimeRuleCell<'program>) -> Self {
        self.tail.push_back(rule);
        self
    }

    /// Splits this non-empty history into the first missed rule and later misses.
    fn into_parts(self) -> (RuntimeRuleCell<'program>, MissedRuleHistoryTail<'program>) {
        (self.first, self.tail)
    }
}

impl<'program> ContinuingRuleTail<'program> {
    /// Builds a non-empty pending tail from its head and remaining rules.
    fn new(next: RuntimeRuleCell<'program>, remaining: PendingRuleTail<'program>) -> Self {
        Self { next, remaining }
    }

    /// Moves the pending tail to the current target after a non-final miss.
    fn advance(mut self) -> (RuntimeRuleCell<'program>, AdvancedRuleTail<'program>) {
        let current = self.next;
        let advanced = match self.remaining.pop_front() {
            Some(next) => AdvancedRuleTail::Continuing(Self::new(next, self.remaining)),
            None => AdvancedRuleTail::Final(FinalRuleTail::new(
                self.remaining.into_future_miss_buffer(),
            )),
        };
        (current, advanced)
    }
}

impl<'program> FinalRuleTail<'program> {
    /// Builds a final tail with a reserved future-miss buffer.
    fn new(future_misses: FutureMissBuffer<'program>) -> Self {
        Self { future_misses }
    }

    /// Consumes this final tail into its future-miss buffer.
    fn into_future_miss_buffer(self) -> FutureMissBuffer<'program> {
        self.future_misses
    }
}

impl<'program> RuntimeRulePassParts<'program> {
    /// Classifies the current target by whether the ordered tail is empty.
    fn into_first_pass_cursor(mut self) -> FirstRuntimeRulePassCursor<'program> {
        let history = NoMissedRules::new(self.attempted);
        match self.remaining.pop_front() {
            Some(next) => RuntimeRulePassCursor::Continuing(RuntimeRulePass {
                current: self.current,
                history,
                tail: ContinuingRuleTail::new(next, self.remaining),
            }),
            None => RuntimeRulePassCursor::Final(RuntimeRulePass {
                current: self.current,
                history,
                tail: FinalRuleTail::new(self.remaining.into_future_miss_buffer()),
            }),
        }
    }
}

impl<'program> PendingRuleTail<'program> {
    /// Builds an empty pending tail with enough capacity for all non-current rules.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the pending-rule tail cannot reserve the
    /// requested runtime-rule capacity.
    fn with_capacity(total_capacity: RequestedCapacity) -> Result<Self, AllocationError> {
        let mut rules = VecDeque::new();
        try_reserve_rule_queue(
            &mut rules,
            total_capacity,
            AllocationContext::RuntimeRuleCell,
        )?;
        Ok(Self { rules })
    }

    /// Appends one pending runtime rule after the current target.
    fn push_back(&mut self, rule: RuntimeRuleCell<'program>) {
        self.rules.push_back(rule);
    }

    /// Pops the next pending runtime rule, if any.
    fn pop_front(&mut self) -> Option<RuntimeRuleCell<'program>> {
        self.rules.pop_front()
    }

    /// Drains all pending rules into reset remaining order.
    fn drain_into(&mut self, output: &mut ResetRemainingRules<'program>) {
        while let Some(rule) = self.pop_front() {
            output.push_back(rule);
        }
    }

    /// Reuses this pending tail as the future-miss buffer after its rules move elsewhere.
    fn into_future_miss_buffer(self) -> FutureMissBuffer<'program> {
        FutureMissBuffer { rules: self.rules }
    }
}

impl<'program> MissedRuleHistoryTail<'program> {
    /// Appends one later missed runtime rule.
    fn push_back(&mut self, rule: RuntimeRuleCell<'program>) {
        self.rules.push_back(rule);
    }

    /// Reclassifies missed-history tail storage as reset remaining rules.
    fn into_reset_remaining(self) -> ResetRemainingRules<'program> {
        ResetRemainingRules { rules: self.rules }
    }
}

impl<'program> FutureMissBuffer<'program> {
    /// Builds an empty future-miss buffer with capacity for this pass's tail.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the future-miss buffer cannot reserve the
    /// requested runtime-rule capacity.
    fn with_capacity(total_capacity: RequestedCapacity) -> Result<Self, AllocationError> {
        let mut rules = VecDeque::new();
        try_reserve_rule_queue(
            &mut rules,
            total_capacity,
            AllocationContext::RuntimeRuleCell,
        )?;
        Ok(Self { rules })
    }

    /// Reuses this empty future-miss buffer as the tail of a non-empty miss history.
    fn into_missed_history_tail(self) -> MissedRuleHistoryTail<'program> {
        MissedRuleHistoryTail { rules: self.rules }
    }
}

impl<'program> ResetRemainingRules<'program> {
    /// Reclassifies the later missed rules as reset remaining rules.
    fn from_missed_history_tail(history_tail: MissedRuleHistoryTail<'program>) -> Self {
        history_tail.into_reset_remaining()
    }

    /// Appends one runtime rule to the reset-time remaining rules.
    fn push_back(&mut self, rule: RuntimeRuleCell<'program>) {
        self.rules.push_back(rule);
    }

    /// Reclassifies reset remaining rules as the pending tail of a rebuilt pass.
    fn into_pending_tail(self) -> PendingRuleTail<'program> {
        PendingRuleTail { rules: self.rules }
    }
}

/// Rebuilds the first-pass cursor after a rewrite commits behind non-empty miss history.
fn reset_after_missed_rewrite<'program, Tail>(
    pass: RuntimeRulePass<'program, MissedRuntimeRules<'program>, Tail>,
) -> FirstRuntimeRulePassCursor<'program>
where
    Tail: MissedRuntimeRuleTail<'program>,
{
    let RuntimeRulePass {
        current,
        history,
        tail,
    } = pass;
    let (first, history_tail) = history.into_parts();
    let mut remaining = ResetRemainingRules::from_missed_history_tail(history_tail);
    remaining.push_back(current);
    let attempted = tail.append_to_reset_remaining(&mut remaining);
    RuntimeRulePassParts {
        current: first,
        remaining: remaining.into_pending_tail(),
        attempted,
    }
    .into_first_pass_cursor()
}

/// Advances a continuing pass after the current target has become a miss.
fn advance_after_miss<'program>(
    history: MissedRuntimeRules<'program>,
    tail: ContinuingRuleTail<'program>,
) -> MissedRuntimeRulePassCursor<'program> {
    let (current, tail) = tail.advance();
    match tail {
        AdvancedRuleTail::Continuing(tail) => RuntimeRulePassCursor::Continuing(RuntimeRulePass {
            current,
            history,
            tail,
        }),
        AdvancedRuleTail::Final(tail) => RuntimeRulePassCursor::Final(RuntimeRulePass {
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
            RuntimeStoredRule::OnceRewrite(rule) => {
                Self::FreshOnceRewrite(FreshOnceRewriteRuntimeRuleCell { rule })
            }
            RuntimeStoredRule::AlwaysReturn(rule) => {
                Self::AlwaysReturn(AlwaysReturnRuntimeRuleCell { rule })
            }
            RuntimeStoredRule::OnceReturn(rule) => {
                Self::FreshOnceReturn(FreshOnceReturnRuntimeRuleCell { rule })
            }
        }
    }

    /// Attempts this rule cell against the current runtime state.
    fn attempt<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttemptEvaluation<'program, 'state, 'once> {
        match self {
            Self::AlwaysRewrite(cell) => attempt_always_rewrite_rule(cell.rule, state),
            Self::FreshOnceRewrite(cell) => {
                let rule = cell.rule;
                let commit = OnceRewriteCommitPermit::new(self, rule);
                attempt_once_rewrite_rule(rule, commit, state)
            }
            Self::ConsumedOnceRewrite(cell) => {
                RuleAttemptEvaluation::Miss(EvaluatedRuleMiss::OnceRewriteConsumed(cell.rule))
            }
            Self::AlwaysReturn(cell) => attempt_always_return_rule(cell.rule, state),
            Self::FreshOnceReturn(cell) => {
                let rule = cell.rule;
                attempt_once_return_rule(rule, state)
            }
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

impl<'program, 'once> OnceRewriteCommitPermit<'program, 'once> {
    /// Creates the commit permit for a fresh once-only rewrite cell.
    fn new(
        cell: &'once mut RuntimeRuleCell<'program>,
        rule: OnceRewriteRuleView<'program>,
    ) -> Self {
        Self {
            cell,
            rule,
            linearity: OnceRewriteCommitLinearity::new(),
        }
    }

    /// Consumes this permit and marks the owning once-rewrite cell as consumed.
    pub(crate) fn commit(self) {
        let Self {
            cell,
            rule,
            linearity: _linearity,
        } = self;
        *cell = RuntimeRuleCell::ConsumedOnceRewrite(ConsumedOnceRewriteRuntimeRuleCell { rule });
    }
}

impl OnceRewriteCommitLinearity {
    /// Creates the non-copy marker for one once-rewrite permit.
    const fn new() -> Self {
        Self
    }
}
