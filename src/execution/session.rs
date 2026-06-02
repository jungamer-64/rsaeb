use crate::error::{RunError, RunFinishError, RunStartError, TracedRunError};
use crate::input::AdmittedRun;
use crate::limits::StepCount;
use crate::policy::{ExecutionPolicy, ParsePolicy};
use crate::program::{ExecutableProgram, ExecutableProgramRef, RunResult};
use crate::trace::{BorrowedTraceEvent, RuntimeStateView};

use super::advance::BorrowedRunWitness;
use super::engine::{BorrowedProgram, CoreRunTransition, Session};
use super::transition::{
    BorrowedAppliedStep, BorrowedFailedRun, BorrowedReturnedRun, BorrowedStableRun,
    BorrowedStepTransition,
};

/// Stateful run session that borrows a reusable parsed program.
///
/// This is the stepwise form returned by
/// [`ExecutableProgram::steps`](crate::program::ExecutableProgram::steps).
/// It consumes itself on every step so callers must handle the returned
/// [`BorrowedStepTransition`] before they can continue.
pub struct BorrowedRunSession<'program, P: ParsePolicy, E: ExecutionPolicy> {
    /// Internal session using the public borrowed program boundary.
    pub(super) session: Session<'program, P, E>,
}

/// Runs a borrowed program to completion.
///
/// # Errors
///
/// Returns `RunError` when execution setup fails or a later matching rule would
/// exceed configured limits.
pub(crate) fn finish_borrowed_run<P: ParsePolicy, E: ExecutionPolicy>(
    executable: ExecutableProgramRef<'_, P>,
    admitted: AdmittedRun<E>,
) -> Result<RunResult, RunError> {
    Session::new(
        BorrowedProgram {
            program: executable.program(),
        },
        admitted,
    )
    .map_err(RunError::from)?
    .finish()
    .map_err(RunError::from)
}

/// Runs a borrowed program to completion while emitting borrowed trace events.
///
/// # Errors
///
/// Returns `TracedRunError::Run` for runtime failures and
/// `TracedRunError::Trace` for user callback failures.
pub(crate) fn trace_events<'program, P, E, F, TraceError>(
    executable: ExecutableProgramRef<'program, P>,
    admitted: AdmittedRun<E>,
    trace: F,
) -> Result<RunResult, TracedRunError<TraceError>>
where
    P: ParsePolicy,
    E: ExecutionPolicy,
    F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), TraceError>,
{
    Session::new(
        BorrowedProgram {
            program: executable.program(),
        },
        admitted,
    )
    .map_err(RunError::from)
    .map_err(TracedRunError::Run)?
    .trace_events(trace)
}

impl<'program, P: ParsePolicy, E: ExecutionPolicy> BorrowedRunSession<'program, P, E> {
    /// Starts a new borrowed run session for a parsed program and admitted run
    /// witness.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule state fails.
    pub(crate) fn new(
        program: &'program ExecutableProgram<P>,
        admitted: AdmittedRun<E>,
    ) -> Result<Self, RunStartError> {
        Ok(Self {
            session: Session::new(BorrowedProgram { program }, admitted)?,
        })
    }

    /// Number of execution steps that have already completed in this run.
    ///
    /// Failed candidate steps are not counted because they never commit.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.session.completed_steps()
    }

    /// Borrow the parsed program used by this session.
    #[must_use]
    pub fn program(&self) -> &'program ExecutableProgram<P> {
        self.session.program()
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
    /// Applying an ordinary rewrite returns [`BorrowedStepTransition::Applied`] with a
    /// continuation session. No match, `(return)`, and runtime failure all
    /// consume the session into terminal typestates.
    #[must_use]
    pub fn step(self) -> BorrowedStepTransition<'program, P, E> {
        step_borrowed_run(self)
    }

    /// Runs this session to completion.
    ///
    /// # Errors
    ///
    /// Returns `RunFinishError` when applying a later matching rule would exceed the
    /// configured limits, allocation fails, or state-size arithmetic overflows.
    pub fn finish(self) -> Result<RunResult, RunFinishError> {
        self.session.finish()
    }
}

/// Advances a borrowed ordinary run and projects the private transition into the public type.
fn step_borrowed_run<'program, P: ParsePolicy, E: ExecutionPolicy>(
    session: BorrowedRunSession<'program, P, E>,
) -> BorrowedStepTransition<'program, P, E> {
    match session.session.advance_run_step::<BorrowedRunWitness>() {
        CoreRunTransition::Applied {
            step,
            rule,
            continuation,
        } => BorrowedStepTransition::Applied(BorrowedAppliedStep {
            step,
            rule,
            session: BorrowedRunSession {
                session: continuation,
            },
        }),
        CoreRunTransition::Returned {
            step,
            rule,
            output_view: _,
            output,
            terminal,
        } => BorrowedStepTransition::Returned(BorrowedReturnedRun {
            step,
            rule,
            program: terminal.program.program,
            output,
        }),
        CoreRunTransition::Stable { terminal } => {
            BorrowedStepTransition::Stable(BorrowedStableRun {
                program: terminal.program.program,
                core: terminal.core,
            })
        }
        CoreRunTransition::Failed { error, terminal } => BorrowedStepTransition::Failed(
            BorrowedFailedRun::new(error, terminal.program.program, terminal.core),
        ),
    }
}
