use core::convert::Infallible;

use super::budget::StepBudget;
use super::input::{InitialStateBytes, RuntimeInput};
use super::matcher::{MatchedRule, RuleSearch, find_next_match};
use super::once::OnceRunStates;
use super::rewrite::RewriteScratch;
use super::state::{MatchedStateSpan, State};
use crate::allocation::AllocationContext;
use crate::bytes::ReturnOutputByteCount;
use crate::error::{LimitError, RunError, TracedRunError};
use crate::program::{Program, ReturnOutput, RunLimits, RunResult, StepCount};
use crate::rule::{Action, PayloadView, Rule, RuleView};
use crate::trace::{BorrowedTraceEffect, BorrowedTraceEvent, RuntimeStateView};

type NoTrace<'program> = for<'run> fn(BorrowedTraceEvent<'program, 'run>) -> Result<(), Infallible>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepApplication<'program> {
    Continue,
    Return(PayloadView<'program>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AppliedRule<'program> {
    step: StepCount,
    rule: &'program Rule,
    effect: StepApplication<'program>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutionTerminal<'program> {
    Running,
    Stable,
    Return {
        step: StepCount,
        rule: &'program Rule,
        output: PayloadView<'program>,
    },
}

/// Result of asking an [`Execution`] to advance by one rule application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionStep<'program, 'run> {
    /// One ordinary rewrite rule was applied and execution can be stepped again.
    Applied {
        /// One-based applied step count.
        step: StepCount,
        /// Structured view of the applied rule.
        rule: RuleView<'program>,
        /// Borrowed runtime state after the applied rewrite step.
        state: RuntimeStateView<'run>,
    },
    /// No rule matched the final runtime state.
    Stable {
        /// Number of rewrite steps applied before reaching the stable state.
        steps: StepCount,
        /// Borrowed final runtime state.
        state: RuntimeStateView<'run>,
    },
    /// A matched rule executed `(return)`.
    Return {
        /// One-based applied step count for the return rule.
        step: StepCount,
        /// Structured view of the return rule.
        rule: RuleView<'program>,
        /// Borrowed return payload from the parsed program.
        output: PayloadView<'program>,
    },
}

/// Stateful execution of one parsed program against one runtime input.
///
/// An execution owns the mutable runtime state, rewrite scratch buffer,
/// completed-step budget, and per-run `(once)` state for one invocation.
#[derive(Debug)]
pub struct Execution<'program> {
    program: &'program Program,
    pub(super) state: State,
    scratch: RewriteScratch,
    step_budget: StepBudget,
    once_states: OnceRunStates,
    limits: RunLimits,
    terminal: ExecutionTerminal<'program>,
}

impl<'program> Execution<'program> {
    pub(crate) fn new(
        program: &'program Program,
        input: RuntimeInput<'_>,
        limits: RunLimits,
    ) -> Result<Self, RunError> {
        let input = InitialStateBytes::materialize(input, limits)?;
        let state = State::from_input(input);
        let once_states = OnceRunStates::new(program.once_slot_count())?;
        let scratch = RewriteScratch::new();

        Ok(Self {
            program,
            state,
            scratch,
            step_budget: StepBudget::new(limits.step_limit()),
            once_states,
            limits,
            terminal: ExecutionTerminal::Running,
        })
    }

