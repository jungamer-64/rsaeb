use crate::error::{
    OwnedRuleAttemptStepError, OwnedRunStepError, RuleAttemptStepError, RunFinishError,
    RunStepError,
};
use crate::inspect::RuleView;
use crate::limits::{RuleAttemptCount, StepCount};
use crate::policy::{ExecutionPolicy, ParsePolicy, RuleAttemptPolicy};
use crate::program::{Program, ReturnOutput, RunResult};
use crate::trace::RuntimeStateView;

use super::attempt::{RuleAttemptStableReason, RuleMiss};
use super::engine::RunCore;
use super::session::{
    BorrowedRuleAttemptSession, BorrowedRunSession, OwnedRuleAttemptSession, OwnedRunSession,
};
use super::witness::OwnedRuleWitness;

/// Result of advancing a borrowed run session once.
///
/// Only [`BorrowedStepTransition::Applied`] carries a continuation session. Stable,
/// returned, and failed transitions are terminal.
pub enum BorrowedStepTransition<'program, P: ParsePolicy, E: ExecutionPolicy> {
    /// One ordinary rewrite rule was applied and execution can continue.
    Applied(BorrowedAppliedStep<'program, P, E>),
    /// No rule matched the final runtime state.
    Stable(BorrowedStableRun<'program, P, E>),
    /// A matched rule executed `(return)`.
    Returned(BorrowedReturnedRun<'program, P>),
    /// A matching rule failed before committing.
    Failed(BorrowedFailedRun<'program, P, E>),
}

/// One committed non-terminal rule application in a borrowed session.
pub struct BorrowedAppliedStep<'program, P: ParsePolicy, E: ExecutionPolicy> {
    /// Step number committed by this transition.
    pub(super) step: StepCount,
    /// Borrowed rewrite rule committed by this transition.
    pub(super) rule: RuleView<'program>,
    /// Continuation session after the committed rule application.
    pub(super) session: BorrowedRunSession<'program, P, E>,
}

/// Terminal borrowed run state reached by no matching rule.
pub struct BorrowedStableRun<'program, P: ParsePolicy, E: ExecutionPolicy> {
    /// Number of committed steps before no rule matched.
    pub(super) steps: StepCount,
    /// Parsed program borrowed by the terminal state.
    pub(super) program: &'program Program<P>,
    /// Terminal runtime core containing the stable state.
    pub(super) core: RunCore<E>,
}

/// Terminal borrowed run state reached by `(return)`.
pub struct BorrowedReturnedRun<'program, P: ParsePolicy> {
    /// Step number that executed the return action.
    pub(super) step: StepCount,
    /// Borrowed return rule committed by this transition.
    pub(super) rule: RuleView<'program>,
    /// Parsed program borrowed by the terminal state.
    pub(super) program: &'program Program<P>,
    /// Materialized return output produced by the committed return rule.
    pub(super) output: ReturnOutput,
}

/// Runtime failure that preserves uncommitted borrowed state for inspection.
pub struct BorrowedFailedRun<'program, P: ParsePolicy, E: ExecutionPolicy> {
    /// Runtime error that stopped the candidate step before commit.
    pub(super) error: RunStepError,
    /// Parsed program borrowed by the failed terminal state.
    pub(super) program: &'program Program<P>,
    /// Uncommitted runtime core retained for diagnostic inspection.
    pub(super) core: RunCore<E>,
}

