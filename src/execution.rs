//! Public stepwise run typestates.
//!
//! [`Program::start_run`](crate::program::Program::start_run) borrows a parsed
//! program and returns a [`RunSession`]. [`Program::into_run`](crate::program::Program::into_run)
//! moves a parsed program into an [`OwnedRunSession`]. Both session types use
//! the same runtime core; the ownership mode only decides where the rule table
//! lives while committed rule positions are resolved for inspection.
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
use crate::input::{InitialStateBytes, RuntimeInput};
use crate::inspect::{RulePosition, RuleView};
use crate::limits::{RunLimits, StepCount};
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

/// Stateful run session that can still apply rules.
///
/// This borrowed session and [`OwnedRunSession`] are the only public states
/// with `step` methods. Stable and returned runs are represented by separate
/// terminal types, so callers cannot step after completion.
pub struct RunSession<'program> {
    program: &'program Program,
    core: RunCore,
}

/// Stateful run session that owns its parsed program.
///
/// This is the owned counterpart of [`RunSession`]. It uses the same runtime
/// core and step semantics while carrying the parsed program by value.
pub struct OwnedRunSession {
    program: Program,
    core: RunCore,
}

#[derive(Debug)]
struct RunCore {
    state: State,
    scratch: RewriteScratch,
    budget: RuntimeBudgetState,
    once_states: OnceStateSet,
}

/// Result of advancing a run session once.
///
/// The transition is exhaustive over the public run lifecycle: one rule
/// committed and execution can continue, no rule matched, a `(return)` rule
/// produced final output, or a matching rule failed before commit.
pub enum StepTransition<'program> {
    /// One ordinary rewrite rule was applied and execution can continue.
    Applied(AppliedStep<'program>),
    /// No rule matched the final runtime state.
    Stable(StableRun),
    /// A matched rule executed `(return)`.
    Returned(ReturnedRun<'program>),
    /// A matching rule failed before committing.
    Failed(FailedRun<'program>),
}

/// Result of advancing an owned run session once.
///
/// This mirrors [`StepTransition`] without borrowing a program from the caller.
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
pub struct AppliedStep<'program> {
    step: StepCount,
    rule: RulePosition,
    session: RunSession<'program>,
}

/// One committed non-terminal rule application for an owned run session.
pub struct OwnedAppliedStep {
    step: StepCount,
    rule: RulePosition,
    session: OwnedRunSession,
}

/// Terminal run state reached by no matching rule.
///
/// Stable runs still own the final runtime state until the caller either
/// borrows it or materializes it with [`StableRun::into_result`].
pub struct StableRun {
    steps: StepCount,
    core: RunCore,
}

/// Terminal owned run state reached by no matching rule.
pub struct OwnedStableRun {
    steps: StepCount,
    program: Program,
    core: RunCore,
}

/// Terminal run state reached by `(return)`.
///
/// The output is a borrowed return output until the caller materializes the
/// terminal [`RunResult`] through [`ReturnedRun::into_result`].
pub struct ReturnedRun<'program> {
    step: StepCount,
    rule: RulePosition,
    program: &'program Program,
}

/// Terminal owned run state reached by `(return)`.
pub struct OwnedReturnedRun {
    step: StepCount,
    rule: RulePosition,
    program: Program,
}

/// Runtime failure that preserves the uncommitted state for inspection.
///
/// Step failures happen before the candidate rewrite is committed. This is a
/// terminal public state: callers can inspect the uncommitted state, then
/// discard the failed run into its runtime error.
pub struct FailedRun<'program> {
    error: RunError,
    session: RunSession<'program>,
}

/// Runtime failure that preserves owned uncommitted state for inspection.
pub struct OwnedFailedRun {
    error: RunError,
    session: OwnedRunSession,
}

impl core::fmt::Debug for RunSession<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RunSession")
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

impl core::fmt::Debug for StepTransition<'_> {
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

impl core::fmt::Debug for AppliedStep<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AppliedStep")
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

impl core::fmt::Debug for StableRun {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("StableRun")
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

impl core::fmt::Debug for ReturnedRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ReturnedRun")
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

impl core::fmt::Debug for FailedRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("FailedRun")
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
    /// Returns `RunError` if input exceeds runtime state limits or per-run
    /// rule state allocation fails.
    fn new(program: &Program, input: RuntimeInput, limits: RunLimits) -> Result<Self, RunError> {
        let budget = RuntimeBudgetState::new(limits);
        let input = InitialStateBytes::from_runtime_input(input, budget)?;
        let state = State::from_input(input);
        let once_states = OnceStateSet::new(program.once_slot_count())?;
        Ok(Self {
            state,
            scratch: RewriteScratch::new(),
            budget,
            once_states,
        })
    }

    const fn completed_steps(&self) -> StepCount {
        self.budget.completed_steps()
    }

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

enum CoreStep {
    Applied(AppliedRule),
    Stable(StepCount),
}

impl<'program> RunSession<'program> {
    /// Starts a new run session for a parsed program and validated input.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if the consumed runtime input exceeds this run's
    /// state limits or if allocating per-run rule state fails.
    pub(crate) fn new(
        program: &'program Program,
        input: RuntimeInput,
        limits: RunLimits,
    ) -> Result<Self, RunError> {
        Ok(Self {
            program,
            core: RunCore::new(program, input, limits)?,
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
    /// [`AppliedStep::into_session`] to continue after an applied rule.
    #[must_use]
    pub fn step(mut self) -> StepTransition<'program> {
        match self.core.step(self.program) {
            Ok(CoreStep::Applied(applied)) => applied.into_transition(self),
            Ok(CoreStep::Stable(steps)) => StepTransition::Stable(StableRun {
                steps,
                core: self.core,
            }),
            Err(error) => StepTransition::Failed(FailedRun::new(error, self)),
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
                StepTransition::Applied(applied) => {
                    self = applied.into_session();
                }
                StepTransition::Stable(stable) => {
                    return stable.into_result();
                }
                StepTransition::Returned(returned) => {
                    return returned.into_result();
                }
                StepTransition::Failed(failed) => return Err(failed.into_error()),
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
                StepTransition::Applied(applied) => {
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
                StepTransition::Stable(stable) => {
                    return stable.into_result().map_err(TracedRunError::Run);
                }
                StepTransition::Returned(returned) => {
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
                StepTransition::Failed(failed) => {
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
    /// Starts a new owned run session for a parsed program and validated input.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if the consumed runtime input exceeds this run's
    /// state limits or if allocating per-run rule state fails.
    pub(crate) fn new(
        program: Program,
        input: RuntimeInput,
        limits: RunLimits,
    ) -> Result<Self, RunError> {
        let core = RunCore::new(&program, input, limits)?;
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
    fn into_transition(self, session: RunSession<'_>) -> StepTransition<'_> {
        match self.effect {
            AppliedRuleEffect::Continue => StepTransition::Applied(AppliedStep {
                step: self.step,
                rule: self.rule,
                session,
            }),
            AppliedRuleEffect::Return => StepTransition::Returned(ReturnedRun {
                step: self.step,
                rule: self.rule,
                program: session.program,
            }),
        }
    }

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

impl<'program> AppliedStep<'program> {
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
    pub fn into_session(self) -> RunSession<'program> {
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

impl StableRun {
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

impl<'program> ReturnedRun<'program> {
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

impl<'program> FailedRun<'program> {
    fn new(error: RunError, session: RunSession<'program>) -> Self {
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

impl core::fmt::Display for FailedRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl core::error::Error for FailedRun<'_> {
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
