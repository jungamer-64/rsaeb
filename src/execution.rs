//! Public stepwise execution typestates.
//!
//! [`Program::start_execution`](crate::Program::start_execution) returns a
//! [`RunningExecution`]. Calling [`RunningExecution::step`] consumes that value
//! and returns an [`ExecutionTransition`], so callers must handle the next
//! state explicitly: continue with [`AppliedExecution::into_running`], finish a
//! [`StableExecution`], or finish a [`ReturnedExecution`].
//!
//! This shape keeps terminal executions separate from the only state that can
//! still step. Runtime internals stay private; public values expose borrowed
//! state and rule views that are valid for observation without making the
//! mutable runtime engine part of the API.

use crate::error::{RunError, TracedRunError};
use crate::program::{Program, RunLimits, RunResult, StepCount};
use crate::runtime::action::{AppliedRule, StepApplication, apply_matched_rule};
use crate::runtime::budget::StepBudget;
use crate::runtime::input::{InitialStateBytes, RuntimeInput};
use crate::runtime::matcher::{RuleSearch, find_next_match};
use crate::runtime::once::RuntimeRules;
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
pub struct RunningExecution<'program> {
    pub(crate) core: ExecutionCore<'program>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ExecutionCore<'program> {
    pub(crate) state: State,
    pub(crate) scratch: RewriteScratch,
    pub(crate) step_budget: StepBudget,
    pub(crate) runtime_rules: RuntimeRules<'program>,
    pub(crate) limits: RunLimits,
}

/// Result of advancing a running execution once.
///
/// The transition is exhaustive over the public execution lifecycle: one rule
/// committed and execution can continue, no rule matched, or a `(return)` rule
/// produced final output.
pub enum ExecutionTransition<'program> {
    /// One ordinary rewrite rule was applied and execution can continue.
    Applied(AppliedExecution<'program>),
    /// No rule matched the final runtime state.
    Stable(StableExecution<'program>),
    /// A matched rule executed `(return)`.
    Returned(ReturnedExecution<'program>),
}

/// One committed non-terminal rule application.
///
/// This value lets a caller inspect the applied rule and post-step state before
/// deciding whether to continue execution.
pub struct AppliedExecution<'program> {
    step: StepCount,
    rule: RuleView<'program>,
    execution: RunningExecution<'program>,
}

/// Terminal execution state reached by no matching rule.
///
/// Stable executions still own the final runtime state until the caller either
/// borrows it or materializes it with [`StableExecution::into_result`].
pub struct StableExecution<'program> {
    steps: StepCount,
    core: ExecutionCore<'program>,
}

/// Terminal execution state reached by `(return)`.
///
/// The output is a borrowed parsed payload until the caller materializes the
/// terminal [`RunResult`] through [`ReturnedExecution::into_result`].
#[derive(Clone, Copy)]
pub struct ReturnedExecution<'program> {
    step: StepCount,
    rule: RuleView<'program>,
    output: PayloadView<'program>,
}

/// Runtime failure that preserves the uncommitted running execution.
///
/// Step failures happen before the candidate rewrite is committed. The failed
/// [`RunningExecution`] is therefore returned by value so hosts can inspect,
/// retry with different limits, or discard it explicitly.
pub struct ExecutionStepError<'program> {
    error: RunError,
    execution: RunningExecution<'program>,
}

