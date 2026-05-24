//! Public stepwise run typestates.
//!
//! [`Program::start_run`](crate::program::Program::start_run) borrows a parsed
//! program into a [`RunSession`]. [`Program::into_run`](crate::program::Program::into_run)
//! is the explicit owned variant for hosts that need a `'static` session.
//! [`Program::run`](crate::program::Program::run) is the borrowed
//! run-to-completion shortcut over the same admitted [`RunSeed`] boundary.
//!
//! A step transition is a typestate value, not a status flag. Applied steps
//! carry the continuation session. Stable and returned states are terminal.
//! Failed states are also terminal for the borrowed API: they preserve the
//! uncommitted state for diagnostics and then let the caller discard the run
//! into its [`RunError`]. Owned failed states additionally let the caller
//! recover the uncommitted owned session or split it from the error.
//!
//! ```
//! use rsaeb::error::{LimitError, RunError};
//! use rsaeb::execution::StepTransition;
//! use rsaeb::input::{RunSeed, RuntimeInput, RuntimeInputSource};
//! use rsaeb::limits::{
//!     DEFAULT_MAX_INPUT_LEN, DEFAULT_MAX_RETURN_LEN, DEFAULT_PARSE_LIMITS, ExecutionLimits,
//!     RuntimeInputLimits, RuntimeStateByteLimit, StepLimit,
//! };
//! use rsaeb::program::Program;
//! use rsaeb::source::ProgramSource;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::parse(ProgramSource::from_text("a=aaaa"), DEFAULT_PARSE_LIMITS)?;
//! let input_limits = RuntimeInputLimits::new(DEFAULT_MAX_INPUT_LEN);
//! let execution_limits = ExecutionLimits::new(
//!     StepLimit::new(10),
//!     RuntimeStateByteLimit::new(1),
//!     DEFAULT_MAX_RETURN_LEN,
//! );
//! let input = RuntimeInput::validate(RuntimeInputSource::from_bytes(b"a"), input_limits)?;
//! let session = program.start_run(RunSeed::admit(input, execution_limits)?)?;
//!
//! let StepTransition::Failed(failed) = session.step() else {
//!     return Err("expected oversized rewrite to fail before commit".into());
//! };
//!
//! if failed.completed_steps().get() != 0 {
//!     return Err("failed step must not commit progress".into());
//! }
//! if failed.state().materialize()?.as_slice() != b"a" {
//!     return Err("failed step must expose the uncommitted state".into());
//! }
//! if !matches!(
//!     failed.error(),
//!     RunError::Limit(LimitError::State { attempted_len, .. })
//!         if attempted_len.get() == 4
//! ) {
//!     return Err("unexpected failed-step error".into());
//! }
//! # Ok(())
//! # }
//! ```

use crate::error::{RunError, TracedRunError};
use crate::input::RunSeed;
use crate::inspect::{RulePosition, RuleView};
use crate::limits::StepCount;
use crate::program::{Program, ReturnOutput, RunResult};
use crate::runtime::action::{AppliedRule, CommittedReturnRule, apply_matched_rule};
use crate::runtime::budget::RuntimeBudgetState;
use crate::runtime::matcher::{RuleSearch, find_next_match};
use crate::runtime::once::OnceStateSet;
use crate::runtime::rewrite::RewriteScratch;
use crate::runtime::state::State;
use crate::trace::{BorrowedTraceEffect, BorrowedTraceEvent, RuntimeStateView};

/// Stateful run session that borrows a reusable parsed program.
///
/// This is the stepwise form returned by
/// [`Program::start_run`](crate::program::Program::start_run). It consumes
/// itself on every step so callers must handle the returned [`StepTransition`]
/// before they can continue.
pub struct RunSession<'program> {
    /// Internal session using the public borrowed program boundary.
    session: Session<BorrowedProgram<'program>>,
}

