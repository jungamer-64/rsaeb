//! Public stepwise execution typestates.
//!
//! [`Program::start_run`](crate::Program::start_run) returns a
//! [`RuntimeSession`]. Calling [`RuntimeSession::step`] consumes that value and returns
//! a [`RuntimeStep`], so callers must handle the next state explicitly:
//! continue with [`RuntimeAppliedStep::into_session`], finish a [`RuntimeStableRun`], or
//! finish a [`RuntimeReturnedRun`].
//!
//! This shape keeps terminal executions separate from the only state that can
//! still step. Runtime internals stay private; public values expose borrowed
//! state and rule views that are valid for observation without making the
//! mutable runtime engine part of the API.

use crate::error::{RunError, TracedRunError};
use crate::program::{Program, RunLimits, RunResult, StepCount};
use crate::rule::Rule;
use crate::runtime::action::{
    AppliedRule, AppliedRuleEffect, apply_matched_rule, materialize_return_output,
};
use crate::runtime::budget::RuntimeBudgetState;
use crate::runtime::input::{InitialStateBytes, RuntimeInput};
use crate::runtime::matcher::{RuleSearch, find_next_match};
use crate::runtime::once::OnceStateSet;
use crate::runtime::rewrite::RewriteScratch;
use crate::runtime::state::State;
use crate::trace::{BorrowedTraceEffect, BorrowedTraceEvent, RuntimeStateView};
use crate::{inspect::PayloadView, inspect::RuleView};

/// Stateful execution that can still apply rules.
///
/// This type represents the only state with a `step` method. Stable and
/// returned executions are represented by separate terminal types, so callers
/// cannot step after completion. A running execution owns per-run `(once)`
/// state and the current runtime state.
pub(crate) struct RuntimeSession<'program> {
    pub(crate) core: RuntimeCore<'program>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RuntimeCore<'program> {
    pub(crate) state: State,
    pub(crate) scratch: RewriteScratch,
    pub(crate) budget: RuntimeBudgetState,
    pub(crate) rules: &'program [Rule],
    pub(crate) once_states: OnceStateSet,
}

/// Result of advancing a running execution once.
///
/// The transition is exhaustive over the public execution lifecycle: one rule
/// committed and execution can continue, no rule matched, or a `(return)` rule
/// produced final output.
pub(crate) enum RuntimeStep<'program> {
    /// One ordinary rewrite rule was applied and execution can continue.
    Applied(RuntimeAppliedStep<'program>),
    /// No rule matched the final runtime state.
    Stable(RuntimeStableRun<'program>),
    /// A matched rule executed `(return)`.
    Returned(RuntimeReturnedRun<'program>),
}

/// One committed non-terminal rule application.
///
/// This value lets a caller inspect the applied rule and post-step state before
/// deciding whether to continue execution.
pub(crate) struct RuntimeAppliedStep<'program> {
    step: StepCount,
    rule: RuleView<'program>,
    session: RuntimeSession<'program>,
}

/// Terminal execution state reached by no matching rule.
///
/// Stable executions still own the final runtime state until the caller either
/// borrows it or materializes it with [`RuntimeStableRun::into_result`].
pub(crate) struct RuntimeStableRun<'program> {
    steps: StepCount,
    core: RuntimeCore<'program>,
}

/// Terminal execution state reached by `(return)`.
///
/// The output is a borrowed parsed payload until the caller materializes the
/// terminal [`RunResult`] through [`RuntimeReturnedRun::into_result`].
#[derive(Clone, Copy)]
pub(crate) struct RuntimeReturnedRun<'program> {
    step: StepCount,
    rule: RuleView<'program>,
    output: PayloadView<'program>,
}

/// Runtime failure that preserves the uncommitted running execution.
///
/// Step failures happen before the candidate rewrite is committed. The failed
/// [`RuntimeSession`] is therefore returned by value so hosts can inspect,
/// update the limits with [`RuntimeSession::with_limits`], or discard it
/// explicitly.
pub(crate) struct RuntimeStepError<'program> {
    error: RunError,
    session: RuntimeSession<'program>,
}

impl core::fmt::Debug for RuntimeSession<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RuntimeSession")
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for RuntimeStep<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Applied(applied) => formatter.debug_tuple("Applied").field(applied).finish(),
            Self::Stable(stable) => formatter.debug_tuple("Stable").field(stable).finish(),
            Self::Returned(returned) => formatter.debug_tuple("Returned").field(returned).finish(),
        }
    }
}

