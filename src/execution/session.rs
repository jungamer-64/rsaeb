use crate::error::{RuleAttemptStepError, RunError, RunFinishError, RunStartError, TracedRunError};
use crate::input::AdmittedRun;
use crate::inspect::RuleView;
use crate::limits::{RuleAttemptCount, StepCount};
use crate::policy::{ExecutionPolicy, ParsePolicy, RuleAttemptPolicy};
use crate::program::{ExecutableProgram, ExecutableProgramRef, RunResult};
use crate::trace::{BorrowedTraceEvent, RuntimeStateView};

use super::advance::{
    BorrowedRunWitness, CoreContinuingRuleAttemptStep, CoreFinalRuleAttemptStep,
    advance_continuing_borrowed_rule_attempt, advance_final_borrowed_rule_attempt,
};
use super::engine::{
    AttemptSessionCursor, BorrowedProgram, ContinuingAttemptSession, CoreRunTransition,
    FinalAttemptSession, Session, TerminalAttemptSession, TerminalRunCore,
};
use super::transition::{
    BorrowedAppliedStep, BorrowedContinuingRuleAttemptTransition, BorrowedFailedRun,
    BorrowedFinalRuleAttemptTransition, BorrowedMissedRuleAttempt, BorrowedReturnedRun,
    BorrowedRuleAttemptAppliedStep, BorrowedRuleAttemptFailedRun, BorrowedRuleAttemptReturnedRun,
    BorrowedRuleAttemptStableRun, BorrowedStableRun, BorrowedStepTransition,
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

/// Borrowed rule-attempt cursor classified by the current pass shape.
///
/// This cursor is observable but not directly stepable. Callers must match the
/// cursor and then advance the continuing or final session, so stable and
/// missed-continuation outcomes stay separated by type.
pub enum BorrowedRuleAttemptCursor<
    'program,
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
> {
    /// Current rule has at least one successor in this pass.
    Continuing(BorrowedContinuingRuleAttemptSession<'program, P, E, A>),
    /// Current rule exhausts this pass.
    Final(BorrowedFinalRuleAttemptSession<'program, P, E, A>),
}

/// Borrowed rule-attempt session whose current target has a successor.
pub struct BorrowedContinuingRuleAttemptSession<
    'program,
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
> {
    /// Internal rule-attempt session pinned to a continuing pass shape.
    pub(super) session: ContinuingAttemptSession<'program, P, E, A>,
}

/// Borrowed rule-attempt session whose current target exhausts the pass.
pub struct BorrowedFinalRuleAttemptSession<
    'program,
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
> {
    /// Internal rule-attempt session pinned to a final pass shape.
    pub(super) session: FinalAttemptSession<'program, P, E, A>,
}

/// Terminal data split out of a borrowed rule-attempt run session.
struct BorrowedRuleAttemptTerminal<'program, P: ParsePolicy> {
    /// Parsed program borrowed by the terminal state.
    program: &'program ExecutableProgram<P>,
    /// Runtime core retained for terminal state observation or materialization.
    core: TerminalRunCore,
    /// Rule attempts consumed before the terminal boundary was reached.
    attempts: RuleAttemptCount,
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

impl<'program, P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy>
    BorrowedRuleAttemptCursor<'program, P, E, A>
{
    /// Starts borrowed rule-attempt execution from an executable program witness.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule-attempt state fails.
    pub(crate) fn new(
        program: &'program ExecutableProgram<P>,
        admitted: AdmittedRun<E>,
    ) -> Result<Self, RunStartError> {
        AttemptSessionCursor::new(BorrowedProgram { program }, admitted).map(Self::from_cursor)
    }

    /// Projects the private session classifier into the public cursor.
    pub(super) fn from_cursor(cursor: AttemptSessionCursor<'program, P, E, A>) -> Self {
        match cursor {
            AttemptSessionCursor::Continuing(session) => {
                Self::Continuing(BorrowedContinuingRuleAttemptSession { session })
            }
            AttemptSessionCursor::Final(session) => {
                Self::Final(BorrowedFinalRuleAttemptSession { session })
            }
        }
    }

    /// Number of execution steps that have already completed in this run.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        match self {
            Self::Continuing(session) => session.completed_steps(),
            Self::Final(session) => session.completed_steps(),
        }
    }

    /// Number of executable rule-line attempts consumed so far.
    #[must_use]
    pub const fn completed_attempts(&self) -> RuleAttemptCount {
        match self {
            Self::Continuing(session) => session.completed_attempts(),
            Self::Final(session) => session.completed_attempts(),
        }
    }

    /// Borrow the parsed program used by this cursor.
    #[must_use]
    pub fn program(&self) -> &'program ExecutableProgram<P> {
        match self {
            Self::Continuing(session) => session.program(),
            Self::Final(session) => session.program(),
        }
    }

    /// Borrow the current runtime state.
    ///
    /// The returned view borrows only for this observation. Materializing it is
    /// an explicit allocation boundary.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        match self {
            Self::Continuing(session) => session.state(),
            Self::Final(session) => session.state(),
        }
    }
}