/// Stateful run session that owns its parsed program.
///
/// This is the stepwise form returned by
/// [`Program::into_run`](crate::program::Program::into_run). It is useful when
/// the session must move independently of a borrowed [`Program`]. Owned
/// terminal and failed states retain a way to recover the parsed program
/// instead of leaking ownership through a parallel API.
pub struct OwnedRunSession {
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

/// Result of advancing a borrowed run session once.
///
/// Only [`StepTransition::Applied`] carries a continuation session. Stable,
/// returned, and failed transitions are terminal.
pub enum StepTransition<'program> {
    /// One ordinary rewrite rule was applied and execution can continue.
    Applied(AppliedStep<'program>),
    /// No rule matched the final runtime state.
    Stable(StableRun<'program>),
    /// A matched rule executed `(return)`.
    Returned(ReturnedRun<'program>),
    /// A matching rule failed before committing.
    Failed(FailedRun<'program>),
}

/// One committed non-terminal rule application in a borrowed session.
pub struct AppliedStep<'program> {
    /// Step number committed by this transition.
    step: StepCount,
    /// Program-local rewrite rule position committed by this transition.
    rule_position: RulePosition,
    /// Continuation session after the committed rewrite.
    session: RunSession<'program>,
}

/// Terminal borrowed run state reached by no matching rule.
pub struct StableRun<'program> {
    /// Number of committed steps before no rule matched.
    steps: StepCount,
    /// Parsed program borrowed by the terminal state.
    program: &'program Program,
    /// Terminal runtime core containing the stable state.
    core: RunCore,
}

/// Terminal borrowed run state reached by `(return)`.
pub struct ReturnedRun<'program> {
    /// Step number that executed the return action.
    step: StepCount,
    /// Program-local return rule position.
    rule_position: RulePosition,
    /// Parsed program borrowed by the terminal state.
    program: &'program Program,
    /// Materialized return output produced by the committed return rule.
    output: ReturnOutput,
}

/// Runtime failure that preserves uncommitted borrowed state for inspection.
pub struct FailedRun<'program> {
    /// Runtime error that stopped the candidate step before commit.
    error: RunError,
    /// Uncommitted borrowed session retained for diagnostic inspection.
    session: RunSession<'program>,
}

/// Result of advancing an owned run session once.
///
/// This mirrors [`StepTransition`] while preserving ownership of the parsed
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
    step: StepCount,
    /// Program-local rewrite rule position committed by this transition.
    rule_position: RulePosition,
    /// Continuation session after the committed rewrite.
    session: OwnedRunSession,
}

/// Terminal run state reached by no matching rule.
pub struct OwnedStableRun {
    /// Number of committed steps before no rule matched.
    steps: StepCount,
    /// Parsed program retained by the owned terminal state.
    program: Program,
    /// Terminal runtime core containing the stable state.
    core: RunCore,
}

/// Terminal run state reached by `(return)`.
pub struct OwnedReturnedRun {
    /// Step number that executed the return action.
    step: StepCount,
    /// Program-local return rule position.
    rule_position: RulePosition,
    /// Parsed program retained by the terminal state.
    program: Program,
    /// Materialized return output produced by the committed return rule.
    output: ReturnOutput,
}

/// Runtime failure that preserves uncommitted state for inspection.
pub struct OwnedFailedRun {
    /// Runtime error that stopped the candidate step before commit.
    error: RunError,
    /// Uncommitted owned session retained for diagnostic inspection.
    session: OwnedRunSession,
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

impl core::fmt::Debug for RunSession<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RunSession")
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for OwnedRunSession {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedRunSession")
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for StepTransition<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Applied(applied) => formatter.debug_tuple("Applied").field(applied).finish(),
            Self::Stable(stable) => formatter.debug_tuple("Stable").field(stable).finish(),
            Self::Returned(returned) => formatter.debug_tuple("Returned").field(returned).finish(),
            Self::Failed(failed) => formatter.debug_tuple("Failed").field(failed).finish(),
        }
    }
}

impl core::fmt::Debug for OwnedStepTransition {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Applied(applied) => formatter.debug_tuple("Applied").field(applied).finish(),
            Self::Stable(stable) => formatter.debug_tuple("Stable").field(stable).finish(),
            Self::Returned(returned) => formatter.debug_tuple("Returned").field(returned).finish(),
            Self::Failed(failed) => formatter.debug_tuple("Failed").field(failed).finish(),
        }
    }
}

