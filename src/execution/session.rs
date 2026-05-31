use crate::error::{RunError, RunFinishError, RunStartError, TracedRunError};
use crate::input::AdmittedRun;
use crate::limits::{RuleAttemptCount, StepCount};
use crate::policy::{ExecutionPolicy, ParsePolicy, RuleAttemptPolicy};
use crate::program::{Program, ReturnOutput, RunResult};
use crate::trace::{BorrowedTraceEvent, RuntimeStateView};

use super::attempt::{RuleAttemptStableReason, RuleMiss};
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
}

/// Terminal data split out of an owned rule-attempt run session.
struct OwnedRuleAttemptTerminal<P: ParsePolicy, E: ExecutionPolicy> {
    /// Parsed program retained by the terminal state.
    program: Program<P>,
    /// Runtime core retained for terminal state observation or materialization.
    core: RunCore<E>,
    /// Rule attempts consumed before the terminal boundary was reached.
    attempts: RuleAttemptCount,
}

/// Private ordinary-run transition vocabulary before projection into public borrowed/owned types.
enum RunStepParts<Continuation, Terminal, RuleWitness, StepError> {
    /// A rewrite committed and the run can continue.
    Applied {
        /// Committed step count.
        step: StepCount,
        /// Rule witness paired with the committed step.
        rule: RuleWitness,
        /// Continuation session after the committed step.
        continuation: Continuation,
    },
    /// A return rule committed and the run is terminal.
    Returned {
        /// Committed step count.
        step: StepCount,
        /// Rule witness paired with the committed return.
        rule: RuleWitness,
        /// Terminal session data.
        terminal: Terminal,
        /// Materialized return output.
        output: ReturnOutput,
    },
    /// No rule matched the final runtime state.
    Stable {
        /// Steps committed before stability.
        steps: StepCount,
        /// Terminal session data.
        terminal: Terminal,
    },
    /// A candidate step failed before commit.
    Failed {
        /// Error that prevented commit.
        error: StepError,
        /// Terminal session data preserving the uncommitted state.
        terminal: Terminal,
    },
}

