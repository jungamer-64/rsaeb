use crate::error::{
    OwnedRuleAttemptStepError, OwnedRunStepError, RuleAttemptStepError, RunFinishError,
    RunStepError,
};
use crate::inspect::RuleView;
use crate::limits::{RuleAttemptCount, StepCount};
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
pub enum BorrowedStepTransition<'program> {
    /// One ordinary rewrite rule was applied and execution can continue.
    Applied(BorrowedAppliedStep<'program>),
    /// No rule matched the final runtime state.
    Stable(BorrowedStableRun<'program>),
    /// A matched rule executed `(return)`.
    Returned(BorrowedReturnedRun<'program>),
    /// A matching rule failed before committing.
    Failed(BorrowedFailedRun<'program>),
}

/// One committed non-terminal rule application in a borrowed session.
pub struct BorrowedAppliedStep<'program> {
    /// Step number committed by this transition.
    pub(super) step: StepCount,
    /// Borrowed rewrite rule committed by this transition.
    pub(super) rule: RuleView<'program>,
    /// Continuation session after the committed rule application.
    pub(super) session: BorrowedRunSession<'program>,
}

/// Terminal borrowed run state reached by no matching rule.
pub struct BorrowedStableRun<'program> {
    /// Number of committed steps before no rule matched.
    pub(super) steps: StepCount,
    /// Parsed program borrowed by the terminal state.
    pub(super) program: &'program Program,
    /// Terminal runtime core containing the stable state.
    pub(super) core: RunCore,
}

/// Terminal borrowed run state reached by `(return)`.
pub struct BorrowedReturnedRun<'program> {
    /// Step number that executed the return action.
    pub(super) step: StepCount,
    /// Borrowed return rule committed by this transition.
    pub(super) rule: RuleView<'program>,
    /// Parsed program borrowed by the terminal state.
    pub(super) program: &'program Program,
    /// Materialized return output produced by the committed return rule.
    pub(super) output: ReturnOutput,
}

/// Runtime failure that preserves uncommitted borrowed state for inspection.
pub struct BorrowedFailedRun<'program> {
    /// Runtime error that stopped the candidate step before commit.
    pub(super) error: RunStepError,
    /// Uncommitted borrowed session retained for diagnostic inspection.
    pub(super) session: BorrowedRunSession<'program>,
}

/// Result of advancing a borrowed rule-attempt session once.
///
/// Only [`BorrowedRuleAttemptTransition::Missed`] and [`BorrowedRuleAttemptTransition::Applied`]
/// carry continuation sessions. Stable, returned, and failed transitions are
/// terminal.
pub enum BorrowedRuleAttemptTransition<'program> {
    /// One executable rule line was consumed without applying.
    Missed(BorrowedMissedRuleAttempt<'program>),
    /// One ordinary rewrite rule was applied and execution can continue.
    Applied(BorrowedRuleAttemptAppliedStep<'program>),
    /// The rule pass completed without a match.
    Stable(BorrowedRuleAttemptStableRun<'program>),
    /// A matched rule executed `(return)`.
    Returned(BorrowedRuleAttemptReturnedRun<'program>),
    /// A matching rule failed before committing runtime state.
    Failed(BorrowedRuleAttemptFailedRun<'program>),
}

/// One consumed non-applying rule line in a borrowed rule-attempt session.
pub struct BorrowedMissedRuleAttempt<'program> {
    /// Rule-attempt count committed by this transition.
    pub(super) attempt: RuleAttemptCount,
    /// Non-applying rule information.
    pub(super) miss: RuleMiss<RuleView<'program>>,
    /// Continuation session after consuming the rule line.
    pub(super) session: BorrowedRuleAttemptSession<'program>,
}

/// One committed non-terminal rule application in a borrowed rule-attempt session.
pub struct BorrowedRuleAttemptAppliedStep<'program> {
    /// Rule-attempt count committed by this transition.
    pub(super) attempt: RuleAttemptCount,
    /// Step number committed by this transition.
    pub(super) step: StepCount,
    /// Borrowed rewrite rule committed by this transition.
    pub(super) rule: RuleView<'program>,
    /// Continuation session after the committed rule application.
    pub(super) session: BorrowedRuleAttemptSession<'program>,
}

/// Terminal borrowed rule-attempt run state reached by no matching rule.
pub struct BorrowedRuleAttemptStableRun<'program> {
    /// Number of consumed rule attempts before stability.
    pub(super) attempts: RuleAttemptCount,
    /// Number of committed execution steps before stability.
    pub(super) steps: StepCount,
    /// Why the rule-attempt run reached stability.
    pub(super) stable_reason: RuleAttemptStableReason<RuleView<'program>>,
    /// Parsed program borrowed by the terminal state.
    pub(super) program: &'program Program,
    /// Terminal runtime core containing the stable state.
    pub(super) core: RunCore,
}

/// Terminal borrowed rule-attempt run state reached by `(return)`.
pub struct BorrowedRuleAttemptReturnedRun<'program> {
    /// Rule-attempt count committed by this transition.
    pub(super) attempt: RuleAttemptCount,
    /// Step number that executed the return action.
    pub(super) step: StepCount,
    /// Borrowed return rule committed by this transition.
    pub(super) rule: RuleView<'program>,
    /// Parsed program borrowed by the terminal state.
    pub(super) program: &'program Program,
    /// Materialized return output produced by the committed return rule.
    pub(super) output: ReturnOutput,
}

