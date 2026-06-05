use crate::error::{RunError, RunFinishError, RunStartError, TracedRunError};
use crate::input::AdmittedRun;
use crate::limits::{RuleAttemptCount, StepCount};
use crate::policy::{ExecutionPolicy, RuleAttemptPolicy};
use crate::program::{ExecutableProgram, RunResult};
use crate::trace::{BorrowedTraceEvent, RuntimeStateView};

use super::advance::{
    CoreContinuingRuleAttemptStep, CoreFinalRuleAttemptStep,
    advance_continuing_borrowed_rule_attempt, advance_final_borrowed_rule_attempt,
};
use super::engine::{
    AttemptSessionCursor, ContinuingAttemptSession, CoreRunTransition, FinalAttemptSession,
    Session, TerminalAttemptSession, TerminalRunCore,
};
use super::transition::{
    BorrowedAlwaysReturnRun, BorrowedAlwaysRewriteStep, BorrowedContinuingRuleAttemptTransition,
    BorrowedFailedRun, BorrowedFinalRuleAttemptTransition, BorrowedMissedRuleAttempt,
    BorrowedOnceReturnRun, BorrowedOnceRewriteStep, BorrowedRuleAttemptAlwaysReturnRun,
    BorrowedRuleAttemptAlwaysRewriteStep, BorrowedRuleAttemptFailedRun,
    BorrowedRuleAttemptOnceReturnRun, BorrowedRuleAttemptOnceRewriteStep,
    BorrowedRuleAttemptStableRun, BorrowedStableRun, BorrowedStepTransition,
};

/// Stateful run session that borrows a reusable parsed program.
///
/// This is the stepwise form returned by
/// [`ExecutableProgram::steps`](crate::program::ExecutableProgram::steps).
/// It consumes itself on every step so callers must handle the returned
/// [`BorrowedStepTransition`] before they can continue.
pub struct BorrowedRunSession<'program, E: ExecutionPolicy> {
    /// Internal session using the public borrowed program boundary.
    pub(super) session: Session<'program, E>,
}

/// Borrowed rule-attempt cursor classified by the current pass shape.
///
/// This cursor is observable but not directly stepable. Callers must match the
/// cursor and then advance the continuing or final session, so stable and
/// missed-continuation outcomes stay separated by type.
pub enum BorrowedRuleAttemptCursor<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// Current rule has at least one successor in this pass.
    Continuing(BorrowedContinuingRuleAttemptSession<'program, E, A>),
    /// Current rule exhausts this pass.
    Final(BorrowedFinalRuleAttemptSession<'program, E, A>),
}

/// Borrowed rule-attempt session whose current target has a successor.
pub struct BorrowedContinuingRuleAttemptSession<'program, E: ExecutionPolicy, A: RuleAttemptPolicy>
{
    /// Internal rule-attempt session pinned to a continuing pass shape.
    pub(super) session: ContinuingAttemptSession<'program, E, A>,
}

/// Borrowed rule-attempt session whose current target exhausts the pass.
pub struct BorrowedFinalRuleAttemptSession<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// Internal rule-attempt session pinned to a final pass shape.
    pub(super) session: FinalAttemptSession<'program, E, A>,
}

/// Terminal data split out of a borrowed rule-attempt run session.
struct BorrowedRuleAttemptTerminal<'program> {
    /// Parsed program borrowed by the terminal state.
    program: &'program ExecutableProgram,
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
pub(crate) fn finish_borrowed_run<E: ExecutionPolicy>(
    executable: &ExecutableProgram,
    admitted: AdmittedRun<E>,
) -> Result<RunResult, RunError> {
    Session::new(executable, admitted)
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
pub(crate) fn trace_events<'program, E, F, TraceError>(
    executable: &'program ExecutableProgram,
    admitted: AdmittedRun<E>,
    trace: F,
) -> Result<RunResult, TracedRunError<TraceError>>
where
    E: ExecutionPolicy,
    F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), TraceError>,
{
    Session::new(executable, admitted)
        .map_err(RunError::from)
        .map_err(TracedRunError::Run)?
        .trace_events(trace)
}

