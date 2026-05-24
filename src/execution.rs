//! Public stepwise run typestates.
//!
//! [`Program::into_run`](crate::program::Program::into_run) moves a parsed
//! program into a [`RunSession`]. Run-to-completion and tracing use the same
//! internal session shape with a borrowed program owner. Public stepwise
//! execution has one ownership model and no parallel borrowed typestate tree.

use crate::error::{RunError, TracedRunError};
use crate::input::RunSeed;
use crate::inspect::{RulePosition, RuleView};
use crate::limits::StepCount;
use crate::program::{Program, ReturnOutputView, RunResult};
use crate::runtime::action::{
    AppliedRule, CommittedReturnRule, apply_matched_rule, materialize_return_output,
};
use crate::runtime::budget::RuntimeBudgetState;
use crate::runtime::matcher::{RuleSearch, find_next_match};
use crate::runtime::once::OnceStateSet;
use crate::runtime::rewrite::RewriteScratch;
use crate::runtime::state::State;
use crate::trace::{BorrowedTraceEffect, BorrowedTraceEvent, RuntimeStateView};

/// Stateful run session that owns its parsed program.
pub struct RunSession {
    /// Internal session using the public owned program boundary.
    session: Session<OwnedProgram>,
}

/// Mutable runtime state independent of program ownership mode.
#[derive(Debug)]
struct RunCore {
    /// Current runtime byte state.
    state: State,
    /// Reusable buffer for candidate rewrites.
    scratch: RewriteScratch,
    /// Runtime limits and completed-step count.
    budget: RuntimeBudgetState,
    /// Per-run consumption state for `(once)` rules.
    once_states: OnceStateSet,
}

/// Runtime session parameterized by program ownership.
struct Session<P> {
    /// Borrowed or owned parsed program.
    program: P,
    /// Mutable execution state.
    core: RunCore,
}

/// Program ownership shape used by the internal runtime session.
trait ProgramOwner {
    /// Borrows the parsed program.
    fn program(&self) -> &Program;
}

/// Borrowed program owner for run-to-completion and tracing.
#[derive(Debug, Clone, Copy)]
struct BorrowedProgram<'program> {
    /// Parsed program borrowed by this run.
    program: &'program Program,
}

/// Owned program owner for public stepwise execution.
#[derive(Debug)]
struct OwnedProgram {
    /// Parsed program owned by the public run session.
    program: Program,
}

/// Result of advancing a run session once.
pub enum StepTransition {
    /// One ordinary rewrite rule was applied and execution can continue.
    Applied(AppliedStep),
    /// No rule matched the final runtime state.
    Stable(StableRun),
    /// A matched rule executed `(return)`.
    Returned(ReturnedRun),
    /// A matching rule failed before committing.
    Failed(FailedRun),
}

/// One committed non-terminal rule application.
pub struct AppliedStep {
    /// Step number committed by this transition.
    step: StepCount,
    /// Program-local rewrite rule position committed by this transition.
    rule: RulePosition,
    /// Continuation session after the committed rewrite.
    session: RunSession,
}

/// Terminal run state reached by no matching rule.
pub struct StableRun {
    /// Number of committed steps before no rule matched.
    steps: StepCount,
    /// Parsed program retained by the owned terminal state.
    program: Program,
    /// Terminal runtime core containing the stable state.
    core: RunCore,
}

/// Terminal run state reached by `(return)`.
pub struct ReturnedRun {
    /// Step number that executed the return action.
    step: StepCount,
    /// Program-local return rule position.
    rule: RulePosition,
    /// Parsed program used to borrow the return payload.
    program: Program,
}

/// Runtime failure that preserves uncommitted state for inspection.
pub struct FailedRun {
    /// Runtime error that stopped the candidate step before commit.
    error: RunError,
    /// Uncommitted owned session retained for diagnostic inspection.
    session: RunSession,
}

/// Internal non-error result of one core step attempt.
enum CoreStep<'program> {
    /// A rule committed and may have terminal side effects.
    Applied(AppliedRule<'program>),
    /// No rule matched the current runtime state.
    Stable(StepCount),
}

impl ProgramOwner for BorrowedProgram<'_> {
    fn program(&self) -> &Program {
        self.program
    }
}