/// Runtime failure that preserves uncommitted borrowed rule-attempt state for inspection.
pub struct BorrowedRuleAttemptFailedRun<'program> {
    /// Runtime error that stopped the candidate attempt before commit.
    pub(super) error: RuleAttemptStepError,
    /// Uncommitted borrowed session retained for diagnostic inspection.
    pub(super) session: BorrowedRuleAttemptSession<'program>,
}

/// Result of advancing an owned run session once.
///
/// This mirrors [`BorrowedStepTransition`] while preserving ownership of the parsed
/// program through owned terminal and failed states.
pub enum OwnedStepTransition {
    /// One ordinary rewrite rule was applied and execution can continue.
    Applied(OwnedAppliedStep),
    /// No rule matched the final runtime state.
    Stable(OwnedStableRun),
    /// A matched rule executed `(return)`.
    Returned(OwnedReturnedRun),
    /// A matching rule failed before committing.
    Failed(OwnedFailedRun),
}

/// One committed non-terminal rule application.
pub struct OwnedAppliedStep {
    /// Step number committed by this transition.
    pub(super) step: StepCount,
    /// Owned rewrite rule witness committed by this transition.
    pub(super) rule: OwnedRuleWitness,
    /// Continuation session after the committed rule application.
    pub(super) session: OwnedRunSession,
}

/// Terminal run state reached by no matching rule.
pub struct OwnedStableRun {
    /// Number of committed steps before no rule matched.
    pub(super) steps: StepCount,
    /// Parsed program retained by the owned terminal state.
    pub(super) program: Program,
    /// Terminal runtime core containing the stable state.
    pub(super) core: RunCore,
}

/// Terminal run state reached by `(return)`.
pub struct OwnedReturnedRun {
    /// Step number that executed the return action.
    pub(super) step: StepCount,
    /// Owned return rule witness committed by this transition.
    pub(super) rule: OwnedRuleWitness,
    /// Parsed program retained by the terminal state.
    pub(super) program: Program,
    /// Materialized return output produced by the committed return rule.
    pub(super) output: ReturnOutput,
}

/// Runtime failure that preserves uncommitted state for inspection.
pub struct OwnedFailedRun {
    /// Runtime error that stopped the candidate step before commit.
    pub(super) error: OwnedRunStepError,
    /// Uncommitted owned session retained for diagnostic inspection.
    pub(super) session: OwnedRunSession,
}

/// Result of advancing an owned rule-attempt session once.
///
/// This mirrors [`BorrowedRuleAttemptTransition`] while preserving ownership of the
/// parsed program through owned terminal and failed states.
pub enum OwnedRuleAttemptTransition {
    /// One executable rule line was consumed without applying.
    Missed(OwnedMissedRuleAttempt),
    /// One ordinary rewrite rule was applied and execution can continue.
    Applied(OwnedRuleAttemptAppliedStep),
    /// The rule pass completed without a match.
    Stable(OwnedRuleAttemptStableRun),
    /// A matched rule executed `(return)`.
    Returned(OwnedRuleAttemptReturnedRun),
    /// A matching rule failed before committing runtime state.
    Failed(OwnedRuleAttemptFailedRun),
}

/// One consumed non-applying rule line.
pub struct OwnedMissedRuleAttempt {
    /// Rule-attempt count committed by this transition.
    pub(super) attempt: RuleAttemptCount,
    /// Non-applying rule information.
    pub(super) miss: RuleMiss<OwnedRuleWitness>,
    /// Continuation session after consuming the rule line.
    pub(super) session: OwnedRuleAttemptSession,
}

/// One committed non-terminal rule application.
pub struct OwnedRuleAttemptAppliedStep {
    /// Rule-attempt count committed by this transition.
    pub(super) attempt: RuleAttemptCount,
    /// Step number committed by this transition.
    pub(super) step: StepCount,
    /// Owned rewrite rule witness committed by this transition.
    pub(super) rule: OwnedRuleWitness,
    /// Continuation session after the committed rule application.
    pub(super) session: OwnedRuleAttemptSession,
}

