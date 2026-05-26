use crate::error::{RunError, RunFinishError, RunStartError, TracedRunError};
use crate::input::RunSeed;
use crate::limits::{RuleAttemptCount, StepCount};
use crate::program::{Program, ReturnOutput, RunResult};
use crate::trace::{BorrowedTraceEvent, RuntimeStateView};

use super::admission::RuleAttemptSeed;
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

/// Terminal data split out of a borrowed ordinary run session.
struct BorrowedRunTerminal<'program> {
    /// Parsed program borrowed by the terminal state.
    program: &'program Program,
    /// Runtime core retained for terminal state observation or materialization.
    core: RunCore,
}

/// Terminal data split out of an owned ordinary run session.
struct OwnedRunTerminal {
    /// Parsed program retained by the terminal state.
    program: Program,
    /// Runtime core retained for terminal state observation or materialization.
    core: RunCore,
}

/// Terminal data split out of a borrowed rule-attempt run session.
struct BorrowedRuleAttemptTerminal<'program> {
    /// Parsed program borrowed by the terminal state.
    program: &'program Program,
    /// Runtime core retained for terminal state observation or materialization.
    core: RunCore,
    /// Rule attempts consumed before the terminal boundary was reached.
    attempts: RuleAttemptCount,
}

/// Terminal data split out of an owned rule-attempt run session.
struct OwnedRuleAttemptTerminal {
    /// Parsed program retained by the terminal state.
    program: Program,
    /// Runtime core retained for terminal state observation or materialization.
    core: RunCore,
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

impl<'program> BorrowedRunTerminal<'program> {
    /// Splits a borrowed run session into terminal data.
    fn from_session(session: BorrowedRunSession<'program>) -> Self {
        let Session { program, core } = session.session;
        Self {
            program: program.program,
            core,
        }
    }
}

impl OwnedRunTerminal {
    /// Splits an owned run session into terminal data.
    fn from_session(session: OwnedRunSession) -> Self {
        let (program, core) = session.session.into_program_core();
        Self { program, core }
    }
}

impl<'program> BorrowedRuleAttemptTerminal<'program> {
    /// Splits a borrowed rule-attempt session into terminal data.
    fn from_session(session: BorrowedRuleAttemptSession<'program>) -> Self {
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

impl OwnedRuleAttemptTerminal {
    /// Splits an owned rule-attempt session into terminal data.
    fn from_session(session: OwnedRuleAttemptSession) -> Self {
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
fn borrowed_run_step_parts<'program>(
    mut session: BorrowedRunSession<'program>,
) -> RunStepParts<
    BorrowedRunSession<'program>,
    BorrowedRunTerminal<'program>,
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
fn owned_run_step_parts(
    mut session: OwnedRunSession,
) -> RunStepParts<
    OwnedRunSession,
    OwnedRunTerminal,
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
fn borrowed_rule_attempt_step_parts<'program>(
    mut session: BorrowedRuleAttemptSession<'program>,
) -> RuleAttemptStepParts<
    BorrowedRuleAttemptSession<'program>,
    BorrowedRuleAttemptTerminal<'program>,
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
fn owned_rule_attempt_step_parts(
    mut session: OwnedRuleAttemptSession,
) -> RuleAttemptStepParts<
    OwnedRuleAttemptSession,
    OwnedRuleAttemptTerminal,
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

impl<'program>
    From<
        RunStepParts<
            BorrowedRunSession<'program>,
            BorrowedRunTerminal<'program>,
            crate::inspect::RuleView<'program>,
            crate::error::RunStepError,
        >,
    > for BorrowedStepTransition<'program>
{
    fn from(
        parts: RunStepParts<
            BorrowedRunSession<'program>,
            BorrowedRunTerminal<'program>,
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

impl
    From<
        RunStepParts<
            OwnedRunSession,
            OwnedRunTerminal,
            super::witness::OwnedRuleWitness,
            crate::error::OwnedRunStepError,
        >,
    > for OwnedStepTransition
{
    fn from(
        parts: RunStepParts<
            OwnedRunSession,
            OwnedRunTerminal,
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

impl<'program>
    From<
        RuleAttemptStepParts<
            BorrowedRuleAttemptSession<'program>,
            BorrowedRuleAttemptTerminal<'program>,
            crate::inspect::RuleView<'program>,
            crate::error::RuleAttemptStepError,
        >,
    > for BorrowedRuleAttemptTransition<'program>
{
    fn from(
        parts: RuleAttemptStepParts<
            BorrowedRuleAttemptSession<'program>,
            BorrowedRuleAttemptTerminal<'program>,
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

impl
    From<
        RuleAttemptStepParts<
            OwnedRuleAttemptSession,
            OwnedRuleAttemptTerminal,
            super::witness::OwnedRuleWitness,
            crate::error::OwnedRuleAttemptStepError,
        >,
    > for OwnedRuleAttemptTransition
{
    fn from(
        parts: RuleAttemptStepParts<
            OwnedRuleAttemptSession,
            OwnedRuleAttemptTerminal,
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
fn step_borrowed_run<'program>(
    session: BorrowedRunSession<'program>,
) -> BorrowedStepTransition<'program> {
    borrowed_run_step_parts(session).into()
}

/// Advances an owned ordinary run and projects the private transition into the public type.
fn step_owned_run(session: OwnedRunSession) -> OwnedStepTransition {
    owned_run_step_parts(session).into()
}

/// Advances a borrowed rule-attempt run and projects the private transition into the public type.
fn step_borrowed_rule_attempt_run<'program>(
    session: BorrowedRuleAttemptSession<'program>,
) -> BorrowedRuleAttemptTransition<'program> {
    borrowed_rule_attempt_step_parts(session).into()
}

/// Advances an owned rule-attempt run and projects the private transition into the public type.
fn step_owned_rule_attempt_run(session: OwnedRuleAttemptSession) -> OwnedRuleAttemptTransition {
    owned_rule_attempt_step_parts(session).into()
}