impl<'program, P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy>
    BorrowedContinuingRuleAttemptSession<'program, P, E, A>
{
    /// Number of execution steps that have already completed in this run.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        match &self.session {
            ContinuingAttemptSession::First(session) => session.completed_steps(),
            ContinuingAttemptSession::AfterMiss(session) => session.completed_steps(),
        }
    }

    /// Number of executable rule-line attempts consumed so far.
    #[must_use]
    pub const fn completed_attempts(&self) -> RuleAttemptCount {
        match &self.session {
            ContinuingAttemptSession::First(session) => session.completed_attempts(),
            ContinuingAttemptSession::AfterMiss(session) => session.completed_attempts(),
        }
    }

    /// Borrow the parsed program used by this session.
    #[must_use]
    pub const fn program(&self) -> &'program ExecutableProgram<P> {
        match &self.session {
            ContinuingAttemptSession::First(session) => session.program.program,
            ContinuingAttemptSession::AfterMiss(session) => session.program.program,
        }
    }

    /// Borrow the current runtime state.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        match &self.session {
            ContinuingAttemptSession::First(session) => session.state(),
            ContinuingAttemptSession::AfterMiss(session) => session.state(),
        }
    }

    /// Advances a continuing rule-attempt session by exactly one executable rule line.
    #[must_use]
    pub fn step(self) -> BorrowedContinuingRuleAttemptTransition<'program, P, E, A> {
        project_continuing_rule_attempt_step(advance_continuing_borrowed_rule_attempt(self.session))
    }
}

impl<'program, P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy>
    BorrowedFinalRuleAttemptSession<'program, P, E, A>
{
    /// Number of execution steps that have already completed in this run.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        match &self.session {
            FinalAttemptSession::First(session) => session.completed_steps(),
            FinalAttemptSession::AfterMiss(session) => session.completed_steps(),
        }
    }

    /// Number of executable rule-line attempts consumed so far.
    #[must_use]
    pub const fn completed_attempts(&self) -> RuleAttemptCount {
        match &self.session {
            FinalAttemptSession::First(session) => session.completed_attempts(),
            FinalAttemptSession::AfterMiss(session) => session.completed_attempts(),
        }
    }

    /// Borrow the parsed program used by this session.
    #[must_use]
    pub const fn program(&self) -> &'program ExecutableProgram<P> {
        match &self.session {
            FinalAttemptSession::First(session) => session.program.program,
            FinalAttemptSession::AfterMiss(session) => session.program.program,
        }
    }

    /// Borrow the current runtime state.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        match &self.session {
            FinalAttemptSession::First(session) => session.state(),
            FinalAttemptSession::AfterMiss(session) => session.state(),
        }
    }

    /// Advances a final rule-attempt session by exactly one executable rule line.
    #[must_use]
    pub fn step(self) -> BorrowedFinalRuleAttemptTransition<'program, P, E, A> {
        project_final_rule_attempt_step(advance_final_borrowed_rule_attempt(self.session))
    }
}