impl<'program> ExecutionCore<'program> {
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
        let input = InitialStateBytes::materialize(input, limits)?;
        let state = State::from_input(input);
        let runtime_rules = RuntimeRules::new(program.rule_slice())?;
        Ok(Self {
            state,
            scratch: RewriteScratch::new(),
            step_budget: StepBudget::new(limits.step_limit()),
            runtime_rules,
            limits,
        })
    }

    pub(crate) const fn completed_steps(&self) -> StepCount {
        self.step_budget.completed_steps()
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

impl<'program> RunningExecution<'program> {
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
            core: ExecutionCore::new(program, input, limits)?,
        })
    }

    /// Number of rewrite steps that have already completed in this execution.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.core.completed_steps()
    }

    /// Borrow the current runtime state.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }

    /// Advances this execution by exactly one matching rule when possible.
    ///
    /// Consuming `self` makes terminal states explicit. Call
    /// [`AppliedExecution::into_running`] to continue after an applied rule.
    ///
    /// # Errors
    ///
    /// Returns `ExecutionStepError` if the matching rule cannot commit because
    /// runtime limits or allocation fail. The error preserves the uncommitted
    /// execution.
    #[expect(
        clippy::result_large_err,
        reason = "ExecutionStepError preserves the uncommitted execution by value without allocating on the error path"
    )]
    pub fn step(mut self) -> Result<ExecutionTransition<'program>, ExecutionStepError<'program>> {
        let applied = {
            let ExecutionCore {
                state,
                scratch,
                step_budget,
                runtime_rules,
                limits,
            } = &mut self.core;

            let matched = match find_next_match(runtime_rules, state) {
                RuleSearch::Matched(matched) => matched,
                RuleSearch::Stable => {
                    let steps = step_budget.completed_steps();
                    return Ok(ExecutionTransition::Stable(StableExecution {
                        steps,
                        core: self.core,
                    }));
                }
            };

            apply_matched_rule(state, scratch, step_budget, *limits, matched)
        };

        let applied = match applied {
            Ok(applied) => applied,
            Err(error) => return Err(ExecutionStepError::new(error, self)),
        };

        Ok(applied.into_transition(self))
    }

    /// Runs this execution to completion.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when applying a later matching rule would exceed the
    /// configured limits, allocation fails, or state-size arithmetic overflows.
    pub fn finish(mut self) -> Result<RunResult, RunError> {
        loop {
            match self.step() {
                Ok(ExecutionTransition::Applied(applied)) => {
                    self = applied.into_running();
                }
                Ok(ExecutionTransition::Stable(stable)) => {
                    return stable.into_result();
                }
                Ok(ExecutionTransition::Returned(returned)) => {
                    return returned.into_result();
                }
                Err(error) => return Err(error.into_error()),
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn find_next_match(&mut self) -> RuleSearch<'program, '_> {
        find_next_match(&mut self.core.runtime_rules, &self.core.state)
    }
}

impl<'program> AppliedRule<'program> {
    fn into_transition(
        self,
        execution: RunningExecution<'program>,
    ) -> ExecutionTransition<'program> {
        match self.effect {
            StepApplication::Continue => ExecutionTransition::Applied(AppliedExecution {
                step: self.step,
                rule: self.rule.view(),
                execution,
            }),
            StepApplication::Return(output) => ExecutionTransition::Returned(ReturnedExecution {
                step: self.step,
                rule: self.rule.view(),
                output,
            }),
        }
    }
}

impl<'program> AppliedExecution<'program> {
    /// One-based applied step count.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Structured view of the applied rule.
    #[must_use]
    pub const fn rule(&self) -> RuleView<'program> {
        self.rule
    }

    /// Runtime state after the applied rewrite step.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.execution.state()
    }

    /// Continue running after observing this applied step.
    #[must_use]
    pub fn into_running(self) -> RunningExecution<'program> {
        self.execution
    }
}

impl StableExecution<'_> {
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

    /// Materializes this stable execution as a run result.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if final state materialization cannot allocate.
    pub fn into_result(self) -> Result<RunResult, RunError> {
        self.core.into_stable_result(self.steps)
    }
}

impl<'program> ReturnedExecution<'program> {
    /// One-based applied step count for the return rule.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Structured view of the return rule.
    #[must_use]
    pub const fn rule(&self) -> RuleView<'program> {
        self.rule
    }

    /// Borrowed return payload from the parsed program.
    #[must_use]
    pub const fn output(&self) -> PayloadView<'program> {
        self.output
    }

    /// Materializes this returned execution as a run result.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if return output materialization cannot allocate.
    pub fn into_result(self) -> Result<RunResult, RunError> {
        Ok(RunResult::from_return(
            ExecutionCore::materialize_return_output(self.output)?,
            self.step,
        ))
    }
}

impl<'program> ExecutionStepError<'program> {
    fn new(error: RunError, execution: RunningExecution<'program>) -> Self {
        Self { error, execution }
    }

    /// Runtime error that prevented the step from committing.
    #[must_use]
    pub const fn error(&self) -> &RunError {
        &self.error
    }

    /// Borrow the uncommitted execution.
    #[must_use]
    pub const fn execution(&self) -> &RunningExecution<'program> {
        &self.execution
    }

    /// Recover the uncommitted execution.
    #[must_use]
    pub fn into_execution(self) -> RunningExecution<'program> {
        self.execution
    }

    /// Discard the uncommitted execution and return the runtime error.
    #[must_use]
    pub fn into_error(self) -> RunError {
        self.error
    }
}

impl core::fmt::Display for ExecutionStepError<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl core::error::Error for ExecutionStepError<'_> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl<'program> RunningExecution<'program> {
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
                Ok(ExecutionTransition::Applied(applied)) => {
                    Self::emit_step_trace(
                        &mut trace,
                        applied.step(),
                        applied.rule(),
                        BorrowedTraceEffect::Continue {
                            state: applied.state(),
                        },
                    )?;
                    self = applied.into_running();
                }
                Ok(ExecutionTransition::Stable(stable)) => {
                    return stable.into_result().map_err(TracedRunError::Run);
                }
                Ok(ExecutionTransition::Returned(returned)) => {
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