impl core::fmt::Debug for AppliedStep<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AppliedStep")
            .field("step", &self.step())
            .field("rule_position", &self.rule_position())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for OwnedAppliedStep {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedAppliedStep")
            .field("step", &self.step())
            .field("rule_position", &self.rule_position())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for StableRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("StableRun")
            .field("steps", &self.steps())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for OwnedStableRun {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedStableRun")
            .field("steps", &self.steps())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for ReturnedRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ReturnedRun")
            .field("step", &self.step())
            .field("rule_position", &self.rule_position())
            .field("output", &self.output())
            .finish()
    }
}

impl core::fmt::Debug for OwnedReturnedRun {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedReturnedRun")
            .field("step", &self.step())
            .field("rule_position", &self.rule_position())
            .field("output", &self.output())
            .finish()
    }
}

impl core::fmt::Debug for FailedRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("FailedRun")
            .field("error", &self.error())
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for OwnedFailedRun {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedFailedRun")
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
        let once_states = OnceStateSet::new(program.rule_slice())?;
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
                    return Ok(committed.into_result());
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
                    let step = committed.step();
                    let rule = committed.rule();
                    let output = committed.output_view();
                    Self::emit_step_trace(
                        &mut trace,
                        step,
                        rule,
                        BorrowedTraceEffect::Return { output },
                    )?;
                    return Ok(committed.into_result());
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
    fn into_result(self) -> RunResult {
        let step = self.step();
        RunResult::from_return(self.into_output(), step)
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

impl<'program> RunSession<'program> {
    /// Starts a new borrowed run session for a parsed program and admitted run
    /// seed.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if allocating per-run rule state fails.
    pub(crate) fn new(program: &'program Program, seed: RunSeed) -> Result<Self, RunError> {
        Ok(Self {
            session: Session::new(BorrowedProgram { program }, seed)?,
        })
    }

    /// Number of rewrite steps that have already completed in this run.
    ///
    /// Failed candidate steps are not counted because they never commit.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.session.completed_steps()
    }

    /// Borrow the parsed program used by this session.
    #[must_use]
    pub fn program(&self) -> &'program Program {
        self.session.program.program
    }

    /// Borrow the current runtime state.
    ///
    /// The returned view borrows only for this observation. Materializing it is
    /// an explicit allocation boundary.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Advances this run by exactly one matching rule when possible.
    ///
    /// Applying an ordinary rewrite returns [`StepTransition::Applied`] with a
    /// continuation session. No match, `(return)`, and runtime failure all
    /// consume the session into terminal typestates.
    #[must_use]
    pub fn step(mut self) -> StepTransition<'program> {
        match self.session.step_borrowed() {
            Ok(CoreStep::Applied(AppliedRule::Rewrite(committed))) => {
                let step = committed.step();
                let rule = committed.rule().position();
                StepTransition::Applied(AppliedStep {
                    step,
                    rule_position: rule,
                    session: self,
                })
            }
            Ok(CoreStep::Applied(AppliedRule::Return(committed))) => {
                let step = committed.step();
                let rule = committed.rule().position();
                let output = committed.into_output();
                let Session { program, core: _ } = self.session;
                StepTransition::Returned(ReturnedRun {
                    step,
                    rule_position: rule,
                    program: program.program,
                    output,
                })
            }
            Ok(CoreStep::Stable(steps)) => {
                let Session { program, core } = self.session;
                StepTransition::Stable(StableRun {
                    steps,
                    program: program.program,
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
                    return Ok(returned.into_result());
                }
                StepTransition::Failed(failed) => return Err(failed.into_error()),
            }
        }
    }
}

impl OwnedRunSession {
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

    /// Discards the current run state and recovers the owned parsed program.
    ///
    /// This intentionally drops the in-progress runtime state; it is for
    /// ownership recovery, not for retrying the same admitted run.
    #[must_use]
    pub fn into_program(self) -> Program {
        let (program, _core) = self.session.into_program_core();
        program
    }