/// Result of advancing a borrowed rule-attempt session once.
///
/// Only [`BorrowedRuleAttemptTransition::Missed`] and [`BorrowedRuleAttemptTransition::Applied`]
/// carry continuation sessions. Stable, returned, and failed transitions are
/// terminal.
pub enum BorrowedRuleAttemptTransition<
    'program,
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
> {
    /// One executable rule line was consumed without applying.
    Missed(BorrowedMissedRuleAttempt<'program, P, E, A>),
    /// One ordinary rewrite rule was applied and execution can continue.
    Applied(BorrowedRuleAttemptAppliedStep<'program, P, E, A>),
    /// The rule pass completed without a match.
    Stable(BorrowedRuleAttemptStableRun<'program, P, E>),
    /// A matched rule executed `(return)`.
    Returned(BorrowedRuleAttemptReturnedRun<'program, P>),
    /// A matching rule failed before committing runtime state.
    Failed(BorrowedRuleAttemptFailedRun<'program, P, E>),
}

/// One consumed non-applying rule line in a borrowed rule-attempt session.
pub struct BorrowedMissedRuleAttempt<
    'program,
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
> {
    /// Rule-attempt count committed by this transition.
    pub(super) attempt: RuleAttemptCount,
    /// Non-applying rule information.
    pub(super) miss: RuleMiss<RuleView<'program>>,
    /// Continuation session after consuming the rule line.
    pub(super) session: BorrowedRuleAttemptSession<'program, P, E, A>,
}

/// One committed non-terminal rule application in a borrowed rule-attempt session.
pub struct BorrowedRuleAttemptAppliedStep<
    'program,
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
> {
    /// Rule-attempt count committed by this transition.
    pub(super) attempt: RuleAttemptCount,
    /// Step number committed by this transition.
    pub(super) step: StepCount,
    /// Borrowed rewrite rule committed by this transition.
    pub(super) rule: RuleView<'program>,
    /// Continuation session after the committed rule application.
    pub(super) session: BorrowedRuleAttemptSession<'program, P, E, A>,
}

/// Terminal borrowed rule-attempt run state reached by no matching rule.
pub struct BorrowedRuleAttemptStableRun<'program, P: ParsePolicy, E: ExecutionPolicy> {
    /// Number of consumed rule attempts before stability.
    pub(super) attempts: RuleAttemptCount,
    /// Number of committed execution steps before stability.
    pub(super) steps: StepCount,
    /// Why the rule-attempt run reached stability.
    pub(super) stable_reason: RuleAttemptStableReason<RuleView<'program>>,
    /// Parsed program borrowed by the terminal state.
    pub(super) program: &'program Program<P>,
    /// Terminal runtime core containing the stable state.
    pub(super) core: RunCore<E>,
}

/// Terminal borrowed rule-attempt run state reached by `(return)`.
pub struct BorrowedRuleAttemptReturnedRun<'program, P: ParsePolicy> {
    /// Rule-attempt count committed by this transition.
    pub(super) attempt: RuleAttemptCount,
    /// Step number that executed the return action.
    pub(super) step: StepCount,
    /// Borrowed return rule committed by this transition.
    pub(super) rule: RuleView<'program>,
    /// Parsed program borrowed by the terminal state.
    pub(super) program: &'program Program<P>,
    /// Materialized return output produced by the committed return rule.
    pub(super) output: ReturnOutput,
}

/// Runtime failure that preserves uncommitted borrowed rule-attempt state for inspection.
pub struct BorrowedRuleAttemptFailedRun<'program, P: ParsePolicy, E: ExecutionPolicy> {
    /// Runtime error that stopped the candidate attempt before commit.
    pub(super) error: RuleAttemptStepError,
    /// Number of rule attempts consumed before the failure was reported.
    pub(super) attempts: RuleAttemptCount,
    /// Parsed program borrowed by the failed terminal state.
    pub(super) program: &'program Program<P>,
    /// Uncommitted runtime core retained for diagnostic inspection.
    pub(super) core: RunCore<E>,
}

/// Result of advancing an owned run session once.
///
/// This mirrors [`BorrowedStepTransition`] while preserving ownership of the parsed
/// program through owned terminal and failed states.
pub enum OwnedStepTransition<P: ParsePolicy, E: ExecutionPolicy> {
    /// One ordinary rewrite rule was applied and execution can continue.
    Applied(OwnedAppliedStep<P, E>),
    /// No rule matched the final runtime state.
    Stable(OwnedStableRun<P, E>),
    /// A matched rule executed `(return)`.
    Returned(OwnedReturnedRun<P>),
    /// A matching rule failed before committing.
    Failed(OwnedFailedRun<P, E>),
}

/// One committed non-terminal rule application.
pub struct OwnedAppliedStep<P: ParsePolicy, E: ExecutionPolicy> {
    /// Step number committed by this transition.
    pub(super) step: StepCount,
    /// Owned rewrite rule witness committed by this transition.
    pub(super) rule: OwnedRuleWitness,
    /// Continuation session after the committed rule application.
    pub(super) session: OwnedRunSession<P, E>,
}

/// Terminal run state reached by no matching rule.
pub struct OwnedStableRun<P: ParsePolicy, E: ExecutionPolicy> {
    /// Number of committed steps before no rule matched.
    pub(super) steps: StepCount,
    /// Parsed program retained by the owned terminal state.
    pub(super) program: Program<P>,
    /// Terminal runtime core containing the stable state.
    pub(super) core: RunCore<E>,
}

/// Terminal run state reached by `(return)`.
pub struct OwnedReturnedRun<P: ParsePolicy> {
    /// Step number that executed the return action.
    pub(super) step: StepCount,
    /// Owned return rule witness committed by this transition.
    pub(super) rule: OwnedRuleWitness,
    /// Parsed program retained by the terminal state.
    pub(super) program: Program<P>,
    /// Materialized return output produced by the committed return rule.
    pub(super) output: ReturnOutput,
}

/// Runtime failure that preserves uncommitted state for inspection.
pub struct OwnedFailedRun<P: ParsePolicy, E: ExecutionPolicy> {
    /// Runtime error that stopped the candidate step before commit.
    pub(super) error: OwnedRunStepError,
    /// Parsed program retained by the failed terminal state.
    pub(super) program: Program<P>,
    /// Uncommitted runtime core retained for diagnostic inspection.
    pub(super) core: RunCore<E>,
}

/// Result of advancing an owned rule-attempt session once.
///
/// This mirrors [`BorrowedRuleAttemptTransition`] while preserving ownership of the
/// parsed program through owned terminal and failed states.
pub enum OwnedRuleAttemptTransition<P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// One executable rule line was consumed without applying.
    Missed(OwnedMissedRuleAttempt<P, E, A>),
    /// One ordinary rewrite rule was applied and execution can continue.
    Applied(OwnedRuleAttemptAppliedStep<P, E, A>),
    /// The rule pass completed without a match.
    Stable(OwnedRuleAttemptStableRun<P, E>),
    /// A matched rule executed `(return)`.
    Returned(OwnedRuleAttemptReturnedRun<P>),
    /// A matching rule failed before committing runtime state.
    Failed(OwnedRuleAttemptFailedRun<P, E>),
}

/// One consumed non-applying rule line.
pub struct OwnedMissedRuleAttempt<P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// Rule-attempt count committed by this transition.
    pub(super) attempt: RuleAttemptCount,
    /// Non-applying rule information.
    pub(super) miss: RuleMiss<OwnedRuleWitness>,
    /// Continuation session after consuming the rule line.
    pub(super) session: OwnedRuleAttemptSession<P, E, A>,
}

