//! Public stepwise run typestates.
//!
//! [`Program::into_run`](crate::program::Program::into_run) moves a parsed
//! program into an [`OwnedRunSession`]. Borrowed run-to-completion remains an
//! internal implementation path for [`Program::run`](crate::program::Program::run)
//! and tracing, but public stepwise execution has one ownership model.
//!
//! Calling `step` consumes the current session and returns an exhaustive
//! transition, so callers must handle the next state explicitly: continue with
//! an applied step, finish a stable or returned run, or inspect and discard a
//! failed run.
//!
//! The run session is the mutable runtime engine. It owns the current state,
//! rewrite scratch, budgets, and per-run `(once)` state directly, so there is
//! no second private typestate layer and no borrowed input copy behind the
//! public API.

use crate::error::{RunError, TracedRunError};
use crate::input::RunSeed;
use crate::inspect::{RulePosition, RuleView};
use crate::limits::StepCount;
use crate::program::{Program, ReturnOutputView, RunResult};
use crate::runtime::action::{
    AppliedRule, AppliedRuleEffect, apply_matched_rule, materialize_return_output,
};
use crate::runtime::budget::RuntimeBudgetState;
use crate::runtime::matcher::{RuleSearch, find_next_match};
use crate::runtime::once::OnceStateSet;
use crate::runtime::rewrite::RewriteScratch;
use crate::runtime::state::State;
use crate::trace::{BorrowedTraceEffect, BorrowedTraceEvent, RuntimeStateView};

/// Internal borrowed run session used by run-to-completion APIs.
pub(crate) struct BorrowedRunSession<'program> {
    /// Parsed program borrowed for rule lookup and rule-view materialization.
    program: &'program Program,
    /// Mutable execution state shared with owned sessions.
    core: RunCore,
}

/// Stateful run session that owns its parsed program.
///
/// This is the owned counterpart of [`BorrowedRunSession`]. It uses the same runtime
/// core and step semantics while carrying the parsed program by value.
pub struct OwnedRunSession {
    /// Parsed program owned by this session for rule lookup and inspection.
    program: Program,
    /// Mutable execution state shared with borrowed sessions.
    core: RunCore,
}

/// Mutable runtime state independent of program ownership mode.
#[derive(Debug)]
struct RunCore {
    /// Current runtime byte state.
    state: State,
    /// Reusable buffer for candidate rewrites.
    scratch: RewriteScratch,
    /// Runtime limits and completed-step count.
    budget: RuntimeBudgetState,
    /// Per-run consumption state for `(once)` rules.
    once_states: OnceStateSet,
}