    /// Borrow the current runtime state.
    ///
    /// The returned view borrows only for this observation. Materializing it is
    /// an explicit allocation boundary.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Advances this run by exactly one matching rule when possible.
    ///
    /// Applying an ordinary rewrite returns [`OwnedStepTransition::Applied`]
    /// with a continuation session. Owned terminal and failed states keep the
    /// parsed program recoverable.
    #[must_use]
    pub fn step(mut self) -> OwnedStepTransition {
        match self.session.step() {
            Ok(CoreStep::Applied(AppliedRule::Rewrite(committed))) => {
                let step = committed.step();
                let rule = committed.rule().position();
                OwnedStepTransition::Applied(OwnedAppliedStep {
                    step,
                    rule_position: rule,
                    session: self,
                })
            }
            Ok(CoreStep::Applied(AppliedRule::Return(committed))) => {
                let step = committed.step();
                let rule = committed.rule().position();
                let output = committed.into_output();
                let (program, _core) = self.session.into_program_core();
                OwnedStepTransition::Returned(OwnedReturnedRun {
                    step,
                    rule_position: rule,
                    program,
                    output,
                })
            }
            Ok(CoreStep::Stable(steps)) => {
                let (program, core) = self.session.into_program_core();
                OwnedStepTransition::Stable(OwnedStableRun {
                    steps,
                    program,
                    core,
                })
            }
            Err(error) => OwnedStepTransition::Failed(OwnedFailedRun::new(error, self)),
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
                OwnedStepTransition::Applied(applied) => {
                    self = applied.into_session();
                }
                OwnedStepTransition::Stable(stable) => {
                    return stable.into_result();
                }
                OwnedStepTransition::Returned(returned) => {
                    return Ok(returned.into_result());
                }
                OwnedStepTransition::Failed(failed) => return Err(failed.into_error()),
            }
        }
    }
}

impl<'program> AppliedStep<'program> {
    /// One-based applied step count.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Program-local position of the applied rule.
    #[must_use]
    pub const fn rule_position(&self) -> RulePosition {
        self.rule_position
    }

    /// Runtime state after the applied rewrite step.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Continue running after observing this applied step.
    ///
    /// This is the only borrowed transition that can resume execution.
    #[must_use]
    pub fn into_session(self) -> RunSession<'program> {
        self.session
    }
}

impl OwnedAppliedStep {
    /// One-based applied step count.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Program-local position of the applied rule.
    #[must_use]
    pub const fn rule_position(&self) -> RulePosition {
        self.rule_position
    }

    /// Runtime state after the applied rewrite step.
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
}

impl<'program> StableRun<'program> {
    /// Number of rewrite steps applied before reaching the stable state.
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
    /// Returns `RunError` if final state materialization cannot allocate.
    pub fn into_result(self) -> Result<RunResult, RunError> {
        self.core.into_stable_result(self.steps)
    }
}

impl OwnedStableRun {
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
    /// Returns `RunError` if final state materialization cannot allocate.
    pub fn into_result(self) -> Result<RunResult, RunError> {
        self.core.into_stable_result(self.steps)
    }
}

impl<'program> ReturnedRun<'program> {
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

    /// Program-local position of the return rule.
    #[must_use]
    pub const fn rule_position(&self) -> RulePosition {
        self.rule_position
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

    /// Discards the return output and recovers the owned parsed program.
    ///
    /// This drops the terminal `(return)` output. Use
    /// [`OwnedReturnedRun::into_result`] when the output bytes are the desired
    /// result.
    #[must_use]
    pub fn into_program(self) -> Program {
        self.program
    }

    /// Program-local position of the return rule.
    #[must_use]
    pub const fn rule_position(&self) -> RulePosition {
        self.rule_position
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

impl<'program> FailedRun<'program> {
    /// Captures a failed borrowed session without committing the attempted step.
    fn new(error: RunError, session: RunSession<'program>) -> Self {
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
    pub fn into_error(self) -> RunError {
        self.error
    }
}

impl core::fmt::Display for FailedRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl core::error::Error for FailedRun<'_> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl OwnedFailedRun {
    /// Captures a failed owned session without committing the attempted step.
    fn new(error: RunError, session: OwnedRunSession) -> Self {
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

    /// Discards the runtime error and recovers the uncommitted owned session.
    ///
    /// This preserves ownership for hosts that need to recover the parsed
    /// program, but the caller is also choosing to drop the structured error.
    #[must_use]
    pub fn into_session(self) -> OwnedRunSession {
        self.session
    }

    /// Splits this failed transition into its runtime error and uncommitted
    /// owned session.
    #[must_use]
    pub fn into_parts(self) -> (RunError, OwnedRunSession) {
        (self.error, self.session)
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