impl ProgramOwner for OwnedProgram {
    fn program(&self) -> &Program {
        &self.program
    }
}

impl core::fmt::Debug for RunSession {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RunSession")
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for StepTransition {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Applied(applied) => formatter.debug_tuple("Applied").field(applied).finish(),
            Self::Stable(stable) => formatter.debug_tuple("Stable").field(stable).finish(),
            Self::Returned(returned) => formatter.debug_tuple("Returned").field(returned).finish(),
            Self::Failed(failed) => formatter.debug_tuple("Failed").field(failed).finish(),
        }
    }
}

impl core::fmt::Debug for AppliedStep {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AppliedStep")
            .field("step", &self.step())
            .field("rule", &self.rule())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for StableRun {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("StableRun")
            .field("steps", &self.steps())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for ReturnedRun {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ReturnedRun")
            .field("step", &self.step())
            .field("rule", &self.rule())
            .field("output", &self.output())
            .finish()
    }
}

impl core::fmt::Debug for FailedRun {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("FailedRun")
            .field("error", &self.error())
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl RunCore {
    /// Builds the mutable runtime core for one execution.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if per-run rule state allocation fails.
    fn new(program: &Program, seed: RunSeed) -> Result<Self, RunError> {
        let (input, budget) = seed.into_runtime_parts();
        let state = State::from_input(input);
        let once_states = OnceStateSet::new(program.once_slot_count())?;
        Ok(Self {
            state,
            scratch: RewriteScratch::new(),
            budget,
            once_states,
        })
    }

    /// Number of steps already committed in this core.
    const fn completed_steps(&self) -> StepCount {
        self.budget.completed_steps()
    }

    /// Borrows the current runtime state.
    fn state(&self) -> RuntimeStateView<'_> {
        self.state.view()
    }

    /// Materializes a stable terminal result.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if final state materialization cannot allocate.
    fn into_stable_result(self, steps: StepCount) -> Result<RunResult, RunError> {
        Ok(RunResult::stable(self.state.into_snapshot()?, steps))
    }

    /// Advances the mutable runtime core against the supplied immutable program.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if rule matching detects an internal invariant
    /// failure or if applying the matched rule exceeds limits or allocation
    /// fails.
    fn step<'program>(
        &mut self,
        program: &'program Program,
    ) -> Result<CoreStep<'program>, RunError> {
        let matched =
            match find_next_match(program.rule_slice(), &mut self.once_states, &self.state)? {
                RuleSearch::Matched(matched) => matched,
                RuleSearch::Stable => return Ok(CoreStep::Stable(self.budget.completed_steps())),
            };

        Ok(CoreStep::Applied(apply_matched_rule(
            &mut self.state,
            &mut self.scratch,
            &mut self.budget,
            matched,
        )?))
    }
}

impl<P: ProgramOwner> Session<P> {
    /// Starts a new run session for a parsed program and admitted run seed.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if allocating per-run rule state fails.
    fn new(program: P, seed: RunSeed) -> Result<Self, RunError> {
        let core = RunCore::new(program.program(), seed)?;
        Ok(Self { program, core })
    }

    /// Borrows the parsed program.
    fn program(&self) -> &Program {
        self.program.program()
    }

    /// Number of rewrite steps that have already completed in this run.
    const fn completed_steps(&self) -> StepCount {
        self.core.completed_steps()
    }

    /// Borrow the current runtime state.
    fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }

    /// Advances this run by exactly one matching rule when possible.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if rule matching or rule application fails.
    fn step(&mut self) -> Result<CoreStep<'_>, RunError> {
        self.core.step(self.program.program())
    }
}