/// Internal result of advancing a borrowed run session once.
pub(crate) enum BorrowedStepTransition<'program> {
    /// One ordinary rewrite rule was applied and execution can continue.
    Applied(BorrowedAppliedStep<'program>),
    /// No rule matched the final runtime state.
    Stable(BorrowedStableRun),
    /// A matched rule executed `(return)`.
    Returned(BorrowedReturnedRun<'program>),
    /// A matching rule failed before committing.
    Failed(BorrowedFailedRun<'program>),
}

/// Result of advancing an owned run session once.
///
/// This mirrors [`BorrowedStepTransition`] without borrowing a program from the caller.
pub enum OwnedStepTransition {
    /// One ordinary rewrite rule was applied and execution can continue.
    Applied(OwnedAppliedStep),
    /// No rule matched the final runtime state.
    Stable(OwnedStableRun),
    /// A matched rule executed `(return)`.
    Returned(OwnedReturnedRun),
    /// A matching rule failed before committing.
    Failed(OwnedFailedRun),
}

/// One committed non-terminal rule application.
///
/// This value lets a caller inspect the applied rule and post-step state before
/// deciding whether to continue the run.
pub(crate) struct BorrowedAppliedStep<'program> {
    /// Step number committed by this transition.
    step: StepCount,
    /// Program-local rule position committed by this transition.
    rule: RulePosition,
    /// Continuation session after the committed rewrite.
    session: BorrowedRunSession<'program>,
}

/// One committed non-terminal rule application for an owned run session.
pub struct OwnedAppliedStep {
    /// Step number committed by this transition.
    step: StepCount,
    /// Program-local rule position committed by this transition.
    rule: RulePosition,
    /// Owned continuation session after the committed rewrite.
    session: OwnedRunSession,
}

/// Terminal run state reached by no matching rule.
///
/// Stable runs still own the final runtime state until the caller either
/// borrows it or materializes it with [`BorrowedStableRun::into_result`].
pub(crate) struct BorrowedStableRun {
    /// Number of committed steps before no rule matched.
    steps: StepCount,
    /// Terminal runtime core containing the stable state.
    core: RunCore,
}

/// Terminal owned run state reached by no matching rule.
pub struct OwnedStableRun {
    /// Number of committed steps before no rule matched.
    steps: StepCount,
    /// Parsed program retained by the owned terminal state.
    program: Program,
    /// Terminal runtime core containing the stable state.
    core: RunCore,
}

/// Terminal run state reached by `(return)`.
///
/// The output is a borrowed return output until the caller materializes the
/// terminal [`RunResult`] through [`BorrowedReturnedRun::into_result`].
pub(crate) struct BorrowedReturnedRun<'program> {
    /// Step number that executed the return action.
    step: StepCount,
    /// Program-local return rule position.
    rule: RulePosition,
    /// Parsed program used to borrow the return payload.
    program: &'program Program,
}

/// Terminal owned run state reached by `(return)`.
pub struct OwnedReturnedRun {
    /// Step number that executed the return action.
    step: StepCount,
    /// Program-local return rule position.
    rule: RulePosition,
    /// Parsed program used to borrow the return payload.
    program: Program,
}

/// Runtime failure that preserves the uncommitted state for inspection.
///
/// Step failures happen before the candidate rewrite is committed. This is a
/// terminal public state: callers can inspect the uncommitted state, then
/// discard the failed run into its runtime error.
pub(crate) struct BorrowedFailedRun<'program> {
    /// Runtime error that stopped the candidate step before commit.
    error: RunError,
    /// Uncommitted session retained for diagnostic inspection.
    session: BorrowedRunSession<'program>,
}

/// Runtime failure that preserves owned uncommitted state for inspection.
pub struct OwnedFailedRun {
    /// Runtime error that stopped the candidate step before commit.
    error: RunError,
    /// Uncommitted owned session retained for diagnostic inspection.
    session: OwnedRunSession,
}

impl core::fmt::Debug for BorrowedRunSession<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedRunSession")
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for OwnedRunSession {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedRunSession")
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for BorrowedStepTransition<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Applied(applied) => formatter.debug_tuple("Applied").field(applied).finish(),
            Self::Stable(stable) => formatter.debug_tuple("Stable").field(stable).finish(),
            Self::Returned(returned) => formatter.debug_tuple("Returned").field(returned).finish(),
            Self::Failed(failed) => formatter.debug_tuple("Failed").field(failed).finish(),
        }
    }
}

impl core::fmt::Debug for OwnedStepTransition {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Applied(applied) => formatter.debug_tuple("Applied").field(applied).finish(),
            Self::Stable(stable) => formatter.debug_tuple("Stable").field(stable).finish(),
            Self::Returned(returned) => formatter.debug_tuple("Returned").field(returned).finish(),
            Self::Failed(failed) => formatter.debug_tuple("Failed").field(failed).finish(),
        }
    }
}

impl core::fmt::Debug for BorrowedAppliedStep<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedAppliedStep")
            .field("step", &self.step())
            .field("rule", &self.rule())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for OwnedAppliedStep {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedAppliedStep")
            .field("step", &self.step())
            .field("rule", &self.rule())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for BorrowedStableRun {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedStableRun")
            .field("steps", &self.steps())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for OwnedStableRun {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedStableRun")
            .field("steps", &self.steps())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for BorrowedReturnedRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedReturnedRun")
            .field("step", &self.step())
            .field("rule", &self.rule())
            .field("output", &self.output())
            .finish()
    }
}

impl core::fmt::Debug for OwnedReturnedRun {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedReturnedRun")
            .field("step", &self.step())
            .field("rule", &self.rule())
            .field("output", &self.output())
            .finish()
    }
}