/// One committed non-terminal rule application.
pub struct OwnedRuleAttemptAppliedStep<P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// Rule-attempt count committed by this transition.
    pub(super) attempt: RuleAttemptCount,
    /// Step number committed by this transition.
    pub(super) step: StepCount,
    /// Owned rewrite rule witness committed by this transition.
    pub(super) rule: OwnedRuleWitness,
    /// Continuation session after the committed rule application.
    pub(super) session: OwnedRuleAttemptSession<P, E, A>,
}

/// Terminal owned rule-attempt run state reached by no matching rule.
pub struct OwnedRuleAttemptStableRun<P: ParsePolicy, E: ExecutionPolicy> {
    /// Number of consumed rule attempts before stability.
    pub(super) attempts: RuleAttemptCount,
    /// Number of committed execution steps before stability.
    pub(super) steps: StepCount,
    /// Why the rule-attempt run reached stability.
    pub(super) stable_reason: RuleAttemptStableReason<OwnedRuleWitness>,
    /// Parsed program retained by the owned terminal state.
    pub(super) program: Program<P>,
    /// Terminal runtime core containing the stable state.
    pub(super) core: RunCore<E>,
}

/// Terminal owned rule-attempt run state reached by `(return)`.
pub struct OwnedRuleAttemptReturnedRun<P: ParsePolicy> {
    /// Rule-attempt count committed by this transition.
    pub(super) attempt: RuleAttemptCount,
    /// Step number that executed the return action.
    pub(super) step: StepCount,
    /// Owned return rule witness committed by this transition.
    pub(super) rule: OwnedRuleWitness,
    /// Parsed program retained by the terminal state.
    pub(super) program: Program<P>,
    /// Materialized return output produced by the committed return rule.
    pub(super) output: ReturnOutput,
}

