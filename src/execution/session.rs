use crate::error::{RunError, RunFinishError, RunStartError, TracedRunError};
use crate::input::AdmittedRun;
use crate::limits::{RuleAttemptCount, StepCount};
use crate::policy::{ExecutionPolicy, ParsePolicy, RuleAttemptPolicy};
use crate::program::{ExecutableProgram, ExecutableProgramRef, RunResult};
use crate::trace::{BorrowedTraceEvent, RuntimeStateView};

use super::advance::{BorrowedRunWitness, CoreRuleAttemptStep, advance_borrowed_rule_attempt};
use super::engine::{
    AttemptSession, BorrowedProgram, CoreRunTransition, Session, TerminalAttemptSession,
    TerminalRunCore,
};
use super::transition::{
    BorrowedAppliedStep, BorrowedFailedRun, BorrowedMissedRuleAttempt, BorrowedReturnedRun,
    BorrowedRuleAttemptAppliedStep, BorrowedRuleAttemptFailedRun, BorrowedRuleAttemptReturnedRun,
    BorrowedRuleAttemptStableRun, BorrowedRuleAttemptTransition, BorrowedStableRun,
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

/// Stateful run session that borrows a reusable parsed program and advances by rule attempt.
///
/// A rule-attempt step consumes one executable rule line even when that rule
/// does not apply. Committed non-terminal rule applications reset the rule
/// cursor to the first executable rule.
pub struct BorrowedRuleAttemptSession<
    'program,
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
> {
    /// Internal rule-attempt session using the public borrowed program boundary.
    pub(super) session: AttemptSession<'program, P, E, A>,
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
    BorrowedRuleAttemptSession<'program, P, E, A>
{
    /// Starts borrowed rule-attempt execution from an executable program witness.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule state fails.
    pub(crate) fn new(
        program: &'program ExecutableProgram<P>,
        admitted: AdmittedRun<E>,
    ) -> Result<Self, RunStartError> {
        Ok(Self {
            session: AttemptSession::new(BorrowedProgram { program }, admitted)?,
        })
    }

    /// Builds a public active rule-attempt session from the internal session.
    const fn from_active(session: AttemptSession<'program, P, E, A>) -> Self {
        Self { session }
    }

    /// Number of execution steps that have already completed in this run.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.session.completed_steps()
    }

    /// Number of executable rule-line attempts consumed so far.
    #[must_use]
    pub const fn completed_attempts(&self) -> RuleAttemptCount {
        self.session.completed_attempts()
    }

    /// Borrow the parsed program used by this session.
    #[must_use]
    pub fn program(&self) -> &'program ExecutableProgram<P> {
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

    /// Advances this run by exactly one executable rule line when possible.
    ///
    /// Non-matching rules return [`BorrowedRuleAttemptTransition::Missed`] with a
    /// continuation session. Matching rewrites return
    /// [`BorrowedRuleAttemptTransition::Applied`] and reset the next attempt to the
    /// first executable rule. No match across the whole pass, `(return)`, and
    /// runtime failure consume the session into terminal typestates.
    #[must_use]
    pub fn step(self) -> BorrowedRuleAttemptTransition<'program, P, E, A> {
        step_borrowed_rule_attempt_run(self)
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

/// Advances a borrowed rule-attempt run and projects the private transition into the public type.
fn step_borrowed_rule_attempt_run<
    'program,
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
>(
    session: BorrowedRuleAttemptSession<'program, P, E, A>,
) -> BorrowedRuleAttemptTransition<'program, P, E, A> {
    match advance_borrowed_rule_attempt(session.session) {
        CoreRuleAttemptStep::Missed {
            attempt,
            miss,
            continuation,
        } => BorrowedRuleAttemptTransition::Missed(BorrowedMissedRuleAttempt {
            attempt,
            miss,
            session: BorrowedRuleAttemptSession::from_active(continuation),
        }),
        CoreRuleAttemptStep::Applied {
            attempt,
            step,
            rule,
            continuation,
        } => BorrowedRuleAttemptTransition::Applied(BorrowedRuleAttemptAppliedStep {
            attempt,
            step,
            rule,
            session: BorrowedRuleAttemptSession::from_active(continuation),
        }),
        CoreRuleAttemptStep::Returned {
            attempt,
            step,
            rule,
            output,
            terminal,
        } => {
            let terminal = BorrowedRuleAttemptTerminal::from_terminal(terminal);
            BorrowedRuleAttemptTransition::Returned(BorrowedRuleAttemptReturnedRun {
                attempt,
                step,
                rule,
                program: terminal.program,
                output,
            })
        }
        CoreRuleAttemptStep::Stable {
            attempts,
            final_miss,
            terminal,
        } => {
            let terminal = BorrowedRuleAttemptTerminal::from_terminal(terminal);
            BorrowedRuleAttemptTransition::Stable(BorrowedRuleAttemptStableRun {
                attempts,
                final_miss,
                program: terminal.program,
                core: terminal.core,
            })
        }
        CoreRuleAttemptStep::Failed { error, terminal } => {
            let terminal = BorrowedRuleAttemptTerminal::from_terminal(terminal);
            BorrowedRuleAttemptTransition::Failed(BorrowedRuleAttemptFailedRun::new(
                error,
                terminal.attempts,
                terminal.program,
                terminal.core,
            ))
        }
    }
}