impl core::fmt::Debug for BorrowedFailedRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedFailedRun")
            .field("error", &self.error())
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for OwnedFailedRun {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedFailedRun")
            .field("error", &self.error())
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl RunCore {
    /// Builds the mutable runtime core for one execution.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if per-run rule state allocation fails.
    fn new(program: &Program, seed: RunSeed) -> Result<Self, RunError> {
        let (input, limits) = seed.into_runtime_parts();
        let budget = RuntimeBudgetState::new(limits);
        let state = State::from_input(input);
        let once_states = OnceStateSet::new(program.once_slot_count())?;
        Ok(Self {
            state,
            scratch: RewriteScratch::new(),
            budget,
            once_states,
        })
    }

    /// Number of steps already committed in this core.
    const fn completed_steps(&self) -> StepCount {
        self.budget.completed_steps()
    }

    /// Borrows the current runtime state.
    fn state(&self) -> RuntimeStateView<'_> {
        self.state.view()
    }

    /// Materializes a stable terminal result.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if final state materialization cannot allocate.
    fn into_stable_result(self, steps: StepCount) -> Result<RunResult, RunError> {
        Ok(RunResult::stable(self.state.into_snapshot()?, steps))
    }

    /// Advances the mutable runtime core against the supplied immutable program.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if rule matching detects an internal invariant
    /// failure or if applying the matched rule exceeds limits or allocation
    /// fails.
    fn step(&mut self, program: &Program) -> Result<CoreStep, RunError> {
        let matched =
            match find_next_match(program.rule_slice(), &mut self.once_states, &self.state)? {
                RuleSearch::Matched(matched) => matched,
                RuleSearch::Stable => return Ok(CoreStep::Stable(self.budget.completed_steps())),
            };

        Ok(CoreStep::Applied(apply_matched_rule(
            &mut self.state,
            &mut self.scratch,
            &mut self.budget,
            matched,
        )?))
    }
}

/// Internal non-error result of one core step attempt.
enum CoreStep {
    /// A rule committed and may have terminal side effects.
    Applied(AppliedRule),
    /// No rule matched the current runtime state.
    Stable(StepCount),
}

/// Runs a borrowed program to completion without exposing borrowed stepwise typestates.
///
/// # Errors
///
/// Returns `RunError` when execution setup fails or a later matching rule would
/// exceed configured limits.
pub(crate) fn finish_borrowed_run(
    program: &Program,
    seed: RunSeed,
) -> Result<RunResult, RunError> {
    BorrowedRunSession::new(program, seed)?.finish()
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
    BorrowedRunSession::new(program, seed)
        .map_err(TracedRunError::Run)?
        .run_with_borrowed_trace(trace)
}

impl<'program> BorrowedRunSession<'program> {
    /// Starts a new run session for a parsed program and admitted run seed.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if allocating per-run rule state fails.
    pub(crate) fn new(program: &'program Program, seed: RunSeed) -> Result<Self, RunError> {
        Ok(Self {
            program,
            core: RunCore::new(program, seed)?,
        })
    }