/// Runtime failure that preserves uncommitted owned rule-attempt state for inspection.
pub struct OwnedRuleAttemptFailedRun<P: ParsePolicy, E: ExecutionPolicy> {
    /// Runtime error that stopped the candidate attempt before commit.
    pub(super) error: OwnedRuleAttemptStepError,
    /// Number of rule attempts consumed before the failure was reported.
    pub(super) attempts: RuleAttemptCount,
    /// Parsed program retained by the failed terminal state.
    pub(super) program: Program<P>,
    /// Uncommitted runtime core retained for diagnostic inspection.
    pub(super) core: RunCore<E>,
}

impl<'program, P: ParsePolicy, E: ExecutionPolicy> BorrowedAppliedStep<'program, P, E> {
    /// One-based applied step count.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Borrowed rule committed by this transition.
    #[must_use]
    pub const fn rule(&self) -> RuleView<'program> {
        self.rule
    }

    /// Runtime state after the applied step.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Continue running after observing this applied step.
    ///
    /// This is the only borrowed transition that can resume execution.
    #[must_use]
    pub fn into_session(self) -> BorrowedRunSession<'program, P, E> {
        self.session
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> OwnedAppliedStep<P, E> {
    /// One-based applied step count.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Owned rule witness committed by this transition.
    #[must_use]
    pub const fn rule(&self) -> &OwnedRuleWitness {
        &self.rule
    }

    /// Runtime state after the applied step.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Continue running after observing this applied step.
    ///
    /// This is the only owned transition that can resume execution.
    #[must_use]
    pub fn into_session(self) -> OwnedRunSession<P, E> {
        self.session
    }

    /// Splits the applied step into its committed count, owned rule witness, and
    /// continuation session.
    #[must_use]
    pub fn into_parts(self) -> (StepCount, OwnedRuleWitness, OwnedRunSession<P, E>) {
        (self.step, self.rule, self.session)
    }
}

impl<'program, P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy>
    BorrowedMissedRuleAttempt<'program, P, E, A>
{
    /// One-based consumed rule-attempt count.
    #[must_use]
    pub const fn attempt(&self) -> RuleAttemptCount {
        self.attempt
    }

    /// Non-applying rule information.
    #[must_use]
    pub const fn miss(&self) -> &RuleMiss<RuleView<'program>> {
        &self.miss
    }

    /// Runtime state after the non-applying rule attempt.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Continue running after observing this missed rule attempt.
    #[must_use]
    pub fn into_session(self) -> BorrowedRuleAttemptSession<'program, P, E, A> {
        self.session
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy> OwnedMissedRuleAttempt<P, E, A> {
    /// One-based consumed rule-attempt count.
    #[must_use]
    pub const fn attempt(&self) -> RuleAttemptCount {
        self.attempt
    }

    /// Non-applying rule information.
    #[must_use]
    pub const fn miss(&self) -> &RuleMiss<OwnedRuleWitness> {
        &self.miss
    }

    /// Runtime state after the non-applying rule attempt.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Continue running after observing this missed rule attempt.
    #[must_use]
    pub fn into_session(self) -> OwnedRuleAttemptSession<P, E, A> {
        self.session
    }

    /// Splits the missed rule attempt into its committed attempt count, owned
    /// miss witness, and continuation session.
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        RuleAttemptCount,
        RuleMiss<OwnedRuleWitness>,
        OwnedRuleAttemptSession<P, E, A>,
    ) {
        (self.attempt, self.miss, self.session)
    }
}