impl<'program> Session<BorrowedProgram<'program>> {
    /// Advances a borrowed-program session while keeping rule views tied to the
    /// parsed program rather than to the mutable session borrow.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if rule matching or rule application fails.
    fn step_borrowed(&mut self) -> Result<CoreStep<'program>, RunError> {
        self.core.step(self.program.program)
    }

    /// Runs this borrowed session to completion.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when a later matching rule would exceed configured
    /// limits.
    fn finish_borrowed(mut self) -> Result<RunResult, RunError> {
        loop {
            match self.step_borrowed()? {
                CoreStep::Applied(AppliedRule::Rewrite(_)) => {}
                CoreStep::Applied(AppliedRule::Return(committed)) => {
                    return committed.into_result();
                }
                CoreStep::Stable(steps) => return self.core.into_stable_result(steps),
            }
        }
    }

    /// Runs to completion while emitting borrowed trace events.
    ///
    /// # Errors
    ///
    /// Returns `TracedRunError::Trace` if the trace sink fails. Returns
    /// `TracedRunError::Run` if runtime execution fails.
    fn run_with_borrowed_trace<F, E>(mut self, mut trace: F) -> Result<RunResult, TracedRunError<E>>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), E>,
    {
        trace(BorrowedTraceEvent::Initial {
            state: self.state(),
        })
        .map_err(TracedRunError::Trace)?;

        loop {
            match self.step_borrowed().map_err(TracedRunError::Run)? {
                CoreStep::Applied(AppliedRule::Rewrite(committed)) => {
                    Self::emit_step_trace(
                        &mut trace,
                        committed.step(),
                        committed.rule(),
                        BorrowedTraceEffect::Continue {
                            state: self.state(),
                        },
                    )?;
                }
                CoreStep::Applied(AppliedRule::Return(committed)) => {
                    Self::emit_step_trace(
                        &mut trace,
                        committed.step(),
                        committed.rule(),
                        BorrowedTraceEffect::Return {
                            output: committed.output(),
                        },
                    )?;
                    return committed.into_result().map_err(TracedRunError::Run);
                }
                CoreStep::Stable(steps) => {
                    return self
                        .core
                        .into_stable_result(steps)
                        .map_err(TracedRunError::Run);
                }
            }
        }
    }

    /// Emits one borrowed step trace event.
    ///
    /// # Errors
    ///
    /// Returns `TracedRunError::Trace` if the trace sink rejects the event.
    fn emit_step_trace<F, E>(
        trace: &mut F,
        step: StepCount,
        rule: RuleView<'program>,
        effect: BorrowedTraceEffect<'program, '_>,
    ) -> Result<(), TracedRunError<E>>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), E>,
    {
        trace(BorrowedTraceEvent::Step { step, rule, effect }).map_err(TracedRunError::Trace)
    }
}

impl CommittedReturnRule<'_> {
    /// Materializes this returned run as a run result.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if return output materialization cannot allocate.
    fn into_result(self) -> Result<RunResult, RunError> {
        Ok(RunResult::from_return(
            materialize_return_output(self.output())?,
            self.step(),
        ))
    }
}

impl Session<OwnedProgram> {
    /// Splits an owned session into its program and mutable core.
    fn into_program_core(self) -> (Program, RunCore) {
        (self.program.program, self.core)
    }
}

/// Runs a borrowed program to completion.
///
/// # Errors
///
/// Returns `RunError` when execution setup fails or a later matching rule would
/// exceed configured limits.
pub(crate) fn finish_borrowed_run(program: &Program, seed: RunSeed) -> Result<RunResult, RunError> {
    Session::new(BorrowedProgram { program }, seed)?.finish_borrowed()
}

/// Runs a borrowed program to completion while emitting borrowed trace events.
///
/// # Errors
///
/// Returns `TracedRunError::Run` for runtime failures and
/// `TracedRunError::Trace` for user callback failures.
pub(crate) fn run_with_borrowed_trace<'program, F, E>(
    program: &'program Program,
    seed: RunSeed,
    trace: F,
) -> Result<RunResult, TracedRunError<E>>
where
    F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), E>,
{
    Session::new(BorrowedProgram { program }, seed)
        .map_err(TracedRunError::Run)?
        .run_with_borrowed_trace(trace)
}

impl RunSession {
    /// Starts a new owned run session for a parsed program and admitted run seed.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if allocating per-run rule state fails.
    pub(crate) fn new(program: Program, seed: RunSeed) -> Result<Self, RunError> {
        Ok(Self {
            session: Session::new(OwnedProgram { program }, seed)?,
        })
    }

