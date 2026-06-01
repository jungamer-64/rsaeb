use crate::error::{RunError, RunFinishError, RunStartError, TracedRunError};
use crate::input::AdmittedRun;
use crate::limits::{RuleAttemptCount, StepCount};
use crate::policy::{ExecutionPolicy, ParsePolicy, RuleAttemptPolicy};
use crate::program::{Program, RunResult};
use crate::trace::{BorrowedTraceEvent, RuntimeStateView};

use super::advance::{
    BorrowedRunWitness, CoreAppliedRule, CoreRuleAttemptStep, CoreStep, OwnedRunWitness,
    advance_borrowed_rule_attempt, advance_owned_rule_attempt, advance_run,
};
use super::engine::{
    AttemptSession, BorrowedProgram, OwnedProgram, RunCore, Session, TerminalAttemptSession,
};
use super::transition::{
    BorrowedAppliedStep, BorrowedFailedRun, BorrowedMissedRuleAttempt, BorrowedReturnedRun,
    BorrowedRuleAttemptAppliedStep, BorrowedRuleAttemptFailedRun,
    BorrowedRuleAttemptReturnedRun, BorrowedRuleAttemptStableRun, BorrowedRuleAttemptTransition,
    BorrowedStableRun, BorrowedStepTransition, OwnedAppliedStep, OwnedFailedRun,
    OwnedMissedRuleAttempt, OwnedReturnedRun, OwnedRuleAttemptAppliedStep, OwnedRuleAttemptFailedRun,
    OwnedRuleAttemptReturnedRun, OwnedRuleAttemptStableRun, OwnedRuleAttemptTransition,
    OwnedStableRun, OwnedStepTransition,
};

/// Stateful run session that borrows a reusable parsed program.
///
/// This is the stepwise form returned by
/// [`Program::execute`](crate::program::Program::execute) with
/// [`BorrowedSteps`](crate::execution::BorrowedSteps). It consumes itself on
/// every step so callers must handle the returned [`BorrowedStepTransition`]
/// before they can continue.
pub struct BorrowedRunSession<'program, P: ParsePolicy, E: ExecutionPolicy> {
    /// Internal session using the public borrowed program boundary.
    pub(super) session: Session<BorrowedProgram<'program, P>, E>,
}

/// Stateful run session that owns its parsed program.
///
/// This is the stepwise form returned by
/// [`Program::into_execute`](crate::program::Program::into_execute) with
/// [`OwnedSteps`](crate::execution::OwnedSteps). It is useful when the session
/// must move independently of a borrowed [`Program`]. Owned terminal and failed
/// states retain a way to recover the parsed program instead of leaking
/// ownership through a parallel API.
pub struct OwnedRunSession<P: ParsePolicy, E: ExecutionPolicy> {
    /// Internal session using the public owned program boundary.
    pub(super) session: Session<OwnedProgram<P>, E>,
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
    pub(super) session: AttemptSession<BorrowedProgram<'program, P>, E, A>,
}

/// Stateful run session that owns its parsed program and advances by rule attempt.
///
/// This is the owned counterpart to [`BorrowedRuleAttemptSession`].
pub struct OwnedRuleAttemptSession<P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// Internal rule-attempt session using the public owned program boundary.
    pub(super) session: AttemptSession<OwnedProgram<P>, E, A>,
}

/// Terminal data split out of a borrowed ordinary run session.
struct BorrowedRunTerminal<'program, P: ParsePolicy, E: ExecutionPolicy> {
    /// Parsed program borrowed by the terminal state.
    program: &'program Program<P>,
    /// Runtime core retained for terminal state observation or materialization.
    core: RunCore<E>,
}

/// Terminal data split out of an owned ordinary run session.
struct OwnedRunTerminal<P: ParsePolicy, E: ExecutionPolicy> {
    /// Parsed program retained by the terminal state.
    program: Program<P>,
    /// Runtime core retained for terminal state observation or materialization.
    core: RunCore<E>,
}

/// Terminal data split out of a borrowed rule-attempt run session.
struct BorrowedRuleAttemptTerminal<'program, P: ParsePolicy, E: ExecutionPolicy> {
    /// Parsed program borrowed by the terminal state.
    program: &'program Program<P>,
    /// Runtime core retained for terminal state observation or materialization.
    core: RunCore<E>,
    /// Rule attempts consumed before the terminal boundary was reached.
    attempts: RuleAttemptCount,
    /// Rewrite steps committed before the terminal boundary was reached.
    steps: StepCount,
}

/// Terminal data split out of an owned rule-attempt run session.
struct OwnedRuleAttemptTerminal<P: ParsePolicy, E: ExecutionPolicy> {
    /// Parsed program retained by the terminal state.
    program: Program<P>,
    /// Runtime core retained for terminal state observation or materialization.
    core: RunCore<E>,
    /// Rule attempts consumed before the terminal boundary was reached.
    attempts: RuleAttemptCount,
    /// Rewrite steps committed before the terminal boundary was reached.
    steps: StepCount,
}