impl core::fmt::Debug for RuntimeAppliedStep<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RuntimeAppliedStep")
            .field("step", &self.step())
            .field("rule", &self.rule())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for RuntimeStableRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RuntimeStableRun")
            .field("steps", &self.steps())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for RuntimeReturnedRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RuntimeReturnedRun")
            .field("step", &self.step())
            .field("rule", &self.rule())
            .field("output", &self.output())
            .finish()
    }
}

impl core::fmt::Debug for RuntimeStepError<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RuntimeStepError")
            .field("error", &self.error())
            .field("session", &self.session())
            .finish()
    }
}

impl<'program> RuntimeCore<'program> {
    /// Builds the mutable runtime core for one execution.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if input materialization fails, input exceeds runtime
    /// state limits, or per-execution rule state allocation fails.
    pub(crate) fn new(
        program: &'program Program,
        input: &RuntimeInput,
        limits: RunLimits,
    ) -> Result<Self, RunError> {
        let budget = RuntimeBudgetState::new(limits);
        let input = InitialStateBytes::materialize(input, budget)?;
        let state = State::from_input(input);
        let once_states = OnceStateSet::new(program.once_slot_count())?;
        Ok(Self {
            state,
            scratch: RewriteScratch::new(),
            budget,
            rules: program.rule_slice(),
            once_states,
        })
    }

    pub(crate) const fn completed_steps(&self) -> StepCount {
        self.budget.completed_steps()
    }

    pub(crate) fn state(&self) -> RuntimeStateView<'_> {
        self.state.view()
    }

    /// Materializes a stable terminal result.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if final state materialization cannot allocate.
    pub(crate) fn into_stable_result(self, steps: StepCount) -> Result<RunResult, RunError> {
        Ok(RunResult::stable(self.state.into_snapshot()?, steps))
    }
}

impl<'program> RuntimeSession<'program> {
    /// Starts a new running execution for a parsed program and validated input.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if runtime input materialization fails, state limits
    /// reject the input, or per-execution rule state allocation fails.
    pub(crate) fn new(
        program: &'program Program,
        input: &RuntimeInput,
        limits: RunLimits,
    ) -> Result<Self, RunError> {
        Ok(Self {
            core: RuntimeCore::new(program, input, limits)?,
        })
    }

    /// Number of rewrite steps that have already completed in this execution.
    #[must_use]
    pub(crate) const fn completed_steps(&self) -> StepCount {
        self.core.completed_steps()
    }

    /// Borrow the current runtime state.
    #[must_use]
    pub(crate) fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }

    /// Replaces runtime limits for this uncommitted execution.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if the already-completed step count or the current
    /// runtime state does not fit the replacement limits.
    pub(crate) fn with_limits(mut self, limits: RunLimits) -> Result<Self, RunError> {
        let state_len = self.core.state.byte_count();

        self.core.budget = self.core.budget.with_limits(limits, state_len)?;
        Ok(self)
    }

    /// Advances this execution by exactly one matching rule when possible.
    ///
    /// Consuming `self` makes terminal states explicit. Call
    /// [`RuntimeAppliedStep::into_session`] to continue after an applied rule.
    ///
    /// # Errors
    ///
    /// Returns `RuntimeStepError` if the matching rule cannot commit because
    /// runtime limits or allocation fail. The error preserves the uncommitted
    /// session.
    #[expect(
        clippy::result_large_err,
        reason = "RuntimeStepError preserves the uncommitted run session by value without allocating on the error path"
    )]
    pub(crate) fn step(mut self) -> Result<RuntimeStep<'program>, RuntimeStepError<'program>> {
        let applied = {
            let RuntimeCore {
                state,
                scratch,
                budget,
                rules,
                once_states,
            } = &mut self.core;

            let matched = match find_next_match(rules, once_states, state) {
                RuleSearch::Matched(matched) => matched,
                RuleSearch::Stable => {
                    let steps = budget.completed_steps();
                    return Ok(RuntimeStep::Stable(RuntimeStableRun {
                        steps,
                        core: self.core,
                    }));
                }
            };

            apply_matched_rule(state, scratch, budget, once_states, matched)
        };

        let applied = match applied {
            Ok(applied) => applied,
            Err(error) => return Err(RuntimeStepError::new(error, self)),
        };

        Ok(applied.into_transition(self))
    }

    /// Runs this execution to completion.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when applying a later matching rule would exceed the
    /// configured limits, allocation fails, or state-size arithmetic overflows.
    pub(crate) fn finish(mut self) -> Result<RunResult, RunError> {
        loop {
            match self.step() {
                Ok(RuntimeStep::Applied(applied)) => {
                    self = applied.into_session();
                }
                Ok(RuntimeStep::Stable(stable)) => {
                    return stable.into_result();
                }
                Ok(RuntimeStep::Returned(returned)) => {
                    return returned.into_result();
                }
                Err(error) => return Err(error.into_error()),
            }
        }
    }
}