    /// Number of rewrite steps that have already completed in this run.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.core.completed_steps()
    }

    /// Borrow the current runtime state.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }

    /// Advances this run by exactly one matching rule when possible.
    ///
    /// Consuming `self` makes terminal states explicit. Call
    /// [`BorrowedAppliedStep::into_session`] to continue after an applied rule.
    #[must_use]
    pub fn step(mut self) -> BorrowedStepTransition<'program> {
        match self.core.step(self.program) {
            Ok(CoreStep::Applied(applied)) => applied.into_transition(self),
            Ok(CoreStep::Stable(steps)) => BorrowedStepTransition::Stable(BorrowedStableRun {
                steps,
                core: self.core,
            }),
            Err(error) => BorrowedStepTransition::Failed(BorrowedFailedRun::new(error, self)),
        }
    }

    /// Runs this session to completion.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when applying a later matching rule would exceed the
    /// configured limits, allocation fails, or state-size arithmetic overflows.
    pub fn finish(mut self) -> Result<RunResult, RunError> {
        loop {
            match self.step() {
                BorrowedStepTransition::Applied(applied) => {
                    self = applied.into_session();
                }
                BorrowedStepTransition::Stable(stable) => {
                    return stable.into_result();
                }
                BorrowedStepTransition::Returned(returned) => {
                    return returned.into_result();
                }
                BorrowedStepTransition::Failed(failed) => return Err(failed.into_error()),
            }
        }
    }

    /// Runs to completion while emitting borrowed trace events.
    ///
    /// # Errors
    ///
    /// Returns `TracedRunError::Trace` if the trace sink fails. Returns
    /// `TracedRunError::Run` if runtime execution fails.
    pub(crate) fn run_with_borrowed_trace<F, E>(
        mut self,
        mut trace: F,
    ) -> Result<RunResult, TracedRunError<E>>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), E>,
    {
        trace(BorrowedTraceEvent::Initial {
            state: self.state(),
        })
        .map_err(TracedRunError::Trace)?;

        loop {
            match self.step() {
                BorrowedStepTransition::Applied(applied) => {
                    let rule = applied.rule().map_err(TracedRunError::Run)?;
                    Self::emit_step_trace(
                        &mut trace,
                        applied.step(),
                        rule,
                        BorrowedTraceEffect::Continue {
                            state: applied.state(),
                        },
                    )?;
                    self = applied.into_session();
                }
                BorrowedStepTransition::Stable(stable) => {
                    return stable.into_result().map_err(TracedRunError::Run);
                }
                BorrowedStepTransition::Returned(returned) => {
                    let rule = returned.rule().map_err(TracedRunError::Run)?;
                    let output = returned.output().map_err(TracedRunError::Run)?;
                    Self::emit_step_trace(
                        &mut trace,
                        returned.step(),
                        rule,
                        BorrowedTraceEffect::Return { output },
                    )?;
                    return returned.into_result().map_err(TracedRunError::Run);
                }
                BorrowedStepTransition::Failed(failed) => {
                    return Err(TracedRunError::Run(failed.into_error()));
                }
            }
        }
    }

    /// Emits one borrowed step trace event.
    ///
    /// # Errors
    ///
    /// Returns `TracedRunError::Trace` if the trace sink rejects the event.
    fn emit_step_trace<F, E>(
        trace: &mut F,
        step: StepCount,
        rule: RuleView<'program>,
        effect: BorrowedTraceEffect<'program, '_>,
    ) -> Result<(), TracedRunError<E>>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), E>,
    {
        trace(BorrowedTraceEvent::Step { step, rule, effect }).map_err(TracedRunError::Trace)
    }
}

impl OwnedRunSession {
    /// Starts a new owned run session for a parsed program and admitted run seed.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if allocating per-run rule state fails.
    pub(crate) fn new(program: Program, seed: RunSeed) -> Result<Self, RunError> {
        let core = RunCore::new(&program, seed)?;
        Ok(Self { program, core })
    }

