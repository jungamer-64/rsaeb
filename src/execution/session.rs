use crate::error::{RunError, RunFinishError, RunStartError, TracedRunError};
use crate::input::AdmittedRun;
use crate::limits::{RuleAttemptCount, StepCount};
use crate::policy::{ExecutionPolicy, RuleAttemptPolicy};
use crate::program::{ExecutableProgram, RunResult};
use crate::runtime::once::{
    AfterMissContinuingRulePass, AfterMissFinalRulePass, FirstContinuingRulePass,
    FirstFinalRulePass, FirstRuntimeRulePassCursor, MissedRuntimeRulePassCursor,
    RuntimeRulePassCursor, StartedRuntimeRuleTable,
};
use crate::trace::{BorrowedTraceEvent, RuntimeStateView};

use super::advance::{
    advance_continuing_borrowed_rule_attempt, advance_final_borrowed_rule_attempt,
};
use super::engine::{
    AttemptRunCoreParts, AttemptSession, RunAdvance, RunReturn, RunRewrite, Session,
};
use super::transition::{
    BorrowedAlwaysReturnRun, BorrowedAlwaysRewriteStep,
    BorrowedContinuingRuleAttemptTransition, BorrowedFailedRun, BorrowedFinalRuleAttemptTransition,
    BorrowedOnceReturnRun, BorrowedOnceRewriteStep, BorrowedStableRun, BorrowedStepTransition,
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
    session: ContinuingRuleAttemptSession<'program, E, A>,
}

/// Borrowed rule-attempt session whose current target exhausts the pass.
pub struct BorrowedFinalRuleAttemptSession<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// Internal rule-attempt session pinned to a final pass shape.
    session: FinalRuleAttemptSession<'program, E, A>,
}

/// Continuing borrowed rule-attempt session classified by miss history.
enum ContinuingRuleAttemptSession<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// Continuing pass that has not missed any earlier rule in this scan.
    First(AttemptSession<'program, E, A, FirstContinuingRulePass<'program>>),
    /// Continuing pass after at least one miss.
    AfterMiss(AttemptSession<'program, E, A, AfterMissContinuingRulePass<'program>>),
}

/// Final borrowed rule-attempt session classified by miss history.
enum FinalRuleAttemptSession<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// Final pass that has not missed any earlier rule in this scan.
    First(AttemptSession<'program, E, A, FirstFinalRulePass<'program>>),
    /// Final pass after at least one miss.
    AfterMiss(AttemptSession<'program, E, A, AfterMissFinalRulePass<'program>>),
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
        let runtime_rules = StartedRuntimeRuleTable::from_program(program)?;
        Ok(Self::from_first_pass_cursor(
            program,
            admitted,
            runtime_rules.into_pass_cursor(),
        ))
    }

    /// Projects a newly started typed pass into the public cursor.
    fn from_first_pass_cursor(
        program: &'program ExecutableProgram,
        admitted: AdmittedRun<E>,
        cursor: FirstRuntimeRulePassCursor<'program>,
    ) -> Self {
        match cursor {
            RuntimeRulePassCursor::Continuing(pass) => {
                Self::Continuing(BorrowedContinuingRuleAttemptSession {
                    session: ContinuingRuleAttemptSession::First(AttemptSession::from_pass(
                        program, admitted, pass,
                    )),
                })
            }
            RuntimeRulePassCursor::Final(pass) => Self::Final(BorrowedFinalRuleAttemptSession {
                session: FinalRuleAttemptSession::First(AttemptSession::from_pass(
                    program, admitted, pass,
                )),
            }),
        }
    }

    /// Projects reset rule-attempt core parts into the public cursor.
    pub(super) fn from_first_parts(
        program: &'program ExecutableProgram,
        parts: AttemptRunCoreParts<E, A>,
        cursor: FirstRuntimeRulePassCursor<'program>,
    ) -> Self {
        match cursor {
            RuntimeRulePassCursor::Continuing(pass) => {
                Self::Continuing(BorrowedContinuingRuleAttemptSession {
                    session: ContinuingRuleAttemptSession::First(AttemptSession {
                        program,
                        core: parts.with_pass(pass),
                    }),
                })
            }
            RuntimeRulePassCursor::Final(pass) => Self::Final(BorrowedFinalRuleAttemptSession {
                session: FinalRuleAttemptSession::First(AttemptSession {
                    program,
                    core: parts.with_pass(pass),
                }),
            }),
        }
    }

    /// Projects after-miss rule-attempt core parts into the public cursor.
    pub(super) fn from_after_miss_parts(
        program: &'program ExecutableProgram,
        parts: AttemptRunCoreParts<E, A>,
        cursor: MissedRuntimeRulePassCursor<'program>,
    ) -> Self {
        match cursor {
            RuntimeRulePassCursor::Continuing(pass) => {
                Self::Continuing(BorrowedContinuingRuleAttemptSession {
                    session: ContinuingRuleAttemptSession::AfterMiss(AttemptSession {
                        program,
                        core: parts.with_pass(pass),
                    }),
                })
            }
            RuntimeRulePassCursor::Final(pass) => Self::Final(BorrowedFinalRuleAttemptSession {
                session: FinalRuleAttemptSession::AfterMiss(AttemptSession {
                    program,
                    core: parts.with_pass(pass),
                }),
            }),
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
            ContinuingRuleAttemptSession::First(session) => session.completed_steps(),
            ContinuingRuleAttemptSession::AfterMiss(session) => session.completed_steps(),
        }
    }

    /// Number of executable rule-line attempts consumed so far.
    #[must_use]
    pub const fn completed_attempts(&self) -> RuleAttemptCount {
        match &self.session {
            ContinuingRuleAttemptSession::First(session) => session.completed_attempts(),
            ContinuingRuleAttemptSession::AfterMiss(session) => session.completed_attempts(),
        }
    }

    /// Borrow the parsed program used by this session.
    #[must_use]
    pub const fn program(&self) -> &'program ExecutableProgram {
        match &self.session {
            ContinuingRuleAttemptSession::First(session) => session.program,
            ContinuingRuleAttemptSession::AfterMiss(session) => session.program,
        }
    }

    /// Borrow the current runtime state.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        match &self.session {
            ContinuingRuleAttemptSession::First(session) => session.state(),
            ContinuingRuleAttemptSession::AfterMiss(session) => session.state(),
        }
    }

    /// Advances a continuing rule-attempt session by exactly one executable rule line.
    #[must_use]
    pub fn step(self) -> BorrowedContinuingRuleAttemptTransition<'program, E, A> {
        match self.session {
            ContinuingRuleAttemptSession::First(session) => {
                advance_continuing_borrowed_rule_attempt(session)
            }
            ContinuingRuleAttemptSession::AfterMiss(session) => {
                advance_continuing_borrowed_rule_attempt(session)
            }
        }
    }
}