impl<'program> AppliedRule<'program> {
    fn into_transition(self, session: RuntimeSession<'program>) -> RuntimeStep<'program> {
        match self.effect {
            AppliedRuleEffect::Continue => RuntimeStep::Applied(RuntimeAppliedStep {
                step: self.step,
                rule: self.rule,
                session,
            }),
            AppliedRuleEffect::Return(output) => RuntimeStep::Returned(RuntimeReturnedRun {
                step: self.step,
                rule: self.rule,
                output,
            }),
        }
    }
}

impl<'program> RuntimeAppliedStep<'program> {
    /// One-based applied step count.
    #[must_use]
    pub(crate) const fn step(&self) -> StepCount {
        self.step
    }

    /// Structured view of the applied rule.
    #[must_use]
    pub(crate) const fn rule(&self) -> RuleView<'program> {
        self.rule
    }

    /// Runtime state after the applied rewrite step.
    #[must_use]
    pub(crate) fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Continue running after observing this applied step.
    #[must_use]
    pub(crate) fn into_session(self) -> RuntimeSession<'program> {
        self.session
    }
}

impl RuntimeStableRun<'_> {
    /// Number of rewrite steps applied before reaching the stable state.
    #[must_use]
    pub(crate) const fn steps(&self) -> StepCount {
        self.steps
    }

    /// Borrowed final runtime state.
    #[must_use]
    pub(crate) fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }

    /// Materializes this stable execution as a run result.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if final state materialization cannot allocate.
    pub(crate) fn into_result(self) -> Result<RunResult, RunError> {
        self.core.into_stable_result(self.steps)
    }
}

impl<'program> RuntimeReturnedRun<'program> {
    /// One-based applied step count for the return rule.
    #[must_use]
    pub(crate) const fn step(&self) -> StepCount {
        self.step
    }

    /// Structured view of the return rule.
    #[must_use]
    pub(crate) const fn rule(&self) -> RuleView<'program> {
        self.rule
    }

    /// Borrowed return payload from the parsed program.
    #[must_use]
    pub(crate) const fn output(&self) -> PayloadView<'program> {
        self.output
    }

    /// Materializes this returned execution as a run result.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if return output materialization cannot allocate.
    pub(crate) fn into_result(self) -> Result<RunResult, RunError> {
        Ok(RunResult::from_return(
            materialize_return_output(self.output)?,
            self.step,
        ))
    }
}

impl<'program> RuntimeStepError<'program> {
    fn new(error: RunError, session: RuntimeSession<'program>) -> Self {
        Self { error, session }
    }

    /// Runtime error that prevented the step from committing.
    #[must_use]
    pub(crate) const fn error(&self) -> &RunError {
        &self.error
    }

    /// Borrow the uncommitted run session.
    #[must_use]
    pub(crate) const fn session(&self) -> &RuntimeSession<'program> {
        &self.session
    }

    /// Discard the uncommitted execution and return the runtime error.
    #[must_use]
    pub(crate) fn into_error(self) -> RunError {
        self.error
    }

    pub(crate) fn into_parts(self) -> (RunError, RuntimeSession<'program>) {
        (self.error, self.session)
    }
}

impl core::fmt::Display for RuntimeStepError<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl core::error::Error for RuntimeStepError<'_> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl<'program> RuntimeSession<'program> {
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
                Ok(RuntimeStep::Applied(applied)) => {
                    Self::emit_step_trace(
                        &mut trace,
                        applied.step(),
                        applied.rule(),
                        BorrowedTraceEffect::Continue {
                            state: applied.state(),
                        },
                    )?;
                    self = applied.into_session();
                }
                Ok(RuntimeStep::Stable(stable)) => {
                    return stable.into_result().map_err(TracedRunError::Run);
                }
                Ok(RuntimeStep::Returned(returned)) => {
                    Self::emit_step_trace(
                        &mut trace,
                        returned.step(),
                        returned.rule(),
                        BorrowedTraceEffect::Return {
                            output: returned.output(),
                        },
                    )?;
                    return returned.into_result().map_err(TracedRunError::Run);
                }
                Err(error) => return Err(TracedRunError::Run(error.into_error())),
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