/// Terminal owned rule-attempt run state reached by no matching rule.
pub struct OwnedRuleAttemptStableRun {
    /// Number of consumed rule attempts before stability.
    pub(super) attempts: RuleAttemptCount,
    /// Number of committed execution steps before stability.
    pub(super) steps: StepCount,
    /// Why the rule-attempt run reached stability.
    pub(super) stable_reason: RuleAttemptStableReason<OwnedRuleWitness>,
    /// Parsed program retained by the owned terminal state.
    pub(super) program: Program,
    /// Terminal runtime core containing the stable state.
    pub(super) core: RunCore,
}

/// Terminal owned rule-attempt run state reached by `(return)`.
pub struct OwnedRuleAttemptReturnedRun {
    /// Rule-attempt count committed by this transition.
    pub(super) attempt: RuleAttemptCount,
    /// Step number that executed the return action.
    pub(super) step: StepCount,
    /// Owned return rule witness committed by this transition.
    pub(super) rule: OwnedRuleWitness,
    /// Parsed program retained by the terminal state.
    pub(super) program: Program,
    /// Materialized return output produced by the committed return rule.
    pub(super) output: ReturnOutput,
}

/// Runtime failure that preserves uncommitted owned rule-attempt state for inspection.
pub struct OwnedRuleAttemptFailedRun {
    /// Runtime error that stopped the candidate attempt before commit.
    pub(super) error: OwnedRuleAttemptStepError,
    /// Uncommitted owned session retained for diagnostic inspection.
    pub(super) session: OwnedRuleAttemptSession,
}

impl<'program> BorrowedAppliedStep<'program> {
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
    pub fn into_session(self) -> BorrowedRunSession<'program> {
        self.session
    }
}

impl OwnedAppliedStep {
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
    pub fn into_session(self) -> OwnedRunSession {
        self.session
    }

    /// Splits the applied step into its committed count, owned rule witness, and
    /// continuation session.
    #[must_use]
    pub fn into_parts(self) -> (StepCount, OwnedRuleWitness, OwnedRunSession) {
        (self.step, self.rule, self.session)
    }
}

impl<'program> BorrowedMissedRuleAttempt<'program> {
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
    pub fn into_session(self) -> BorrowedRuleAttemptSession<'program> {
        self.session
    }
}

impl OwnedMissedRuleAttempt {
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
    pub fn into_session(self) -> OwnedRuleAttemptSession {
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
        OwnedRuleAttemptSession,
    ) {
        (self.attempt, self.miss, self.session)
    }
}

impl<'program> BorrowedRuleAttemptAppliedStep<'program> {
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
    pub fn into_session(self) -> BorrowedRuleAttemptSession<'program> {
        self.session
    }
}

impl OwnedRuleAttemptAppliedStep {
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
    pub fn into_session(self) -> OwnedRuleAttemptSession {
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
        OwnedRuleAttemptSession,
    ) {
        (self.attempt, self.step, self.rule, self.session)
    }
}

impl<'program> BorrowedStableRun<'program> {
    /// Number of execution steps committed before reaching the stable state.
    #[must_use]
    pub const fn steps(&self) -> StepCount {
        self.steps
    }

    /// Borrow the parsed program used by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &'program Program {
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

impl OwnedStableRun {
    /// Number of execution steps committed before reaching the stable state.
    #[must_use]
    pub const fn steps(&self) -> StepCount {
        self.steps
    }

    /// Borrow the parsed program owned by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &Program {
        &self.program
    }