impl<'program, P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy>
    BorrowedRuleAttemptAppliedStep<'program, P, E, A>
{
    /// One-based consumed rule-attempt count.
    #[must_use]
    pub const fn attempt(&self) -> RuleAttemptCount {
        self.attempt
    }

    /// One-based applied step count.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Borrowed rule committed by this rule-attempt transition.
    #[must_use]
    pub const fn rule(&self) -> RuleView<'program> {
        self.rule
    }

    /// Runtime state after the applied step.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Continue running after observing this applied rule attempt.
    #[must_use]
    pub fn into_session(self) -> BorrowedRuleAttemptSession<'program, P, E, A> {
        self.session
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy>
    OwnedRuleAttemptAppliedStep<P, E, A>
{
    /// One-based consumed rule-attempt count.
    #[must_use]
    pub const fn attempt(&self) -> RuleAttemptCount {
        self.attempt
    }

    /// One-based applied step count.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Owned rule witness committed by this rule-attempt transition.
    #[must_use]
    pub const fn rule(&self) -> &OwnedRuleWitness {
        &self.rule
    }

    /// Runtime state after the applied step.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Continue running after observing this applied rule attempt.
    #[must_use]
    pub fn into_session(self) -> OwnedRuleAttemptSession<P, E, A> {
        self.session
    }

    /// Splits the applied rule attempt into its committed attempt count,
    /// committed step count, owned rule witness, and continuation session.
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        RuleAttemptCount,
        StepCount,
        OwnedRuleWitness,
        OwnedRuleAttemptSession<P, E, A>,
    ) {
        (self.attempt, self.step, self.rule, self.session)
    }
}

impl<'program, P: ParsePolicy, E: ExecutionPolicy> BorrowedStableRun<'program, P, E> {
    /// Number of execution steps committed before reaching the stable state.
    #[must_use]
    pub const fn steps(&self) -> StepCount {
        self.steps
    }

    /// Borrow the parsed program used by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &'program Program<P> {
        self.program
    }

    /// Borrowed final runtime state.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }

    /// Materializes this stable run as a run result.
    ///
    /// # Errors
    ///
    /// Returns `RunFinishError` if final state materialization cannot allocate.
    pub fn into_result(self) -> Result<RunResult, RunFinishError> {
        self.core.into_stable_result(self.steps)
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> OwnedStableRun<P, E> {
    /// Number of execution steps committed before reaching the stable state.
    #[must_use]
    pub const fn steps(&self) -> StepCount {
        self.steps
    }

    /// Borrow the parsed program owned by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &Program<P> {
        &self.program
    }

    /// Discards the terminal state and recovers the owned parsed program.
    ///
    /// This drops the stable runtime state. Use [`OwnedStableRun::into_result`]
    /// when the final state bytes are the desired output.
    #[must_use]
    pub fn into_program(self) -> Program<P> {
        self.program
    }

    /// Borrowed final runtime state.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }

    /// Materializes this stable run as a run result.
    ///
    /// # Errors
    ///
    /// Returns `RunFinishError` if final state materialization cannot allocate.
    pub fn into_result(self) -> Result<RunResult, RunFinishError> {
        self.core.into_stable_result(self.steps)
    }
}

impl<'program, P: ParsePolicy, E: ExecutionPolicy> BorrowedRuleAttemptStableRun<'program, P, E> {
    /// Number of rule attempts consumed before reaching the stable state.
    #[must_use]
    pub const fn attempts(&self) -> RuleAttemptCount {
        self.attempts
    }