impl<'program, P: ParsePolicy> BorrowedRuleAttemptTerminal<'program, P> {
    /// Projects terminal borrowed rule-attempt state into public terminal data.
    fn from_terminal(terminal: TerminalAttemptSession<'program, P>) -> Self {
        let TerminalAttemptSession {
            program,
            core,
            attempts,
        } = terminal;
        Self {
            program: program.program,
            core,
            attempts,
        }
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

/// Projects one private continuing rule-attempt transition into the public transition type.
fn project_continuing_rule_attempt_step<
    'program,
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
>(
    step: CoreContinuingRuleAttemptStep<
        'program,
        P,
        E,
        A,
        RuleView<'program>,
        RuleAttemptStepError,
    >,
) -> BorrowedContinuingRuleAttemptTransition<'program, P, E, A> {
    match step {
        CoreContinuingRuleAttemptStep::Missed {
            attempt,
            miss,
            continuation,
        } => BorrowedContinuingRuleAttemptTransition::Missed(BorrowedMissedRuleAttempt {
            attempt,
            miss,
            cursor: BorrowedRuleAttemptCursor::from_cursor(continuation),
        }),
        CoreContinuingRuleAttemptStep::Applied {
            attempt,
            step,
            rule,
            continuation,
        } => BorrowedContinuingRuleAttemptTransition::Applied(BorrowedRuleAttemptAppliedStep {
            attempt,
            step,
            rule,
            cursor: BorrowedRuleAttemptCursor::from_cursor(continuation),
        }),
        CoreContinuingRuleAttemptStep::Returned {
            attempt,
            step,
            rule,
            output,
            terminal,
        } => {
            let terminal = BorrowedRuleAttemptTerminal::from_terminal(terminal);
            BorrowedContinuingRuleAttemptTransition::Returned(BorrowedRuleAttemptReturnedRun {
                attempt,
                step,
                rule,
                program: terminal.program,
                output,
            })
        }
        CoreContinuingRuleAttemptStep::Failed { error, terminal } => {
            let terminal = BorrowedRuleAttemptTerminal::from_terminal(terminal);
            BorrowedContinuingRuleAttemptTransition::Failed(BorrowedRuleAttemptFailedRun::new(
                error,
                terminal.attempts,
                terminal.program,
                terminal.core,
            ))
        }
    }
}

/// Projects one private final rule-attempt transition into the public transition type.
fn project_final_rule_attempt_step<
    'program,
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
>(
    step: CoreFinalRuleAttemptStep<'program, P, E, A, RuleView<'program>, RuleAttemptStepError>,
) -> BorrowedFinalRuleAttemptTransition<'program, P, E, A> {
    match step {
        CoreFinalRuleAttemptStep::Applied {
            attempt,
            step,
            rule,
            continuation,
        } => BorrowedFinalRuleAttemptTransition::Applied(BorrowedRuleAttemptAppliedStep {
            attempt,
            step,
            rule,
            cursor: BorrowedRuleAttemptCursor::from_cursor(continuation),
        }),
        CoreFinalRuleAttemptStep::Returned {
            attempt,
            step,
            rule,
            output,
            terminal,
        } => {
            let terminal = BorrowedRuleAttemptTerminal::from_terminal(terminal);
            BorrowedFinalRuleAttemptTransition::Returned(BorrowedRuleAttemptReturnedRun {
                attempt,
                step,
                rule,
                program: terminal.program,
                output,
            })
        }
        CoreFinalRuleAttemptStep::Stable {
            attempts,
            final_miss,
            terminal,
        } => {
            let terminal = BorrowedRuleAttemptTerminal::from_terminal(terminal);
            BorrowedFinalRuleAttemptTransition::Stable(BorrowedRuleAttemptStableRun {
                attempts,
                final_miss,
                program: terminal.program,
                core: terminal.core,
            })
        }
        CoreFinalRuleAttemptStep::Failed { error, terminal } => {
            let terminal = BorrowedRuleAttemptTerminal::from_terminal(terminal);
            BorrowedFinalRuleAttemptTransition::Failed(BorrowedRuleAttemptFailedRun::new(
                error,
                terminal.attempts,
                terminal.program,
                terminal.core,
            ))
        }
    }
}
