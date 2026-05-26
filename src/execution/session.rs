use crate::error::{RunError, RunFinishError, RunStartError, TracedRunError};
use crate::input::RunSeed;
use crate::limits::{RuleAttemptCount, StepCount};
use crate::program::{Program, RunResult};
use crate::trace::{BorrowedTraceEvent, RuntimeStateView};

use super::admission::RuleAttemptSeed;
use super::engine::{
    AttemptSession, BorrowedProgram, CoreAppliedRule, CoreRuleAttempt, CoreStep, OwnedProgram,
    RunCore, Session,
};
use super::transition::{
    BorrowedAppliedStep, BorrowedFailedRun, BorrowedMissedRuleAttempt, BorrowedReturnedRun,
    BorrowedRuleAttemptAppliedStep, BorrowedRuleAttemptFailedRun, BorrowedRuleAttemptReturnedRun,
    BorrowedRuleAttemptStableRun, BorrowedRuleAttemptTransition, BorrowedStableRun,
    BorrowedStepTransition, OwnedAppliedStep, OwnedFailedRun, OwnedMissedRuleAttempt,
    OwnedReturnedRun, OwnedRuleAttemptAppliedStep, OwnedRuleAttemptFailedRun,
    OwnedRuleAttemptReturnedRun, OwnedRuleAttemptStableRun, OwnedRuleAttemptTransition,
    OwnedStableRun, OwnedStepTransition,
};

/// Stateful run session that borrows a reusable parsed program.
///
/// This is the stepwise form returned by
/// [`Program::start_run`](crate::program::Program::start_run). It consumes
/// itself on every step so callers must handle the returned
/// [`BorrowedStepTransition`] before they can continue.
pub struct BorrowedRunSession<'program> {
    /// Internal session using the public borrowed program boundary.
    pub(super) session: Session<BorrowedProgram<'program>>,
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
    pub(super) session: Session<OwnedProgram>,
}

/// Stateful run session that borrows a reusable parsed program and advances by rule attempt.
///
/// A rule-attempt step consumes one executable rule line even when that rule
/// does not apply. Committed non-terminal rule applications reset the rule
/// cursor to the first executable rule.
pub struct BorrowedRuleAttemptSession<'program> {
    /// Internal rule-attempt session using the public borrowed program boundary.
    pub(super) session: AttemptSession<BorrowedProgram<'program>>,
}

/// Stateful run session that owns its parsed program and advances by rule attempt.
///
/// This is the owned counterpart to [`BorrowedRuleAttemptSession`].
pub struct OwnedRuleAttemptSession {
    /// Internal rule-attempt session using the public owned program boundary.
    pub(super) session: AttemptSession<OwnedProgram>,
}

/// Runs a borrowed program to completion.
///
/// # Errors
///
/// Returns `RunError` when execution setup fails or a later matching rule would
/// exceed configured limits.
pub(crate) fn finish_borrowed_run(program: &Program, seed: RunSeed) -> Result<RunResult, RunError> {
    Session::new(BorrowedProgram { program }, seed)
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
pub(crate) fn run_with_borrowed_trace<'program, F, E>(
    program: &'program Program,
    seed: RunSeed,
    trace: F,
) -> Result<RunResult, TracedRunError<E>>
where
    F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), E>,
{
    Session::new(BorrowedProgram { program }, seed)
        .map_err(RunError::from)
        .map_err(TracedRunError::Run)?
        .run_with_borrowed_trace(trace)
}