impl<'program, E: ExecutionPolicy, A: RuleAttemptPolicy>
    BorrowedFinalRuleAttemptSession<'program, E, A>
{
    /// Number of execution steps that have already completed in this run.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        match &self.session {
            FinalRuleAttemptSession::First(session) => session.completed_steps(),
            FinalRuleAttemptSession::AfterMiss(session) => session.completed_steps(),
        }
    }

    /// Number of executable rule-line attempts consumed so far.
    #[must_use]
    pub const fn completed_attempts(&self) -> RuleAttemptCount {
        match &self.session {
            FinalRuleAttemptSession::First(session) => session.completed_attempts(),
            FinalRuleAttemptSession::AfterMiss(session) => session.completed_attempts(),
        }
    }

    /// Borrow the parsed program used by this session.
    #[must_use]
    pub const fn program(&self) -> &'program ExecutableProgram {
        match &self.session {
            FinalRuleAttemptSession::First(session) => session.program,
            FinalRuleAttemptSession::AfterMiss(session) => session.program,
        }
    }

    /// Borrow the current runtime state.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        match &self.session {
            FinalRuleAttemptSession::First(session) => session.state(),
            FinalRuleAttemptSession::AfterMiss(session) => session.state(),
        }
    }

    /// Advances a final rule-attempt session by exactly one executable rule line.
    #[must_use]
    pub fn step(self) -> BorrowedFinalRuleAttemptTransition<'program, E, A> {
        match self.session {
            FinalRuleAttemptSession::First(session) => advance_final_borrowed_rule_attempt(session),
            FinalRuleAttemptSession::AfterMiss(session) => {
                advance_final_borrowed_rule_attempt(session)
            }
        }
    }
}

/// Advances a borrowed ordinary run and projects the private transition into the public type.
fn step_borrowed_run<'program, E: ExecutionPolicy>(
    session: BorrowedRunSession<'program, E>,
) -> BorrowedStepTransition<'program, E> {
    match session.session.advance_run_step() {
        RunAdvance::Rewritten(rewrite) => project_run_rewrite(rewrite),
        RunAdvance::Returned(returned) => project_run_return(returned),
        RunAdvance::Stable(terminal) => BorrowedStepTransition::Stable(BorrowedStableRun {
            program: terminal.program,
            core: terminal.core,
        }),
        RunAdvance::Failed(failure) => BorrowedStepTransition::Failed(BorrowedFailedRun::new(
            failure.error,
            failure.terminal.program,
            failure.terminal.core,
        )),
    }
}

/// Projects an exact private rewrite payload into the public stepwise transition.
fn project_run_rewrite<'program, E: ExecutionPolicy>(
    rewrite: RunRewrite<'program, E>,
) -> BorrowedStepTransition<'program, E> {
    match rewrite {
        RunRewrite::Always {
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
        RunRewrite::Once {
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
    }
}

/// Projects an exact private return payload into the public stepwise transition.
fn project_run_return<'program, E: ExecutionPolicy>(
    returned: RunReturn<'program>,
) -> BorrowedStepTransition<'program, E> {
    match returned {
        RunReturn::Always {
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
        RunReturn::Once {
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
    }
}