    /// Number of rewrite steps that have already completed in this run.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.core.completed_steps()
    }

    /// Borrow the parsed program owned by this session.
    #[must_use]
    pub const fn program(&self) -> &Program {
        &self.program
    }

    /// Borrow the current runtime state.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }

    /// Advances this owned run by exactly one matching rule when possible.
    #[must_use]
    pub fn step(mut self) -> OwnedStepTransition {
        match self.core.step(&self.program) {
            Ok(CoreStep::Applied(applied)) => applied.into_owned_transition(self),
            Ok(CoreStep::Stable(steps)) => OwnedStepTransition::Stable(OwnedStableRun {
                steps,
                program: self.program,
                core: self.core,
            }),
            Err(error) => OwnedStepTransition::Failed(OwnedFailedRun::new(error, self)),
        }
    }

    /// Runs this owned session to completion.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when applying a later matching rule would exceed the
    /// configured limits, allocation fails, or state-size arithmetic overflows.
    pub fn finish(mut self) -> Result<RunResult, RunError> {
        loop {
            match self.step() {
                OwnedStepTransition::Applied(applied) => {
                    self = applied.into_session();
                }
                OwnedStepTransition::Stable(stable) => {
                    return stable.into_result();
                }
                OwnedStepTransition::Returned(returned) => {
                    return returned.into_result();
                }
                OwnedStepTransition::Failed(failed) => return Err(failed.into_error()),
            }
        }
    }
}

impl AppliedRule {
    /// Converts the internal applied rule into the borrowed public transition.
    fn into_transition(self, session: BorrowedRunSession<'_>) -> BorrowedStepTransition<'_> {
        match self.effect {
            AppliedRuleEffect::Continue => BorrowedStepTransition::Applied(BorrowedAppliedStep {
                step: self.step,
                rule: self.rule,
                session,
            }),
            AppliedRuleEffect::Return => BorrowedStepTransition::Returned(BorrowedReturnedRun {
                step: self.step,
                rule: self.rule,
                program: session.program,
            }),
        }
    }

    /// Converts the internal applied rule into the owned public transition.
    fn into_owned_transition(self, session: OwnedRunSession) -> OwnedStepTransition {
        match self.effect {
            AppliedRuleEffect::Continue => OwnedStepTransition::Applied(OwnedAppliedStep {
                step: self.step,
                rule: self.rule,
                session,
            }),
            AppliedRuleEffect::Return => OwnedStepTransition::Returned(OwnedReturnedRun {
                step: self.step,
                rule: self.rule,
                program: session.program,
            }),
        }
    }
}

impl<'program> BorrowedAppliedStep<'program> {
    /// One-based applied step count.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Structured view of the applied rule.
    ///
    /// # Errors
    ///
    /// Returns `RunError::InternalInvariant` if the committed rule position no
    /// longer resolves inside the parsed program that produced this session.
    pub fn rule(&self) -> Result<RuleView<'program>, RunError> {
        self.session.program.rule_view_at(self.rule)
    }

    /// Runtime state after the applied rewrite step.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Continue running after observing this applied step.
    #[must_use]
    pub fn into_session(self) -> BorrowedRunSession<'program> {
        self.session
    }
}

impl OwnedAppliedStep {
    /// One-based applied step count.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Structured view of the applied rule.
    ///
    /// # Errors
    ///
    /// Returns `RunError::InternalInvariant` if the committed rule position no
    /// longer resolves inside the owned parsed program.
    pub fn rule(&self) -> Result<RuleView<'_>, RunError> {
        self.session.program.rule_view_at(self.rule)
    }

    /// Runtime state after the applied rewrite step.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Continue running after observing this applied step.
    #[must_use]
    pub fn into_session(self) -> OwnedRunSession {
        self.session
    }
}

impl BorrowedStableRun {
    /// Number of rewrite steps applied before reaching the stable state.
    #[must_use]
    pub const fn steps(&self) -> StepCount {
        self.steps
    }

    /// Borrowed final runtime state.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }

    /// Materializes this stable run as a run result.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if final state materialization cannot allocate.
    pub fn into_result(self) -> Result<RunResult, RunError> {
        self.core.into_stable_result(self.steps)
    }
}

impl OwnedStableRun {
    /// Number of rewrite steps applied before reaching the stable state.
    #[must_use]
    pub const fn steps(&self) -> StepCount {
        self.steps
    }