impl<'program> BorrowedRunSession<'program> {
    /// Starts a new borrowed run session for a parsed program and admitted run
    /// seed.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule state fails.
    pub(crate) fn new(program: &'program Program, seed: RunSeed) -> Result<Self, RunStartError> {
        Ok(Self {
            session: Session::new(BorrowedProgram { program }, seed)?,
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
    /// Applying an ordinary rewrite returns [`BorrowedStepTransition::Applied`] with a
    /// continuation session. No match, `(return)`, and runtime failure all
    /// consume the session into terminal typestates.
    #[must_use]
    pub fn step(self) -> BorrowedStepTransition<'program> {
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

impl<'program> BorrowedRuleAttemptSession<'program> {
    /// Starts a new borrowed rule-attempt run session for a parsed program and admitted run seed.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule state fails.
    pub(crate) fn new(
        program: &'program Program,
        seed: RuleAttemptSeed,
    ) -> Result<Self, RunStartError> {
        let (seed, limit) = seed.into_parts();
        Ok(Self {
            session: AttemptSession::new(BorrowedProgram { program }, seed, limit)?,
        })
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

    /// Advances this run by exactly one executable rule line when possible.
    ///
    /// Non-matching rules return [`BorrowedRuleAttemptTransition::Missed`] with a
    /// continuation session. Matching rewrites return
    /// [`BorrowedRuleAttemptTransition::Applied`] and reset the next attempt to the
    /// first executable rule. No match across the whole pass, `(return)`, and
    /// runtime failure consume the session into terminal typestates.
    #[must_use]
    pub fn step(self) -> BorrowedRuleAttemptTransition<'program> {
        step_borrowed_rule_attempt_run(self)
    }
}

impl OwnedRunSession {
    /// Starts a new owned run session for a parsed program and admitted run seed.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule state fails.
    pub(crate) fn new(program: Program, seed: RunSeed) -> Result<Self, RunStartError> {
        Ok(Self {
            session: Session::new(OwnedProgram { program }, seed)?,
        })
    }

    /// Number of execution steps that have already completed in this run.
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
    pub fn step(self) -> OwnedStepTransition {
        step_owned_run(self)
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

impl OwnedRuleAttemptSession {
    /// Starts a new owned rule-attempt run session for a parsed program and admitted run seed.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule state fails.
    pub(crate) fn new(program: Program, seed: RuleAttemptSeed) -> Result<Self, RunStartError> {
        let (seed, limit) = seed.into_parts();
        Ok(Self {
            session: AttemptSession::new(OwnedProgram { program }, seed, limit)?,
        })
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

    /// Advances this run by exactly one executable rule line when possible.
    ///
    /// Non-matching rules return [`OwnedRuleAttemptTransition::Missed`] with a
    /// continuation session. Matching rewrites return
    /// [`OwnedRuleAttemptTransition::Applied`] and reset the next attempt to
    /// the first executable rule. Owned terminal and failed states keep the
    /// parsed program recoverable.
    #[must_use]
    pub fn step(self) -> OwnedRuleAttemptTransition {
        step_owned_rule_attempt_run(self)
    }
}

/// Splits a borrowed run session into the public terminal data.
fn borrowed_run_terminal_parts(session: BorrowedRunSession<'_>) -> (&Program, RunCore) {
    let Session { program, core } = session.session;
    (program.program, core)
}

/// Splits an owned run session into the public terminal data.
fn owned_run_terminal_parts(session: OwnedRunSession) -> (Program, RunCore) {
    session.session.into_program_core()
}

/// Splits a borrowed rule-attempt session into the public terminal data.
fn borrowed_rule_attempt_terminal_parts(
    session: BorrowedRuleAttemptSession<'_>,
) -> (&Program, RunCore, RuleAttemptCount) {
    let AttemptSession {
        program,
        core,
        cursor: _,
        attempt_budget,
    } = session.session;
    (program.program, core, attempt_budget.completed_attempts())
}

/// Splits an owned rule-attempt session into the public terminal data.
fn owned_rule_attempt_terminal_parts(
    session: OwnedRuleAttemptSession,
) -> (Program, RunCore, RuleAttemptCount) {
    let AttemptSession {
        program,
        core,
        cursor: _,
        attempt_budget,
    } = session.session;
    (program.program, core, attempt_budget.completed_attempts())
}

/// Advances a borrowed ordinary run and builds the public transition explicitly.
fn step_borrowed_run<'program>(
    mut session: BorrowedRunSession<'program>,
) -> BorrowedStepTransition<'program> {
    match session.session.step_borrowed() {
        Ok(CoreStep::Applied(CoreAppliedRule::Rewrite { step, rule })) => {
            BorrowedStepTransition::Applied(BorrowedAppliedStep {
                step,
                rule,
                session,
            })
        }
        Ok(CoreStep::Applied(CoreAppliedRule::Return { step, rule, output })) => {
            let (program, _core) = borrowed_run_terminal_parts(session);
            BorrowedStepTransition::Returned(BorrowedReturnedRun {
                step,
                rule,
                program,
                output,
            })
        }
        Ok(CoreStep::Stable(steps)) => {
            let (program, core) = borrowed_run_terminal_parts(session);
            BorrowedStepTransition::Stable(BorrowedStableRun {
                steps,
                program,
                core,
            })
        }
        Err(error) => {
            let (program, core) = borrowed_run_terminal_parts(session);
            BorrowedStepTransition::Failed(BorrowedFailedRun::new(error, program, core))
        }
    }
}

/// Advances an owned ordinary run and builds the public transition explicitly.
fn step_owned_run(mut session: OwnedRunSession) -> OwnedStepTransition {
    match session.session.step_owned() {
        Ok(CoreStep::Applied(CoreAppliedRule::Rewrite { step, rule })) => {
            OwnedStepTransition::Applied(OwnedAppliedStep {
                step,
                rule,
                session,
            })
        }
        Ok(CoreStep::Applied(CoreAppliedRule::Return { step, rule, output })) => {
            let (program, _core) = owned_run_terminal_parts(session);
            OwnedStepTransition::Returned(OwnedReturnedRun {
                step,
                rule,
                program,
                output,
            })
        }
        Ok(CoreStep::Stable(steps)) => {
            let (program, core) = owned_run_terminal_parts(session);
            OwnedStepTransition::Stable(OwnedStableRun {
                steps,
                program,
                core,
            })
        }
        Err(error) => {
            let (program, core) = owned_run_terminal_parts(session);
            OwnedStepTransition::Failed(OwnedFailedRun::new(error, program, core))
        }
    }
}

/// Advances a borrowed rule-attempt run and builds the public transition explicitly.
fn step_borrowed_rule_attempt_run<'program>(
    mut session: BorrowedRuleAttemptSession<'program>,
) -> BorrowedRuleAttemptTransition<'program> {
    match session.session.step_borrowed() {
        Ok(CoreRuleAttempt::Missed { attempt, miss }) => {
            BorrowedRuleAttemptTransition::Missed(BorrowedMissedRuleAttempt {
                attempt,
                miss,
                session,
            })
        }
        Ok(CoreRuleAttempt::Applied {
            attempt,
            applied: CoreAppliedRule::Rewrite { step, rule },
        }) => BorrowedRuleAttemptTransition::Applied(BorrowedRuleAttemptAppliedStep {
            attempt,
            step,
            rule,
            session,
        }),
        Ok(CoreRuleAttempt::Applied {
            attempt,
            applied: CoreAppliedRule::Return { step, rule, output },
        }) => {
            let (program, _core, _attempts) = borrowed_rule_attempt_terminal_parts(session);
            BorrowedRuleAttemptTransition::Returned(BorrowedRuleAttemptReturnedRun {
                attempt,
                step,
                rule,
                program,
                output,
            })
        }
        Ok(CoreRuleAttempt::Stable {
            attempts,
            steps,
            stable_reason,
        }) => {
            let (program, core, _completed_attempts) =
                borrowed_rule_attempt_terminal_parts(session);
            BorrowedRuleAttemptTransition::Stable(BorrowedRuleAttemptStableRun {
                attempts,
                steps,
                stable_reason,
                program,
                core,
            })
        }
        Err(error) => {
            let (program, core, attempts) = borrowed_rule_attempt_terminal_parts(session);
            BorrowedRuleAttemptTransition::Failed(BorrowedRuleAttemptFailedRun::new(
                error, attempts, program, core,
            ))
        }
    }
}

/// Advances an owned rule-attempt run and builds the public transition explicitly.
fn step_owned_rule_attempt_run(mut session: OwnedRuleAttemptSession) -> OwnedRuleAttemptTransition {
    match session.session.step_owned() {
        Ok(CoreRuleAttempt::Missed { attempt, miss }) => {
            OwnedRuleAttemptTransition::Missed(OwnedMissedRuleAttempt {
                attempt,
                miss,
                session,
            })
        }
        Ok(CoreRuleAttempt::Applied {
            attempt,
            applied: CoreAppliedRule::Rewrite { step, rule },
        }) => OwnedRuleAttemptTransition::Applied(OwnedRuleAttemptAppliedStep {
            attempt,
            step,
            rule,
            session,
        }),
        Ok(CoreRuleAttempt::Applied {
            attempt,
            applied: CoreAppliedRule::Return { step, rule, output },
        }) => {
            let (program, _core, _attempts) = owned_rule_attempt_terminal_parts(session);
            OwnedRuleAttemptTransition::Returned(OwnedRuleAttemptReturnedRun {
                attempt,
                step,
                rule,
                program,
                output,
            })
        }
        Ok(CoreRuleAttempt::Stable {
            attempts,
            steps,
            stable_reason,
        }) => {
            let (program, core, _completed_attempts) = owned_rule_attempt_terminal_parts(session);
            OwnedRuleAttemptTransition::Stable(OwnedRuleAttemptStableRun {
                attempts,
                steps,
                stable_reason,
                program,
                core,
            })
        }
        Err(error) => {
            let (program, core, attempts) = owned_rule_attempt_terminal_parts(session);
            OwnedRuleAttemptTransition::Failed(OwnedRuleAttemptFailedRun::new(
                error, attempts, program, core,
            ))
        }
    }
}