    /// Number of execution steps committed before reaching the stable state.
    #[must_use]
    pub const fn steps(&self) -> StepCount {
        self.steps
    }

    /// Why this rule-attempt pass reached stability.
    #[must_use]
    pub const fn stable_reason(&self) -> &RuleAttemptStableReason<RuleView<'program>> {
        &self.stable_reason
    }

    /// Borrow the parsed program used by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &'program Program<P> {
        self.program
    }

    /// Borrowed final runtime state.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }

    /// Materializes this stable run as a run result.
    ///
    /// # Errors
    ///
    /// Returns `RunFinishError` if final state materialization cannot allocate.
    pub fn into_result(self) -> Result<RunResult, RunFinishError> {
        self.core.into_stable_result(self.steps)
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> OwnedRuleAttemptStableRun<P, E> {
    /// Number of rule attempts consumed before reaching the stable state.
    #[must_use]
    pub const fn attempts(&self) -> RuleAttemptCount {
        self.attempts
    }

    /// Number of execution steps committed before reaching the stable state.
    #[must_use]
    pub const fn steps(&self) -> StepCount {
        self.steps
    }

    /// Why this rule-attempt pass reached stability.
    #[must_use]
    pub const fn stable_reason(&self) -> &RuleAttemptStableReason<OwnedRuleWitness> {
        &self.stable_reason
    }

    /// Borrow the parsed program owned by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &Program<P> {
        &self.program
    }

    /// Discards the terminal state and recovers the owned parsed program.
    ///
    /// This drops the stable runtime state. Use
    /// [`OwnedRuleAttemptStableRun::into_result`] when the final state bytes
    /// are the desired output.
    #[must_use]
    pub fn into_program(self) -> Program<P> {
        self.program
    }

    /// Borrowed final runtime state.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }

    /// Materializes this stable run as a run result.
    ///
    /// # Errors
    ///
    /// Returns `RunFinishError` if final state materialization cannot allocate.
    pub fn into_result(self) -> Result<RunResult, RunFinishError> {
        self.core.into_stable_result(self.steps)
    }
}

impl<'program, P: ParsePolicy> BorrowedReturnedRun<'program, P> {
    /// One-based applied step count for the return rule.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Borrow the parsed program used by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &'program Program<P> {
        self.program
    }

    /// Borrowed return rule committed by this terminal state.
    #[must_use]
    pub const fn rule(&self) -> RuleView<'program> {
        self.rule
    }

    /// Materialized return output from runtime execution.
    #[must_use]
    pub const fn output(&self) -> &ReturnOutput {
        &self.output
    }

    /// Materializes this returned run as a run result.
    #[must_use]
    pub fn into_result(self) -> RunResult {
        RunResult::from_return(self.output, self.step)
    }
}

impl<P: ParsePolicy> OwnedReturnedRun<P> {
    /// One-based applied step count for the return rule.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Borrow the parsed program owned by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &Program<P> {
        &self.program
    }

    /// Owned return rule witness committed by this terminal state.
    #[must_use]
    pub const fn rule(&self) -> &OwnedRuleWitness {
        &self.rule
    }

    /// Discards the return output and recovers the owned parsed program.
    ///
    /// This drops the terminal `(return)` output. Use
    /// [`OwnedReturnedRun::into_result`] when the output bytes are the desired
    /// result.
    #[must_use]
    pub fn into_program(self) -> Program<P> {
        self.program
    }

    /// Materialized return output from runtime execution.
    #[must_use]
    pub const fn output(&self) -> &ReturnOutput {
        &self.output
    }

    /// Materializes this returned run as a run result.
    #[must_use]
    pub fn into_result(self) -> RunResult {
        RunResult::from_return(self.output, self.step)
    }
}

impl<'program, P: ParsePolicy> BorrowedRuleAttemptReturnedRun<'program, P> {
    /// One-based consumed rule-attempt count.
    #[must_use]
    pub const fn attempt(&self) -> RuleAttemptCount {
        self.attempt
    }

