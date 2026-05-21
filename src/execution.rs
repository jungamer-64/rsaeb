//! Public stepwise run typestates.
//!
//! [`Program::start_run`](crate::Program::start_run) returns a [`RunSession`].
//! Calling [`RunSession::step`] consumes that value and returns a
//! [`StepTransition`], so callers must handle the next state explicitly:
//! continue with [`AppliedStep::into_session`], finish a [`StableRun`], or
//! finish a [`ReturnedRun`].
//!
//! The mutable runtime engine lives under the private runtime module. These
//! public values are typed host-facing wrappers around that engine; they move
//! the existing session state without allocating snapshots or heap indirection.

use crate::error::RunError;
use crate::inspect::{PayloadView, RuleView};
use crate::program::{Program, RunLimits, RunResult, StepCount};
use crate::runtime::RuntimeInput;
use crate::runtime::session::{
    RuntimeAppliedStep, RuntimeReturnedRun, RuntimeSession, RuntimeStableRun, RuntimeStep,
    RuntimeStepError,
};
use crate::trace::RuntimeStateView;

/// Stateful run session that can still apply rules.
///
/// This type represents the only public state with a `step` method. Stable and
/// returned runs are represented by separate terminal types, so callers cannot
/// step after completion.
pub struct RunSession<'program> {
    pub(crate) session: RuntimeSession<'program>,
}

/// Result of advancing a run session once.
///
/// The transition is exhaustive over the public run lifecycle: one rule
/// committed and execution can continue, no rule matched, or a `(return)` rule
/// produced final output.
pub enum StepTransition<'program> {
    /// One ordinary rewrite rule was applied and execution can continue.
    Applied(AppliedStep<'program>),
    /// No rule matched the final runtime state.
    Stable(StableRun<'program>),
    /// A matched rule executed `(return)`.
    Returned(ReturnedRun<'program>),
}

/// One committed non-terminal rule application.
///
/// This value lets a caller inspect the applied rule and post-step state before
/// deciding whether to continue the run.
pub struct AppliedStep<'program> {
    step: RuntimeAppliedStep<'program>,
}

/// Terminal run state reached by no matching rule.
///
/// Stable runs still own the final runtime state until the caller either
/// borrows it or materializes it with [`StableRun::into_result`].
pub struct StableRun<'program> {
    run: RuntimeStableRun<'program>,
}

/// Terminal run state reached by `(return)`.
///
/// The output is a borrowed parsed payload until the caller materializes the
/// terminal [`RunResult`] through [`ReturnedRun::into_result`].
#[derive(Clone, Copy)]
pub struct ReturnedRun<'program> {
    run: RuntimeReturnedRun<'program>,
}

/// Runtime failure that preserves the uncommitted run session.
///
/// Step failures happen before the candidate rewrite is committed. The failed
/// [`RunSession`] is therefore returned by value so hosts can inspect, update
/// limits with [`RunSession::with_limits`], or discard it explicitly.
pub struct RunStepError<'program> {
    error: RunError,
    session: RunSession<'program>,
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

impl core::fmt::Debug for StepTransition<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Applied(applied) => formatter.debug_tuple("Applied").field(applied).finish(),
            Self::Stable(stable) => formatter.debug_tuple("Stable").field(stable).finish(),
            Self::Returned(returned) => formatter.debug_tuple("Returned").field(returned).finish(),
        }
    }
}

impl core::fmt::Debug for AppliedStep<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AppliedStep")
            .field("step", &self.step())
            .field("rule", &self.rule())
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

impl core::fmt::Debug for ReturnedRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ReturnedRun")
            .field("step", &self.step())
            .field("rule", &self.rule())
            .field("output", &self.output())
            .finish()
    }
}

impl core::fmt::Debug for RunStepError<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RunStepError")
            .field("error", &self.error())
            .field("session", &self.session())
            .finish()
    }
}

impl<'program> RunSession<'program> {
    /// Starts a new run session for a parsed program and validated input.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if runtime input materialization fails, state limits
    /// reject the input, or per-run rule state allocation fails.
    pub(crate) fn new(
        program: &'program Program,
        input: &RuntimeInput,
        limits: RunLimits,
    ) -> Result<Self, RunError> {
        Ok(Self {
            session: RuntimeSession::new(program, input, limits)?,
        })
    }

