use crate::error::TracedRunError;
use crate::program::{RunResult, StepCount};
use crate::rule::RuleView;
use crate::trace::{BorrowedTraceEffect, BorrowedTraceEvent};

use super::execution::{ExecutionTransition, RunningExecution};

impl<'program> RunningExecution<'program> {
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