    /// One-based applied step count for the return rule.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Borrow the parsed program used by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &'program Program<P> {
        self.program
    }

    /// Borrowed return rule committed by this terminal state.
    #[must_use]
    pub const fn rule(&self) -> RuleView<'program> {
        self.rule
    }

    /// Materialized return output from runtime execution.
    #[must_use]
    pub const fn output(&self) -> &ReturnOutput {
        &self.output
    }

    /// Materializes this returned run as a run result.
    #[must_use]
    pub fn into_result(self) -> RunResult {
        RunResult::from_return(self.output, self.step)
    }
}

impl<P: ParsePolicy> OwnedRuleAttemptReturnedRun<P> {
    /// One-based consumed rule-attempt count.
    #[must_use]
    pub const fn attempt(&self) -> RuleAttemptCount {
        self.attempt
    }

    /// One-based applied step count for the return rule.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Borrow the parsed program owned by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &Program<P> {
        &self.program
    }

    /// Owned return rule witness committed by this terminal state.
    #[must_use]
    pub const fn rule(&self) -> &OwnedRuleWitness {
        &self.rule
    }

    /// Discards the return output and recovers the owned parsed program.
    ///
    /// This drops the terminal `(return)` output. Use
    /// [`OwnedRuleAttemptReturnedRun::into_result`] when the output bytes are
    /// the desired result.
    #[must_use]
    pub fn into_program(self) -> Program<P> {
        self.program
    }

    /// Materialized return output from runtime execution.
    #[must_use]
    pub const fn output(&self) -> &ReturnOutput {
        &self.output
    }

    /// Materializes this returned run as a run result.
    #[must_use]
    pub fn into_result(self) -> RunResult {
        RunResult::from_return(self.output, self.step)
    }
}

impl<'program, P: ParsePolicy, E: ExecutionPolicy> BorrowedFailedRun<'program, P, E> {
    /// Captures a failed borrowed session without committing the attempted step.
    pub(super) fn new(
        error: RunStepError,
        program: &'program Program<P>,
        core: RunCore<E>,
    ) -> Self {
        Self {
            error,
            program,
            core,
        }
    }

    /// Runtime error that prevented the step from committing.
    #[must_use]
    pub const fn error(&self) -> &RunStepError {
        &self.error
    }