    /// Borrow the parsed program owned by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &Program {
        &self.program
    }

    /// Borrowed final runtime state.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }

    /// Materializes this stable run as a run result.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if final state materialization cannot allocate.
    pub fn into_result(self) -> Result<RunResult, RunError> {
        self.core.into_stable_result(self.steps)
    }
}

impl<'program> BorrowedReturnedRun<'program> {
    /// One-based applied step count for the return rule.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Structured view of the return rule.
    ///
    /// # Errors
    ///
    /// Returns `RunError::InternalInvariant` if the committed rule position no
    /// longer resolves inside the parsed program that produced this session.
    pub fn rule(&self) -> Result<RuleView<'program>, RunError> {
        self.program.rule_view_at(self.rule)
    }

    /// Borrowed return output from runtime execution.
    ///
    /// # Errors
    ///
    /// Returns `RunError::InternalInvariant` if the committed return rule no
    /// longer resolves to a `(return)` action.
    pub fn output(&self) -> Result<ReturnOutputView<'program>, RunError> {
        self.program.return_output_at(self.rule)
    }

    /// Materializes this returned run as a run result.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if return output materialization cannot allocate.
    pub fn into_result(self) -> Result<RunResult, RunError> {
        Ok(RunResult::from_return(
            materialize_return_output(self.output()?)?,
            self.step,
        ))
    }
}

impl OwnedReturnedRun {
    /// One-based applied step count for the return rule.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Borrow the parsed program owned by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &Program {
        &self.program
    }

    /// Structured view of the return rule.
    ///
    /// # Errors
    ///
    /// Returns `RunError::InternalInvariant` if the committed rule position no
    /// longer resolves inside the owned parsed program.
    pub fn rule(&self) -> Result<RuleView<'_>, RunError> {
        self.program.rule_view_at(self.rule)
    }

    /// Borrowed return output from runtime execution.
    ///
    /// # Errors
    ///
    /// Returns `RunError::InternalInvariant` if the committed return rule no
    /// longer resolves to a `(return)` action.
    pub fn output(&self) -> Result<ReturnOutputView<'_>, RunError> {
        self.program.return_output_at(self.rule)
    }

    /// Materializes this returned run as a run result.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if return output materialization cannot allocate.
    pub fn into_result(self) -> Result<RunResult, RunError> {
        Ok(RunResult::from_return(
            materialize_return_output(self.output()?)?,
            self.step,
        ))
    }
}

impl<'program> BorrowedFailedRun<'program> {
    /// Captures a failed borrowed session without committing the attempted step.
    fn new(error: RunError, session: BorrowedRunSession<'program>) -> Self {
        Self { error, session }
    }

    /// Runtime error that prevented the step from committing.
    #[must_use]
    pub const fn error(&self) -> &RunError {
        &self.error
    }

    /// Number of rewrite steps that completed before the failed step attempt.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.session.completed_steps()
    }

    /// Borrow the uncommitted runtime state preserved by this error.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Discard the uncommitted run session and return the runtime error.
    #[must_use]
    pub fn into_error(self) -> RunError {
        self.error
    }
}

impl OwnedFailedRun {
    /// Captures a failed owned session without committing the attempted step.
    fn new(error: RunError, session: OwnedRunSession) -> Self {
        Self { error, session }
    }

    /// Runtime error that prevented the step from committing.
    #[must_use]
    pub const fn error(&self) -> &RunError {
        &self.error
    }

    /// Number of rewrite steps that completed before the failed step attempt.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.session.completed_steps()
    }

    /// Borrow the parsed program owned by this failed session.
    #[must_use]
    pub const fn program(&self) -> &Program {
        self.session.program()
    }

    /// Borrow the uncommitted runtime state preserved by this error.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Discard the uncommitted run session and return the runtime error.
    #[must_use]
    pub fn into_error(self) -> RunError {
        self.error
    }
}

impl core::fmt::Display for BorrowedFailedRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl core::error::Error for BorrowedFailedRun<'_> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl core::fmt::Display for OwnedFailedRun {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl core::error::Error for OwnedFailedRun {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.error)
    }
}