impl<'program, E: ExecutionPolicy> BorrowedRunSession<'program, E> {
    /// Starts a new borrowed run session for a parsed program and admitted run
    /// witness.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule state fails.
    pub(crate) fn new(
        program: &'program ExecutableProgram,
        admitted: AdmittedRun<E>,
    ) -> Result<Self, RunStartError> {
        Ok(Self {
            session: Session::new(program, admitted)?,
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
    pub fn program(&self) -> &'program ExecutableProgram {
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
    /// Applying a rewrite returns an exact rewritten transition with a continuation
    /// session. No match, `(return)`, and runtime failure all consume the session
    /// into terminal typestates.
    #[must_use]
    pub fn step(self) -> BorrowedStepTransition<'program, E> {
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

impl<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> BorrowedRuleAttemptCursor<'program, E, A> {
    /// Starts borrowed rule-attempt execution from an executable program witness.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule-attempt state fails.
    pub(crate) fn new(
        program: &'program ExecutableProgram,
        admitted: AdmittedRun<E>,
    ) -> Result<Self, RunStartError> {
        AttemptSessionCursor::new(program, admitted).map(Self::from_cursor)
    }

    /// Projects the private session classifier into the public cursor.
    pub(super) fn from_cursor(cursor: AttemptSessionCursor<'program, E, A>) -> Self {
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
    pub fn program(&self) -> &'program ExecutableProgram {
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

impl<'program, E: ExecutionPolicy, A: RuleAttemptPolicy>
    BorrowedContinuingRuleAttemptSession<'program, E, A>
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
    pub const fn program(&self) -> &'program ExecutableProgram {
        match &self.session {
            ContinuingAttemptSession::First(session) => session.program,
            ContinuingAttemptSession::AfterMiss(session) => session.program,
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
    pub fn step(self) -> BorrowedContinuingRuleAttemptTransition<'program, E, A> {
        project_continuing_rule_attempt_step(advance_continuing_borrowed_rule_attempt(self.session))
    }
}

impl<'program, E: ExecutionPolicy, A: RuleAttemptPolicy>
    BorrowedFinalRuleAttemptSession<'program, E, A>
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
    pub const fn program(&self) -> &'program ExecutableProgram {
        match &self.session {
            FinalAttemptSession::First(session) => session.program,
            FinalAttemptSession::AfterMiss(session) => session.program,
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
    pub fn step(self) -> BorrowedFinalRuleAttemptTransition<'program, E, A> {
        project_final_rule_attempt_step(advance_final_borrowed_rule_attempt(self.session))
    }
}

impl<'program> BorrowedRuleAttemptTerminal<'program> {
    /// Projects terminal borrowed rule-attempt state into public terminal data.
    fn from_terminal(terminal: TerminalAttemptSession<'program>) -> Self {
        let TerminalAttemptSession {
            program,
            core,
            attempts,
        } = terminal;
        Self {
            program,
            core,
            attempts,
        }
    }
}

/// Advances a borrowed ordinary run and projects the private transition into the public type.
fn step_borrowed_run<'program, E: ExecutionPolicy>(
    session: BorrowedRunSession<'program, E>,
) -> BorrowedStepTransition<'program, E> {
    match session.session.advance_run_step() {
        CoreRunTransition::AlwaysRewritten {
            step,
            rule,
            continuation,
        } => BorrowedStepTransition::AlwaysRewritten(BorrowedAlwaysRewriteStep {
            step,
            rule,
            session: BorrowedRunSession {
                session: continuation,
            },
        }),
        CoreRunTransition::OnceRewritten {
            step,
            rule,
            continuation,
        } => BorrowedStepTransition::OnceRewritten(BorrowedOnceRewriteStep {
            step,
            rule,
            session: BorrowedRunSession {
                session: continuation,
            },
        }),
        CoreRunTransition::AlwaysReturned {
            step,
            rule,
            output_view: _,
            output,
            terminal,
        } => BorrowedStepTransition::AlwaysReturned(BorrowedAlwaysReturnRun {
            step,
            rule,
            program: terminal.program,
            output,
        }),
        CoreRunTransition::OnceReturned {
            step,
            rule,
            output_view: _,
            output,
            terminal,
        } => BorrowedStepTransition::OnceReturned(BorrowedOnceReturnRun {
            step,
            rule,
            program: terminal.program,
            output,
        }),
        CoreRunTransition::Stable { terminal } => {
            BorrowedStepTransition::Stable(BorrowedStableRun {
                program: terminal.program,
                core: terminal.core,
            })
        }
        CoreRunTransition::Failed { error, terminal } => BorrowedStepTransition::Failed(
            BorrowedFailedRun::new(error, terminal.program, terminal.core),
        ),
    }
}

/// Projects one private continuing rule-attempt transition into the public transition type.
fn project_continuing_rule_attempt_step<'program, E: ExecutionPolicy, A: RuleAttemptPolicy>(
    step: CoreContinuingRuleAttemptStep<'program, E, A>,
) -> BorrowedContinuingRuleAttemptTransition<'program, E, A> {
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
        CoreContinuingRuleAttemptStep::AlwaysRewritten {
            attempt,
            step,
            rule,
            continuation,
        } => BorrowedContinuingRuleAttemptTransition::AlwaysRewritten(
            BorrowedRuleAttemptAlwaysRewriteStep {
                attempt,
                step,
                rule,
                cursor: BorrowedRuleAttemptCursor::from_cursor(continuation),
            },
        ),
        CoreContinuingRuleAttemptStep::OnceRewritten {
            attempt,
            step,
            rule,
            continuation,
        } => BorrowedContinuingRuleAttemptTransition::OnceRewritten(
            BorrowedRuleAttemptOnceRewriteStep {
                attempt,
                step,
                rule,
                cursor: BorrowedRuleAttemptCursor::from_cursor(continuation),
            },
        ),
        CoreContinuingRuleAttemptStep::AlwaysReturned {
            attempt,
            step,
            rule,
            output,
            terminal,
        } => {
            let terminal = BorrowedRuleAttemptTerminal::from_terminal(terminal);
            BorrowedContinuingRuleAttemptTransition::AlwaysReturned(
                BorrowedRuleAttemptAlwaysReturnRun {
                    attempt,
                    step,
                    rule,
                    program: terminal.program,
                    output,
                },
            )
        }
        CoreContinuingRuleAttemptStep::OnceReturned {
            attempt,
            step,
            rule,
            output,
            terminal,
        } => {
            let terminal = BorrowedRuleAttemptTerminal::from_terminal(terminal);
            BorrowedContinuingRuleAttemptTransition::OnceReturned(
                BorrowedRuleAttemptOnceReturnRun {
                    attempt,
                    step,
                    rule,
                    program: terminal.program,
                    output,
                },
            )
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
fn project_final_rule_attempt_step<'program, E: ExecutionPolicy, A: RuleAttemptPolicy>(
    step: CoreFinalRuleAttemptStep<'program, E, A>,
) -> BorrowedFinalRuleAttemptTransition<'program, E, A> {
    match step {
        CoreFinalRuleAttemptStep::AlwaysRewritten {
            attempt,
            step,
            rule,
            continuation,
        } => BorrowedFinalRuleAttemptTransition::AlwaysRewritten(
            BorrowedRuleAttemptAlwaysRewriteStep {
                attempt,
                step,
                rule,
                cursor: BorrowedRuleAttemptCursor::from_cursor(continuation),
            },
        ),
        CoreFinalRuleAttemptStep::OnceRewritten {
            attempt,
            step,
            rule,
            continuation,
        } => {
            BorrowedFinalRuleAttemptTransition::OnceRewritten(BorrowedRuleAttemptOnceRewriteStep {
                attempt,
                step,
                rule,
                cursor: BorrowedRuleAttemptCursor::from_cursor(continuation),
            })
        }
        CoreFinalRuleAttemptStep::AlwaysReturned {
            attempt,
            step,
            rule,
            output,
            terminal,
        } => {
            let terminal = BorrowedRuleAttemptTerminal::from_terminal(terminal);
            BorrowedFinalRuleAttemptTransition::AlwaysReturned(BorrowedRuleAttemptAlwaysReturnRun {
                attempt,
                step,
                rule,
                program: terminal.program,
                output,
            })
        }
        CoreFinalRuleAttemptStep::OnceReturned {
            attempt,
            step,
            rule,
            output,
            terminal,
        } => {
            let terminal = BorrowedRuleAttemptTerminal::from_terminal(terminal);
            BorrowedFinalRuleAttemptTransition::OnceReturned(BorrowedRuleAttemptOnceReturnRun {
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