    /// Number of rewrite steps that have already completed in this run.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.session.completed_steps()
    }

    /// Borrow the parsed program owned by this session.
    #[must_use]
    pub fn program(&self) -> &Program {
        self.session.program()
    }

    /// Borrow the current runtime state.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Advances this run by exactly one matching rule when possible.
    #[must_use]
    pub fn step(mut self) -> StepTransition {
        match self.session.step() {
            Ok(CoreStep::Applied(AppliedRule::Rewrite(committed))) => {
                let step = committed.step();
                let rule = committed.rule().position();
                StepTransition::Applied(AppliedStep {
                    step,
                    rule,
                    session: self,
                })
            }
            Ok(CoreStep::Applied(AppliedRule::Return(committed))) => {
                let step = committed.step();
                let rule = committed.rule().position();
                let (program, _core) = self.session.into_program_core();
                StepTransition::Returned(ReturnedRun {
                    step,
                    rule,
                    program,
                })
            }
            Ok(CoreStep::Stable(steps)) => {
                let (program, core) = self.session.into_program_core();
                StepTransition::Stable(StableRun {
                    steps,
                    program,
                    core,
                })
            }
            Err(error) => StepTransition::Failed(FailedRun::new(error, self)),
        }
    }

    /// Runs this session to completion.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when applying a later matching rule would exceed the
    /// configured limits, allocation fails, or state-size arithmetic overflows.
    pub fn finish(mut self) -> Result<RunResult, RunError> {
        loop {
            match self.step() {
                StepTransition::Applied(applied) => {
                    self = applied.into_session();
                }
                StepTransition::Stable(stable) => {
                    return stable.into_result();
                }
                StepTransition::Returned(returned) => {
                    return returned.into_result();
                }
                StepTransition::Failed(failed) => return Err(failed.into_error()),
            }
        }
    }
}

impl AppliedStep {
    /// One-based applied step count.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Structured view of the applied rule.
    ///
    /// # Errors
    ///
    /// Returns `RunError::InternalInvariant` if the committed rule position no
    /// longer resolves inside the owned parsed program.
    pub fn rule(&self) -> Result<RuleView<'_>, RunError> {
        self.session.program().rule_view_at(self.rule)
    }

    /// Runtime state after the applied rewrite step.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Continue running after observing this applied step.
    #[must_use]
    pub fn into_session(self) -> RunSession {
        self.session
    }
}

impl StableRun {
    /// Number of rewrite steps applied before reaching the stable state.
    #[must_use]
    pub const fn steps(&self) -> StepCount {
        self.steps
    }

    /// Borrow the parsed program owned by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &Program {
        &self.program
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
    /// Returns `RunError` if final state materialization cannot allocate.
    pub fn into_result(self) -> Result<RunResult, RunError> {
        self.core.into_stable_result(self.steps)
    }
}

impl ReturnedRun {
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

    /// Structured view of the return rule.
    ///
    /// # Errors
    ///
    /// Returns `RunError::InternalInvariant` if the committed rule position no
    /// longer resolves inside the owned parsed program.
    pub fn rule(&self) -> Result<RuleView<'_>, RunError> {
        self.program.rule_view_at(self.rule)
    }

    /// Borrowed return output from runtime execution.
    ///
    /// # Errors
    ///
    /// Returns `RunError::InternalInvariant` if the committed return rule no
    /// longer resolves to a `(return)` action.
    pub fn output(&self) -> Result<ReturnOutputView<'_>, RunError> {
        self.program.return_output_at(self.rule)
    }

    /// Materializes this returned run as a run result.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if return output materialization cannot allocate.
    pub fn into_result(self) -> Result<RunResult, RunError> {
        Ok(RunResult::from_return(
            materialize_return_output(self.output()?)?,
            self.step,
        ))
    }
}

impl FailedRun {
    /// Captures a failed owned session without committing the attempted step.
    fn new(error: RunError, session: RunSession) -> Self {
        Self { error, session }
    }

    /// Runtime error that prevented the step from committing.
    #[must_use]
    pub const fn error(&self) -> &RunError {
        &self.error
    }

    /// Number of rewrite steps that completed before the failed step attempt.
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
    pub fn into_error(self) -> RunError {
        self.error
    }
}

impl core::fmt::Display for FailedRun {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl core::error::Error for FailedRun {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.error)
    }
}