    /// Number of rewrite steps that have already completed in this run.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.session.completed_steps()
    }

    /// Borrow the current runtime state.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Replaces runtime limits for this uncommitted run.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if the already-completed step count or the current
    /// runtime state does not fit the replacement limits.
    pub fn with_limits(self, limits: RunLimits) -> Result<Self, RunError> {
        Ok(Self {
            session: self.session.with_limits(limits)?,
        })
    }

    /// Advances this run by exactly one matching rule when possible.
    ///
    /// Consuming `self` makes terminal states explicit. Call
    /// [`AppliedStep::into_session`] to continue after an applied rule.
    ///
    /// # Errors
    ///
    /// Returns `RunStepError` if the matching rule cannot commit because
    /// runtime limits or allocation fail. The error preserves the uncommitted
    /// run session without allocating on the error path.
    #[expect(
        clippy::result_large_err,
        reason = "RunStepError preserves the uncommitted run session by value without allocating on the error path"
    )]
    pub fn step(self) -> Result<StepTransition<'program>, RunStepError<'program>> {
        match self.session.step() {
            Ok(RuntimeStep::Applied(step)) => Ok(StepTransition::Applied(AppliedStep { step })),
            Ok(RuntimeStep::Stable(run)) => Ok(StepTransition::Stable(StableRun { run })),
            Ok(RuntimeStep::Returned(run)) => Ok(StepTransition::Returned(ReturnedRun { run })),
            Err(error) => Err(RunStepError::from_runtime(error)),
        }
    }

    /// Runs this session to completion.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when applying a later matching rule would exceed the
    /// configured limits, allocation fails, or state-size arithmetic overflows.
    pub fn finish(self) -> Result<RunResult, RunError> {
        self.session.finish()
    }
}

impl<'program> AppliedStep<'program> {
    /// One-based applied step count.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step.step()
    }

    /// Structured view of the applied rule.
    #[must_use]
    pub const fn rule(&self) -> RuleView<'program> {
        self.step.rule()
    }

    /// Runtime state after the applied rewrite step.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.step.state()
    }

    /// Continue running after observing this applied step.
    #[must_use]
    pub fn into_session(self) -> RunSession<'program> {
        RunSession {
            session: self.step.into_session(),
        }
    }
}

impl StableRun<'_> {
    /// Number of rewrite steps applied before reaching the stable state.
    #[must_use]
    pub const fn steps(&self) -> StepCount {
        self.run.steps()
    }

    /// Borrowed final runtime state.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.run.state()
    }

    /// Materializes this stable run as a run result.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if final state materialization cannot allocate.
    pub fn into_result(self) -> Result<RunResult, RunError> {
        self.run.into_result()
    }
}

impl<'program> ReturnedRun<'program> {
    /// One-based applied step count for the return rule.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.run.step()
    }

    /// Structured view of the return rule.
    #[must_use]
    pub const fn rule(&self) -> RuleView<'program> {
        self.run.rule()
    }

    /// Borrowed return payload from the parsed program.
    #[must_use]
    pub const fn output(&self) -> PayloadView<'program> {
        self.run.output()
    }

    /// Materializes this returned run as a run result.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if return output materialization cannot allocate.
    pub fn into_result(self) -> Result<RunResult, RunError> {
        self.run.into_result()
    }
}

impl<'program> RunStepError<'program> {
    fn from_runtime(error: RuntimeStepError<'program>) -> Self {
        let (error, session) = error.into_parts();
        Self {
            error,
            session: RunSession { session },
        }
    }

    /// Runtime error that prevented the step from committing.
    #[must_use]
    pub const fn error(&self) -> &RunError {
        &self.error
    }

    /// Borrow the uncommitted run session.
    #[must_use]
    pub const fn session(&self) -> &RunSession<'program> {
        &self.session
    }

    /// Recover the uncommitted run session.
    #[must_use]
    pub fn into_session(self) -> RunSession<'program> {
        self.session
    }

    /// Discard the uncommitted run session and return the runtime error.
    #[must_use]
    pub fn into_error(self) -> RunError {
        self.error
    }
}

impl core::fmt::Display for RunStepError<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl core::error::Error for RunStepError<'_> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.error)
    }
}