    /// Number of rewrite steps that have already completed in this execution.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.step_budget.completed_steps()
    }

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

    /// Advances this execution by at most one matching rule.
    ///
    /// Returns [`ExecutionStep::Applied`] after one ordinary rewrite step.
    /// Returns [`ExecutionStep::Stable`] when no rule matches.
    /// Returns [`ExecutionStep::Return`] when the next matching rule executes
    /// `(return)`.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when applying the next matching rule would exceed the
    /// configured limits, allocation fails, state-size arithmetic overflows, or
    /// an internal runtime invariant is violated. On error, no rewrite step is
    /// completed: the runtime state, `(once)`
    /// state, and completed-step count remain unchanged.
    pub fn step(&mut self) -> Result<ExecutionStep<'program, '_>, RunError> {
        match self.terminal {
            ExecutionTerminal::Running => {}
            ExecutionTerminal::Stable => {
                return Ok(ExecutionStep::Stable {
                    steps: self.step_budget.completed_steps(),
                    state: self.state.view(),
                });
            }
            ExecutionTerminal::Return { step, rule, output } => {
                return Ok(ExecutionStep::Return {
                    step,
                    rule: rule.view(),
                    output,
                });
            }
        }

        let matched = match self.find_next_match()? {
            RuleSearch::Matched(matched) => matched,
            RuleSearch::Stable => {
                self.terminal = ExecutionTerminal::Stable;
                return Ok(ExecutionStep::Stable {
                    steps: self.step_budget.completed_steps(),
                    state: self.state.view(),
                });
            }
        };

        let applied = self.apply_matched_rule(matched)?;
        match applied.effect {
            StepApplication::Continue => Ok(ExecutionStep::Applied {
                step: applied.step,
                rule: applied.rule.view(),
                state: self.state.view(),
            }),
            StepApplication::Return(output) => Ok(ExecutionStep::Return {
                step: applied.step,
                rule: applied.rule.view(),
                output,
            }),
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

    pub(super) fn find_next_match(&self) -> Result<RuleSearch<'program>, RunError> {
        find_next_match(self.program.rule_slice(), &self.state, &self.once_states)
    }

    fn apply_matched_rule(
        &mut self,
        matched: MatchedRule<'program>,
    ) -> Result<AppliedRule<'program>, RunError> {
        let permit = self
            .step_budget
            .reserve_next_step(self.state.byte_count())
            .map_err(RunError::from)?;

        let effect = self.apply_action_to_scratch(matched.state_match, matched.rule.action())?;
        self.once_states
            .consume(matched.schedule)
            .map_err(RunError::from)?;

        let step = self.step_budget.commit(permit);

        match effect {
            StepApplication::Continue => {
                self.state.swap_with_scratch(&mut self.scratch);
                Ok(AppliedRule {
                    step,
                    rule: matched.rule,
                    effect: StepApplication::Continue,
                })
            }
            StepApplication::Return(output) => {
                self.terminal = ExecutionTerminal::Return {
                    step,
                    rule: matched.rule,
                    output,
                };
                Ok(AppliedRule {
                    step,
                    rule: matched.rule,
                    effect: StepApplication::Return(output),
                })
            }
        }
    }

    fn materialize_return_output(output: PayloadView<'program>) -> Result<ReturnOutput, RunError> {
        Ok(ReturnOutput::from_vec(
            output.to_vec_with_context(AllocationContext::ReturnOutput)?,
        ))
    }

    fn apply_action_to_scratch(
        &mut self,
        state_match: MatchedStateSpan,
        action: &'program Action,
    ) -> Result<StepApplication<'program>, RunError> {
        match action {
            Action::Replace(rhs) => {
                self.state
                    .replace_at_into(state_match, rhs, &mut self.scratch, self.limits)?;
                Ok(StepApplication::Continue)
            }
            Action::MoveStart(rhs) => {
                self.state
                    .move_start_at_into(state_match, rhs, &mut self.scratch, self.limits)?;
                Ok(StepApplication::Continue)
            }
            Action::MoveEnd(rhs) => {
                self.state
                    .move_end_at_into(state_match, rhs, &mut self.scratch, self.limits)?;
                Ok(StepApplication::Continue)
            }
            Action::Return(output) => {
                let output_len = ReturnOutputByteCount::new(output.len());
                if output_len.get() > self.limits.return_byte_limit().get() {
                    return Err(LimitError::return_output(
                        self.limits.return_byte_limit(),
                        output_len,
                    )
                    .into());
                }

                Ok(StepApplication::Return(PayloadView::new(output)))
            }
        }
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