/// Runs a borrowed program to completion.
///
/// # Errors
///
/// Returns `RunError` when execution setup fails or a later matching rule would
/// exceed configured limits.
pub(crate) fn finish_borrowed_run<P: ParsePolicy, E: ExecutionPolicy>(
    program: &Program<P>,
    admitted: AdmittedRun<E>,
) -> Result<RunResult, RunError> {
    Session::new(BorrowedProgram { program }, admitted)
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
    program: &'program Program<P>,
    admitted: AdmittedRun<E>,
    trace: F,
) -> Result<RunResult, TracedRunError<TraceError>>
where
    P: ParsePolicy,
    E: ExecutionPolicy,
    F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), TraceError>,
{
    Session::new(BorrowedProgram { program }, admitted)
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
        program: &'program Program<P>,
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
    pub fn program(&self) -> &'program Program<P> {
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
    /// Builds a public active rule-attempt session from the internal session.
    const fn from_active(session: AttemptSession<BorrowedProgram<'program, P>, E, A>) -> Self {
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
    pub fn program(&self) -> &'program Program<P> {
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

impl<P: ParsePolicy, E: ExecutionPolicy> OwnedRunSession<P, E> {
    /// Starts a new owned run session for a parsed program and admitted run witness.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule state fails.
    pub(crate) fn new(
        program: Program<P>,
        admitted: AdmittedRun<E>,
    ) -> Result<Self, RunStartError> {
        Ok(Self {
            session: Session::new(OwnedProgram { program }, admitted)?,
        })
    }

    /// Number of execution steps that have already completed in this run.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.session.completed_steps()
    }

    /// Borrow the parsed program owned by this session.
    #[must_use]
    pub fn program(&self) -> &Program<P> {
        self.session.program()
    }

    /// Discards the current run state and recovers the owned parsed program.
    ///
    /// This intentionally drops the in-progress runtime state; it is for
    /// ownership recovery, not for retrying the same admitted run.
    #[must_use]
    pub fn into_program(self) -> Program<P> {
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
    pub fn step(self) -> OwnedStepTransition<P, E> {
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

impl<P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy> OwnedRuleAttemptSession<P, E, A> {
    /// Builds a public active rule-attempt session from the internal session.
    const fn from_active(session: AttemptSession<OwnedProgram<P>, E, A>) -> Self {
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

    /// Borrow the parsed program owned by this session.
    #[must_use]
    pub fn program(&self) -> &Program<P> {
        self.session.program()
    }

    /// Discards the current run state and recovers the owned parsed program.
    ///
    /// This intentionally drops the in-progress runtime state; it is for
    /// ownership recovery, not for retrying the same admitted run.
    #[must_use]
    pub fn into_program(self) -> Program<P> {
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
    pub fn step(self) -> OwnedRuleAttemptTransition<P, E, A> {
        step_owned_rule_attempt_run(self)
    }
}

impl<'program, P: ParsePolicy, E: ExecutionPolicy> BorrowedRunTerminal<'program, P, E> {
    /// Splits a borrowed run session into terminal data.
    fn from_session(session: BorrowedRunSession<'program, P, E>) -> Self {
        let Session { program, core } = session.session;
        Self {
            program: program.program,
            core,
        }
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> OwnedRunTerminal<P, E> {
    /// Splits an owned run session into terminal data.
    fn from_session(session: OwnedRunSession<P, E>) -> Self {
        let (program, core) = session.session.into_program_core();
        Self { program, core }
    }
}

impl<'program, P: ParsePolicy, E: ExecutionPolicy> BorrowedRuleAttemptTerminal<'program, P, E> {
    /// Projects terminal borrowed rule-attempt state into public terminal data.
    fn from_terminal<A: RuleAttemptPolicy>(
        terminal: TerminalAttemptSession<BorrowedProgram<'program, P>, E, A>,
    ) -> Self {
        let TerminalAttemptSession {
            program,
            core,
            attempt_budget,
        } = terminal;
        let steps = core.completed_steps();
        Self {
            program: program.program,
            core,
            attempts: attempt_budget.completed_attempts(),
            steps,
        }
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> OwnedRuleAttemptTerminal<P, E> {
    /// Projects terminal owned rule-attempt state into public terminal data.
    fn from_terminal<A: RuleAttemptPolicy>(
        terminal: TerminalAttemptSession<OwnedProgram<P>, E, A>,
    ) -> Self {
        let TerminalAttemptSession {
            program,
            core,
            attempt_budget,
        } = terminal;
        let steps = core.completed_steps();
        Self {
            program: program.program,
            core,
            attempts: attempt_budget.completed_attempts(),
            steps,
        }
    }
}

/// Advances a borrowed ordinary run and projects the private transition into the public type.
fn step_borrowed_run<'program, P: ParsePolicy, E: ExecutionPolicy>(
    mut session: BorrowedRunSession<'program, P, E>,
) -> BorrowedStepTransition<'program, P, E> {
    match advance_run::<_, _, BorrowedRunWitness>(
        session.session.program.program,
        &mut session.session.core,
    ) {
        Ok(CoreStep::Applied(CoreAppliedRule::Rewrite { step, rule })) => {
            BorrowedStepTransition::Applied(BorrowedAppliedStep {
                step,
                rule,
                session,
            })
        }
        Ok(CoreStep::Applied(CoreAppliedRule::Return {
            step,
            rule,
            output_view: _,
            output,
        })) => {
            let terminal = BorrowedRunTerminal::from_session(session);
            BorrowedStepTransition::Returned(BorrowedReturnedRun {
                step,
                rule,
                program: terminal.program,
                output,
            })
        }
        Ok(CoreStep::Stable(steps)) => {
            let terminal = BorrowedRunTerminal::from_session(session);
            BorrowedStepTransition::Stable(BorrowedStableRun {
                steps,
                program: terminal.program,
                core: terminal.core,
            })
        }
        Err(error) => {
            let terminal = BorrowedRunTerminal::from_session(session);
            BorrowedStepTransition::Failed(BorrowedFailedRun::new(
                error,
                terminal.program,
                terminal.core,
            ))
        }
    }
}

/// Advances an owned ordinary run and projects the private transition into the public type.
fn step_owned_run<P: ParsePolicy, E: ExecutionPolicy>(
    mut session: OwnedRunSession<P, E>,
) -> OwnedStepTransition<P, E> {
    match advance_run::<_, _, OwnedRunWitness>(
        &session.session.program.program,
        &mut session.session.core,
    ) {
        Ok(CoreStep::Applied(CoreAppliedRule::Rewrite { step, rule })) => {
            OwnedStepTransition::Applied(OwnedAppliedStep {
                step,
                rule,
                session,
            })
        }
        Ok(CoreStep::Applied(CoreAppliedRule::Return {
            step,
            rule,
            output_view: _,
            output,
        })) => {
            let terminal = OwnedRunTerminal::from_session(session);
            OwnedStepTransition::Returned(OwnedReturnedRun {
                step,
                rule,
                program: terminal.program,
                output,
            })
        }
        Ok(CoreStep::Stable(steps)) => {
            let terminal = OwnedRunTerminal::from_session(session);
            OwnedStepTransition::Stable(OwnedStableRun {
                steps,
                program: terminal.program,
                core: terminal.core,
            })
        }
        Err(error) => {
            let terminal = OwnedRunTerminal::from_session(session);
            OwnedStepTransition::Failed(OwnedFailedRun::new(error, terminal.program, terminal.core))
        }
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
            steps,
            final_miss,
            terminal,
        } => {
            let terminal = BorrowedRuleAttemptTerminal::from_terminal(terminal);
            BorrowedRuleAttemptTransition::Stable(BorrowedRuleAttemptStableRun {
                attempts,
                steps,
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

/// Advances an owned rule-attempt run and projects the private transition into the public type.
fn step_owned_rule_attempt_run<P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy>(
    session: OwnedRuleAttemptSession<P, E, A>,
) -> OwnedRuleAttemptTransition<P, E, A> {
    match advance_owned_rule_attempt(session.session) {
        CoreRuleAttemptStep::Missed {
            attempt,
            miss,
            continuation,
        } => OwnedRuleAttemptTransition::Missed(OwnedMissedRuleAttempt {
            attempt,
            miss,
            session: OwnedRuleAttemptSession::from_active(continuation),
        }),
        CoreRuleAttemptStep::Applied {
            attempt,
            step,
            rule,
            continuation,
        } => OwnedRuleAttemptTransition::Applied(OwnedRuleAttemptAppliedStep {
            attempt,
            step,
            rule,
            session: OwnedRuleAttemptSession::from_active(continuation),
        }),
        CoreRuleAttemptStep::Returned {
            attempt,
            step,
            rule,
            output,
            terminal,
        } => {
            let terminal = OwnedRuleAttemptTerminal::from_terminal(terminal);
            OwnedRuleAttemptTransition::Returned(OwnedRuleAttemptReturnedRun {
                attempt,
                step,
                rule,
                program: terminal.program,
                output,
            })
        }
        CoreRuleAttemptStep::Stable {
            attempts,
            steps,
            final_miss,
            terminal,
        } => {
            let terminal = OwnedRuleAttemptTerminal::from_terminal(terminal);
            OwnedRuleAttemptTransition::Stable(OwnedRuleAttemptStableRun {
                attempts,
                steps,
                final_miss,
                program: terminal.program,
                core: terminal.core,
            })
        }
        CoreRuleAttemptStep::Failed { error, terminal } => {
            let terminal = OwnedRuleAttemptTerminal::from_terminal(terminal);
            OwnedRuleAttemptTransition::Failed(OwnedRuleAttemptFailedRun::new(
                error,
                terminal.attempts,
                terminal.program,
                terminal.core,
            ))
        }
    }
}