    /// Number of execution steps that completed before the failed step attempt.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.core.completed_steps()
    }

    /// Borrow the parsed program used by this failed session.
    #[must_use]
    pub fn program(&self) -> &'program Program<P> {
        self.program
    }

    /// Borrow the uncommitted runtime state preserved by this error.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }

    /// Discard the uncommitted run session and return the runtime error.
    ///
    /// Borrowed failed runs are terminal; there is no retryable borrowed
    /// continuation after an uncommitted failure.
    #[must_use]
    pub fn into_error(self) -> RunStepError {
        self.error
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> core::fmt::Display for BorrowedFailedRun<'_, P, E> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> core::error::Error for BorrowedFailedRun<'_, P, E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl<'program, P: ParsePolicy, E: ExecutionPolicy> BorrowedRuleAttemptFailedRun<'program, P, E> {
    /// Captures a failed borrowed rule-attempt session without committing runtime state.
    pub(super) fn new(
        error: RuleAttemptStepError,
        attempts: RuleAttemptCount,
        program: &'program Program<P>,
        core: RunCore<E>,
    ) -> Self {
        Self {
            error,
            attempts,
            program,
            core,
        }
    }

    /// Runtime error that prevented the rule attempt from completing.
    #[must_use]
    pub const fn error(&self) -> &RuleAttemptStepError {
        &self.error
    }

    /// Number of rule attempts consumed before the failure was reported.
    #[must_use]
    pub const fn completed_attempts(&self) -> RuleAttemptCount {
        self.attempts
    }

    /// Number of execution steps that completed before the failed rule attempt.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.core.completed_steps()
    }

    /// Borrow the parsed program used by this failed session.
    #[must_use]
    pub fn program(&self) -> &'program Program<P> {
        self.program
    }

    /// Borrow the uncommitted runtime state preserved by this error.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }

    /// Discard the uncommitted run session and return the runtime error.
    #[must_use]
    pub fn into_error(self) -> RuleAttemptStepError {
        self.error
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> core::fmt::Display
    for BorrowedRuleAttemptFailedRun<'_, P, E>
{
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> core::error::Error
    for BorrowedRuleAttemptFailedRun<'_, P, E>
{
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> OwnedFailedRun<P, E> {
    /// Captures a failed owned session without committing the attempted step.
    pub(super) fn new(error: OwnedRunStepError, program: Program<P>, core: RunCore<E>) -> Self {
        Self {
            error,
            program,
            core,
        }
    }

    /// Runtime error that prevented the step from committing.
    #[must_use]
    pub const fn error(&self) -> &OwnedRunStepError {
        &self.error
    }

    /// Number of execution steps that completed before the failed step attempt.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.core.completed_steps()
    }

    /// Borrow the parsed program owned by this failed session.
    #[must_use]
    pub fn program(&self) -> &Program<P> {
        &self.program
    }

    /// Borrow the uncommitted runtime state preserved by this error.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }

    /// Discard the uncommitted run session and return the runtime error.
    #[must_use]
    pub fn into_error(self) -> OwnedRunStepError {
        self.error
    }

    /// Discards the runtime error and recovers the owned parsed program.
    ///
    /// This drops the failed runtime state. Failed transitions are terminal;
    /// callers cannot resume the failed step by recovering a session.
    #[must_use]
    pub fn into_program(self) -> Program<P> {
        self.program
    }

    /// Splits this failed transition into its runtime error and parsed program.
    #[must_use]
    pub fn into_parts(self) -> (OwnedRunStepError, Program<P>) {
        (self.error, self.program)
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> OwnedRuleAttemptFailedRun<P, E> {
    /// Captures a failed owned rule-attempt session without committing runtime state.
    pub(super) fn new(
        error: OwnedRuleAttemptStepError,
        attempts: RuleAttemptCount,
        program: Program<P>,
        core: RunCore<E>,
    ) -> Self {
        Self {
            error,
            attempts,
            program,
            core,
        }
    }

    /// Runtime error that prevented the rule attempt from completing.
    #[must_use]
    pub const fn error(&self) -> &OwnedRuleAttemptStepError {
        &self.error
    }

    /// Number of rule attempts consumed before the failure was reported.
    #[must_use]
    pub const fn completed_attempts(&self) -> RuleAttemptCount {
        self.attempts
    }

    /// Number of execution steps that completed before the failed rule attempt.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.core.completed_steps()
    }

    /// Borrow the parsed program owned by this failed session.
    #[must_use]
    pub fn program(&self) -> &Program<P> {
        &self.program
    }

    /// Borrow the uncommitted runtime state preserved by this error.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }

    /// Discard the uncommitted run session and return the runtime error.
    #[must_use]
    pub fn into_error(self) -> OwnedRuleAttemptStepError {
        self.error
    }

    /// Discards the runtime error and recovers the owned parsed program.
    ///
    /// This drops the failed runtime state. Failed transitions are terminal;
    /// callers cannot resume the failed rule-attempt step by recovering a
    /// session.
    #[must_use]
    pub fn into_program(self) -> Program<P> {
        self.program
    }

    /// Splits this failed transition into its runtime error and parsed program.
    #[must_use]
    pub fn into_parts(self) -> (OwnedRuleAttemptStepError, Program<P>) {
        (self.error, self.program)
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> core::fmt::Display for OwnedRuleAttemptFailedRun<P, E> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> core::error::Error for OwnedRuleAttemptFailedRun<P, E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> core::fmt::Display for OwnedFailedRun<P, E> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> core::error::Error for OwnedFailedRun<P, E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.error)
    }
}
