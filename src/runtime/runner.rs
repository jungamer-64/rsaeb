use core::convert::Infallible;

use crate::error::{RunError, TracedRunError};
use crate::program::{RunResult, StepCount};
use crate::rule::Rule;
use crate::trace::{BorrowedTraceEffect, BorrowedTraceEvent};

use super::action::StepApplication;
use super::execution::Execution;
use super::matcher::RuleSearch;
use super::terminal::ExecutionTerminal;

type NoTrace<'program> = for<'run> fn(BorrowedTraceEvent<'program, 'run>) -> Result<(), Infallible>;

impl<'program> Execution<'program> {
    /// Runs this execution from its current state to completion.
    ///
    /// This consumes the execution and preserves already-applied steps, `(once)`
    /// state, and byte budgets. It is the non-tracing counterpart to repeated
    /// calls to [`Execution::step`].
    ///
    /// # Errors
    ///
    /// Returns `RunError` when applying a later matching rule would exceed the
    /// configured limits, allocation fails, state-size arithmetic overflows, or
    /// an internal runtime invariant is violated.
    pub fn finish(self) -> Result<RunResult, RunError> {
        match self.run_impl::<NoTrace<'program>, Infallible>(None) {
            Ok(result) => Ok(result),
            Err(TracedRunError::Run(error)) => Err(error),
            Err(TracedRunError::Trace(error)) => match error {},
        }
    }

    pub(crate) fn run_with_borrowed_trace<F, E>(
        self,
        trace: F,
    ) -> Result<RunResult, TracedRunError<E>>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), E>,
    {
        self.run_impl(Some(trace))
    }

    fn run_impl<F, E>(mut self, mut trace: Option<F>) -> Result<RunResult, TracedRunError<E>>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), E>,
    {
        self.emit_initial_trace(&mut trace)?;

        loop {
            match self.terminal {
                ExecutionTerminal::Running => {}
                ExecutionTerminal::Stable => {
                    return Ok(RunResult::stable(
                        self.state.into_snapshot()?,
                        self.step_budget.completed_steps(),
                    ));
                }
                ExecutionTerminal::Return { step, output, .. } => {
                    return Ok(RunResult::from_return(
                        Self::materialize_return_output(output).map_err(TracedRunError::Run)?,
                        step,
                    ));
                }
            }

            let matched = match self.find_next_match().map_err(TracedRunError::Run)? {
                RuleSearch::Matched(matched) => matched,
                RuleSearch::Stable => {
                    return Ok(RunResult::stable(
                        self.state.into_snapshot()?,
                        self.step_budget.completed_steps(),
                    ));
                }
            };

            let applied = self
                .apply_matched_rule(matched)
                .map_err(TracedRunError::Run)?;
            match applied.effect {
                StepApplication::Continue => {
                    Self::emit_step_trace(
                        &mut trace,
                        applied.step,
                        applied.rule,
                        BorrowedTraceEffect::Continue {
                            state: self.state.view(),
                        },
                    )?;
                }
                StepApplication::Return(output) => {
                    Self::emit_step_trace(
                        &mut trace,
                        applied.step,
                        applied.rule,
                        BorrowedTraceEffect::Return { output },
                    )?;
                    return Ok(RunResult::from_return(
                        Self::materialize_return_output(output).map_err(TracedRunError::Run)?,
                        applied.step,
                    ));
                }
            }
        }
    }

    fn emit_initial_trace<F, E>(&self, trace: &mut Option<F>) -> Result<(), TracedRunError<E>>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), E>,
    {
        if let Some(trace) = trace.as_mut() {
            trace(BorrowedTraceEvent::Initial {
                state: self.state.view(),
            })
            .map_err(TracedRunError::Trace)?;
        }

        Ok(())
    }

    fn emit_step_trace<F, E>(
        trace: &mut Option<F>,
        step: StepCount,
        rule: &'program Rule,
        effect: BorrowedTraceEffect<'program, '_>,
    ) -> Result<(), TracedRunError<E>>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), E>,
    {
        if let Some(trace) = trace.as_mut() {
            trace(BorrowedTraceEvent::Step {
                step,
                rule: rule.view(),
                effect,
            })
            .map_err(TracedRunError::Trace)?;
        }

        Ok(())
    }
}