    /// Discards the terminal state and recovers the owned parsed program.
    ///
    /// This drops the stable runtime state. Use [`OwnedStableRun::into_result`]
    /// when the final state bytes are the desired output.
    #[must_use]
    pub fn into_program(self) -> Program {
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

impl<'program> BorrowedRuleAttemptStableRun<'program> {
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
    pub const fn program(&self) -> &'program Program {
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

impl OwnedRuleAttemptStableRun {
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
    pub const fn program(&self) -> &Program {
        &self.program
    }

    /// Discards the terminal state and recovers the owned parsed program.
    ///
    /// This drops the stable runtime state. Use
    /// [`OwnedRuleAttemptStableRun::into_result`] when the final state bytes
    /// are the desired output.
    #[must_use]
    pub fn into_program(self) -> Program {
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

impl<'program> BorrowedReturnedRun<'program> {
    /// One-based applied step count for the return rule.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Borrow the parsed program used by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &'program Program {
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

impl OwnedReturnedRun {
    /// One-based applied step count for the return rule.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Borrow the parsed program owned by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &Program {
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
    pub fn into_program(self) -> Program {
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

impl<'program> BorrowedRuleAttemptReturnedRun<'program> {
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
    pub const fn program(&self) -> &'program Program {
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

impl OwnedRuleAttemptReturnedRun {
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
    pub const fn program(&self) -> &Program {
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
    pub fn into_program(self) -> Program {
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

impl<'program> BorrowedFailedRun<'program> {
    /// Captures a failed borrowed session without committing the attempted step.
    pub(super) fn new(error: RunStepError, session: BorrowedRunSession<'program>) -> Self {
        Self { error, session }
    }

    /// Runtime error that prevented the step from committing.
    #[must_use]
    pub const fn error(&self) -> &RunStepError {
        &self.error
    }

    /// Number of execution steps that completed before the failed step attempt.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.session.completed_steps()
    }

    /// Borrow the parsed program used by this failed session.
    #[must_use]
    pub fn program(&self) -> &'program Program {
        self.session.program()
    }

    /// Borrow the uncommitted runtime state preserved by this error.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
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

impl core::fmt::Display for BorrowedFailedRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl core::error::Error for BorrowedFailedRun<'_> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl<'program> BorrowedRuleAttemptFailedRun<'program> {
    /// Captures a failed borrowed rule-attempt session without committing runtime state.
    pub(super) fn new(
        error: RuleAttemptStepError,
        session: BorrowedRuleAttemptSession<'program>,
    ) -> Self {
        Self { error, session }
    }

    /// Runtime error that prevented the rule attempt from completing.
    #[must_use]
    pub const fn error(&self) -> &RuleAttemptStepError {
        &self.error
    }

    /// Number of rule attempts consumed before the failure was reported.
    #[must_use]
    pub const fn completed_attempts(&self) -> RuleAttemptCount {
        self.session.completed_attempts()
    }

    /// Number of execution steps that completed before the failed rule attempt.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.session.completed_steps()
    }

    /// Borrow the parsed program used by this failed session.
    #[must_use]
    pub fn program(&self) -> &'program Program {
        self.session.program()
    }

    /// Borrow the uncommitted runtime state preserved by this error.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Discard the uncommitted run session and return the runtime error.
    #[must_use]
    pub fn into_error(self) -> RuleAttemptStepError {
        self.error
    }
}

impl core::fmt::Display for BorrowedRuleAttemptFailedRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl core::error::Error for BorrowedRuleAttemptFailedRun<'_> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl OwnedFailedRun {
    /// Captures a failed owned session without committing the attempted step.
    pub(super) fn new(error: OwnedRunStepError, session: OwnedRunSession) -> Self {
        Self { error, session }
    }

    /// Runtime error that prevented the step from committing.
    #[must_use]
    pub const fn error(&self) -> &OwnedRunStepError {
        &self.error
    }

    /// Number of execution steps that completed before the failed step attempt.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.session.completed_steps()
    }

    /// Borrow the parsed program owned by this failed session.
    #[must_use]
    pub fn program(&self) -> &Program {
        self.session.program()
    }

    /// Borrow the uncommitted runtime state preserved by this error.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
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
    pub fn into_program(self) -> Program {
        self.session.into_program()
    }

    /// Splits this failed transition into its runtime error and parsed program.
    #[must_use]
    pub fn into_parts(self) -> (OwnedRunStepError, Program) {
        let program = self.session.into_program();
        (self.error, program)
    }
}

impl OwnedRuleAttemptFailedRun {
    /// Captures a failed owned rule-attempt session without committing runtime state.
    pub(super) fn new(error: OwnedRuleAttemptStepError, session: OwnedRuleAttemptSession) -> Self {
        Self { error, session }
    }

    /// Runtime error that prevented the rule attempt from completing.
    #[must_use]
    pub const fn error(&self) -> &OwnedRuleAttemptStepError {
        &self.error
    }

    /// Number of rule attempts consumed before the failure was reported.
    #[must_use]
    pub const fn completed_attempts(&self) -> RuleAttemptCount {
        self.session.completed_attempts()
    }

    /// Number of execution steps that completed before the failed rule attempt.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.session.completed_steps()
    }

    /// Borrow the parsed program owned by this failed session.
    #[must_use]
    pub fn program(&self) -> &Program {
        self.session.program()
    }

    /// Borrow the uncommitted runtime state preserved by this error.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
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
    pub fn into_program(self) -> Program {
        self.session.into_program()
    }

    /// Splits this failed transition into its runtime error and parsed program.
    #[must_use]
    pub fn into_parts(self) -> (OwnedRuleAttemptStepError, Program) {
        let program = self.session.into_program();
        (self.error, program)
    }
}

impl core::fmt::Display for OwnedRuleAttemptFailedRun {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl core::error::Error for OwnedRuleAttemptFailedRun {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl core::fmt::Display for OwnedFailedRun {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl core::error::Error for OwnedFailedRun {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.error)
    }
}