/// Private rule-attempt transition vocabulary before projection into public borrowed/owned types.
enum RuleAttemptStepParts<Continuation, Terminal, RuleWitness, StepError> {
    /// A non-applying rule line was consumed and the run can continue.
    Missed {
        /// Committed rule-attempt count.
        attempt: RuleAttemptCount,
        /// Rule miss witness.
        miss: RuleMiss<RuleWitness>,
        /// Continuation session after the consumed attempt.
        continuation: Continuation,
    },
    /// A rewrite committed and the rule-attempt run can continue.
    Applied {
        /// Committed rule-attempt count.
        attempt: RuleAttemptCount,
        /// Committed rewrite step count.
        step: StepCount,
        /// Rule witness paired with the committed step.
        rule: RuleWitness,
        /// Continuation session after the committed step.
        continuation: Continuation,
    },
    /// A return rule committed and the run is terminal.
    Returned {
        /// Committed rule-attempt count.
        attempt: RuleAttemptCount,
        /// Committed step count.
        step: StepCount,
        /// Rule witness paired with the committed return.
        rule: RuleWitness,
        /// Terminal session data.
        terminal: Terminal,
        /// Materialized return output.
        output: ReturnOutput,
    },
    /// A rule pass completed without a match.
    Stable {
        /// Attempts consumed before stability.
        attempts: RuleAttemptCount,
        /// Steps committed before stability.
        steps: StepCount,
        /// Typed reason for stability.
        stable_reason: RuleAttemptStableReason<RuleWitness>,
        /// Terminal session data.
        terminal: Terminal,
    },
    /// A candidate rule attempt failed before committing runtime state.
    Failed {
        /// Error that prevented commit.
        error: StepError,
        /// Terminal session data preserving the uncommitted state.
        terminal: Terminal,
    },
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
    /// Starts a new borrowed rule-attempt run session for a parsed program and admitted run witness.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule state fails.
    pub(crate) fn new(
        program: &'program Program<P>,
        admitted: AdmittedRun<E>,
    ) -> Result<Self, RunStartError> {
        Ok(Self {
            session: AttemptSession::new(BorrowedProgram { program }, admitted)?,
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
    /// Starts a new owned rule-attempt run session for a parsed program and admitted run witness.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule state fails.
    pub(crate) fn new(
        program: Program<P>,
        admitted: AdmittedRun<E>,
    ) -> Result<Self, RunStartError> {
        Ok(Self {
            session: AttemptSession::new(OwnedProgram { program }, admitted)?,
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
    /// Splits a borrowed rule-attempt session into terminal data.
    fn from_session<A: RuleAttemptPolicy>(
        session: BorrowedRuleAttemptSession<'program, P, E, A>,
    ) -> Self {
        let AttemptSession {
            program,
            core,
            cursor: _,
            attempt_budget,
        } = session.session;
        Self {
            program: program.program,
            core,
            attempts: attempt_budget.completed_attempts(),
        }
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> OwnedRuleAttemptTerminal<P, E> {
    /// Splits an owned rule-attempt session into terminal data.
    fn from_session<A: RuleAttemptPolicy>(session: OwnedRuleAttemptSession<P, E, A>) -> Self {
        let AttemptSession {
            program,
            core,
            cursor: _,
            attempt_budget,
        } = session.session;
        Self {
            program: program.program,
            core,
            attempts: attempt_budget.completed_attempts(),
        }
    }
}

/// Advances a borrowed ordinary run through the private transition vocabulary.
fn borrowed_run_step_parts<'program, P: ParsePolicy, E: ExecutionPolicy>(
    mut session: BorrowedRunSession<'program, P, E>,
) -> RunStepParts<
    BorrowedRunSession<'program, P, E>,
    BorrowedRunTerminal<'program, P, E>,
    crate::inspect::RuleView<'program>,
    crate::error::RunStepError,
> {
    match session.session.step_borrowed() {
        Ok(CoreStep::Applied(CoreAppliedRule::Rewrite { step, rule })) => RunStepParts::Applied {
            step,
            rule,
            continuation: session,
        },
        Ok(CoreStep::Applied(CoreAppliedRule::Return { step, rule, output })) => {
            RunStepParts::Returned {
                step,
                rule,
                terminal: BorrowedRunTerminal::from_session(session),
                output,
            }
        }
        Ok(CoreStep::Stable(steps)) => RunStepParts::Stable {
            steps,
            terminal: BorrowedRunTerminal::from_session(session),
        },
        Err(error) => RunStepParts::Failed {
            error,
            terminal: BorrowedRunTerminal::from_session(session),
        },
    }
}

/// Advances an owned ordinary run through the private transition vocabulary.
fn owned_run_step_parts<P: ParsePolicy, E: ExecutionPolicy>(
    mut session: OwnedRunSession<P, E>,
) -> RunStepParts<
    OwnedRunSession<P, E>,
    OwnedRunTerminal<P, E>,
    super::witness::OwnedRuleWitness,
    crate::error::OwnedRunStepError,
> {
    match session.session.step_owned() {
        Ok(CoreStep::Applied(CoreAppliedRule::Rewrite { step, rule })) => RunStepParts::Applied {
            step,
            rule,
            continuation: session,
        },
        Ok(CoreStep::Applied(CoreAppliedRule::Return { step, rule, output })) => {
            RunStepParts::Returned {
                step,
                rule,
                terminal: OwnedRunTerminal::from_session(session),
                output,
            }
        }
        Ok(CoreStep::Stable(steps)) => RunStepParts::Stable {
            steps,
            terminal: OwnedRunTerminal::from_session(session),
        },
        Err(error) => RunStepParts::Failed {
            error,
            terminal: OwnedRunTerminal::from_session(session),
        },
    }
}

/// Advances a borrowed rule-attempt run through the private transition vocabulary.
fn borrowed_rule_attempt_step_parts<
    'program,
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
>(
    mut session: BorrowedRuleAttemptSession<'program, P, E, A>,
) -> RuleAttemptStepParts<
    BorrowedRuleAttemptSession<'program, P, E, A>,
    BorrowedRuleAttemptTerminal<'program, P, E>,
    crate::inspect::RuleView<'program>,
    crate::error::RuleAttemptStepError,
> {
    match session.session.step_borrowed() {
        Ok(CoreRuleAttempt::Missed { attempt, miss }) => RuleAttemptStepParts::Missed {
            attempt,
            miss,
            continuation: session,
        },
        Ok(CoreRuleAttempt::Applied {
            attempt,
            applied: CoreAppliedRule::Rewrite { step, rule },
        }) => RuleAttemptStepParts::Applied {
            attempt,
            step,
            rule,
            continuation: session,
        },
        Ok(CoreRuleAttempt::Applied {
            attempt,
            applied: CoreAppliedRule::Return { step, rule, output },
        }) => RuleAttemptStepParts::Returned {
            attempt,
            step,
            rule,
            terminal: BorrowedRuleAttemptTerminal::from_session(session),
            output,
        },
        Ok(CoreRuleAttempt::Stable {
            attempts,
            steps,
            stable_reason,
        }) => RuleAttemptStepParts::Stable {
            attempts,
            steps,
            stable_reason,
            terminal: BorrowedRuleAttemptTerminal::from_session(session),
        },
        Err(error) => RuleAttemptStepParts::Failed {
            error,
            terminal: BorrowedRuleAttemptTerminal::from_session(session),
        },
    }
}

/// Advances an owned rule-attempt run through the private transition vocabulary.
fn owned_rule_attempt_step_parts<P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy>(
    mut session: OwnedRuleAttemptSession<P, E, A>,
) -> RuleAttemptStepParts<
    OwnedRuleAttemptSession<P, E, A>,
    OwnedRuleAttemptTerminal<P, E>,
    super::witness::OwnedRuleWitness,
    crate::error::OwnedRuleAttemptStepError,
> {
    match session.session.step_owned() {
        Ok(CoreRuleAttempt::Missed { attempt, miss }) => RuleAttemptStepParts::Missed {
            attempt,
            miss,
            continuation: session,
        },
        Ok(CoreRuleAttempt::Applied {
            attempt,
            applied: CoreAppliedRule::Rewrite { step, rule },
        }) => RuleAttemptStepParts::Applied {
            attempt,
            step,
            rule,
            continuation: session,
        },
        Ok(CoreRuleAttempt::Applied {
            attempt,
            applied: CoreAppliedRule::Return { step, rule, output },
        }) => RuleAttemptStepParts::Returned {
            attempt,
            step,
            rule,
            terminal: OwnedRuleAttemptTerminal::from_session(session),
            output,
        },
        Ok(CoreRuleAttempt::Stable {
            attempts,
            steps,
            stable_reason,
        }) => RuleAttemptStepParts::Stable {
            attempts,
            steps,
            stable_reason,
            terminal: OwnedRuleAttemptTerminal::from_session(session),
        },
        Err(error) => RuleAttemptStepParts::Failed {
            error,
            terminal: OwnedRuleAttemptTerminal::from_session(session),
        },
    }
}

impl<'program, P: ParsePolicy, E: ExecutionPolicy>
    From<
        RunStepParts<
            BorrowedRunSession<'program, P, E>,
            BorrowedRunTerminal<'program, P, E>,
            crate::inspect::RuleView<'program>,
            crate::error::RunStepError,
        >,
    > for BorrowedStepTransition<'program, P, E>
{
    fn from(
        parts: RunStepParts<
            BorrowedRunSession<'program, P, E>,
            BorrowedRunTerminal<'program, P, E>,
            crate::inspect::RuleView<'program>,
            crate::error::RunStepError,
        >,
    ) -> Self {
        match parts {
            RunStepParts::Applied {
                step,
                rule,
                continuation,
            } => Self::Applied(BorrowedAppliedStep {
                step,
                rule,
                session: continuation,
            }),
            RunStepParts::Returned {
                step,
                rule,
                terminal,
                output,
            } => Self::Returned(BorrowedReturnedRun {
                step,
                rule,
                program: terminal.program,
                output,
            }),
            RunStepParts::Stable { steps, terminal } => Self::Stable(BorrowedStableRun {
                steps,
                program: terminal.program,
                core: terminal.core,
            }),
            RunStepParts::Failed { error, terminal } => Self::Failed(BorrowedFailedRun::new(
                error,
                terminal.program,
                terminal.core,
            )),
        }
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy>
    From<
        RunStepParts<
            OwnedRunSession<P, E>,
            OwnedRunTerminal<P, E>,
            super::witness::OwnedRuleWitness,
            crate::error::OwnedRunStepError,
        >,
    > for OwnedStepTransition<P, E>
{
    fn from(
        parts: RunStepParts<
            OwnedRunSession<P, E>,
            OwnedRunTerminal<P, E>,
            super::witness::OwnedRuleWitness,
            crate::error::OwnedRunStepError,
        >,
    ) -> Self {
        match parts {
            RunStepParts::Applied {
                step,
                rule,
                continuation,
            } => Self::Applied(OwnedAppliedStep {
                step,
                rule,
                session: continuation,
            }),
            RunStepParts::Returned {
                step,
                rule,
                terminal,
                output,
            } => Self::Returned(OwnedReturnedRun {
                step,
                rule,
                program: terminal.program,
                output,
            }),
            RunStepParts::Stable { steps, terminal } => Self::Stable(OwnedStableRun {
                steps,
                program: terminal.program,
                core: terminal.core,
            }),
            RunStepParts::Failed { error, terminal } => {
                Self::Failed(OwnedFailedRun::new(error, terminal.program, terminal.core))
            }
        }
    }
}

impl<'program, P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy>
    From<
        RuleAttemptStepParts<
            BorrowedRuleAttemptSession<'program, P, E, A>,
            BorrowedRuleAttemptTerminal<'program, P, E>,
            crate::inspect::RuleView<'program>,
            crate::error::RuleAttemptStepError,
        >,
    > for BorrowedRuleAttemptTransition<'program, P, E, A>
{
    fn from(
        parts: RuleAttemptStepParts<
            BorrowedRuleAttemptSession<'program, P, E, A>,
            BorrowedRuleAttemptTerminal<'program, P, E>,
            crate::inspect::RuleView<'program>,
            crate::error::RuleAttemptStepError,
        >,
    ) -> Self {
        match parts {
            RuleAttemptStepParts::Missed {
                attempt,
                miss,
                continuation,
            } => Self::Missed(BorrowedMissedRuleAttempt {
                attempt,
                miss,
                session: continuation,
            }),
            RuleAttemptStepParts::Applied {
                attempt,
                step,
                rule,
                continuation,
            } => Self::Applied(BorrowedRuleAttemptAppliedStep {
                attempt,
                step,
                rule,
                session: continuation,
            }),
            RuleAttemptStepParts::Returned {
                attempt,
                step,
                rule,
                terminal,
                output,
            } => Self::Returned(BorrowedRuleAttemptReturnedRun {
                attempt,
                step,
                rule,
                program: terminal.program,
                output,
            }),
            RuleAttemptStepParts::Stable {
                attempts,
                steps,
                stable_reason,
                terminal,
            } => Self::Stable(BorrowedRuleAttemptStableRun {
                attempts,
                steps,
                stable_reason,
                program: terminal.program,
                core: terminal.core,
            }),
            RuleAttemptStepParts::Failed { error, terminal } => {
                Self::Failed(BorrowedRuleAttemptFailedRun::new(
                    error,
                    terminal.attempts,
                    terminal.program,
                    terminal.core,
                ))
            }
        }
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy>
    From<
        RuleAttemptStepParts<
            OwnedRuleAttemptSession<P, E, A>,
            OwnedRuleAttemptTerminal<P, E>,
            super::witness::OwnedRuleWitness,
            crate::error::OwnedRuleAttemptStepError,
        >,
    > for OwnedRuleAttemptTransition<P, E, A>
{
    fn from(
        parts: RuleAttemptStepParts<
            OwnedRuleAttemptSession<P, E, A>,
            OwnedRuleAttemptTerminal<P, E>,
            super::witness::OwnedRuleWitness,
            crate::error::OwnedRuleAttemptStepError,
        >,
    ) -> Self {
        match parts {
            RuleAttemptStepParts::Missed {
                attempt,
                miss,
                continuation,
            } => Self::Missed(OwnedMissedRuleAttempt {
                attempt,
                miss,
                session: continuation,
            }),
            RuleAttemptStepParts::Applied {
                attempt,
                step,
                rule,
                continuation,
            } => Self::Applied(OwnedRuleAttemptAppliedStep {
                attempt,
                step,
                rule,
                session: continuation,
            }),
            RuleAttemptStepParts::Returned {
                attempt,
                step,
                rule,
                terminal,
                output,
            } => Self::Returned(OwnedRuleAttemptReturnedRun {
                attempt,
                step,
                rule,
                program: terminal.program,
                output,
            }),
            RuleAttemptStepParts::Stable {
                attempts,
                steps,
                stable_reason,
                terminal,
            } => Self::Stable(OwnedRuleAttemptStableRun {
                attempts,
                steps,
                stable_reason,
                program: terminal.program,
                core: terminal.core,
            }),
            RuleAttemptStepParts::Failed { error, terminal } => {
                Self::Failed(OwnedRuleAttemptFailedRun::new(
                    error,
                    terminal.attempts,
                    terminal.program,
                    terminal.core,
                ))
            }
        }
    }
}

/// Advances a borrowed ordinary run and projects the private transition into the public type.
fn step_borrowed_run<'program, P: ParsePolicy, E: ExecutionPolicy>(
    session: BorrowedRunSession<'program, P, E>,
) -> BorrowedStepTransition<'program, P, E> {
    borrowed_run_step_parts(session).into()
}

/// Advances an owned ordinary run and projects the private transition into the public type.
fn step_owned_run<P: ParsePolicy, E: ExecutionPolicy>(
    session: OwnedRunSession<P, E>,
) -> OwnedStepTransition<P, E> {
    owned_run_step_parts(session).into()
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
    borrowed_rule_attempt_step_parts(session).into()
}

/// Advances an owned rule-attempt run and projects the private transition into the public type.
fn step_owned_rule_attempt_run<P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy>(
    session: OwnedRuleAttemptSession<P, E, A>,
) -> OwnedRuleAttemptTransition<P, E, A> {
    owned_rule_attempt_step_parts(session).into()
}
