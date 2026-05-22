//! Public stepwise run typestates.
//!
//! [`Program::start_run`](crate::Program::start_run) consumes validated
//! [`RuntimeInput`] and returns a [`RunSession`]. Calling
//! [`RunSession::step`] consumes that value and returns a [`StepTransition`],
//! so callers must handle the next state explicitly: continue with
//! [`AppliedStep::into_session`], finish a [`StableRun`], finish a
//! [`ReturnedRun`], or inspect and discard a [`FailedRun`].
//!
//! The run session is the mutable runtime engine. It owns the current state,
//! rewrite scratch, budgets, and per-run `(once)` state directly, so there is
//! no second private typestate layer and no borrowed input copy behind the
//! public API.

use crate::error::{RunError, TracedRunError};
use crate::inspect::RuleView;
use crate::program::{Program, ReturnOutputView, RunLimits, RunResult, StepCount};
use crate::rule::Rule;
use crate::runtime::RuntimeInput;
use crate::runtime::action::{
    AppliedRule, AppliedRuleEffect, apply_matched_rule, materialize_return_output,
};
use crate::runtime::budget::RuntimeBudgetState;
use crate::runtime::input::InitialStateBytes;
use crate::runtime::matcher::{RuleSearch, find_next_match};
use crate::runtime::once::OnceStateSet;
use crate::runtime::rewrite::RewriteScratch;
use crate::runtime::state::State;
use crate::trace::{BorrowedTraceEffect, BorrowedTraceEvent, RuntimeStateView};

/// Stateful run session that can still apply rules.
///
/// This type represents the only public state with a `step` method. Stable and
/// returned runs are represented by separate terminal types, so callers cannot
/// step after completion.
pub struct RunSession<'program> {
    core: RunCore<'program>,
}

#[derive(Debug, PartialEq, Eq)]
struct RunCore<'program> {
    state: State,
    scratch: RewriteScratch,
    budget: RuntimeBudgetState,
    rules: &'program [Rule],
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
    Stable(StableRun<'program>),
    /// A matched rule executed `(return)`.
    Returned(ReturnedRun<'program>),
    /// A matching rule failed before committing.
    Failed(FailedRun<'program>),
}

/// One committed non-terminal rule application.
///
/// This value lets a caller inspect the applied rule and post-step state before
/// deciding whether to continue the run.
pub struct AppliedStep<'program> {
    step: StepCount,
    rule: RuleView<'program>,
    session: RunSession<'program>,
}

/// Terminal run state reached by no matching rule.
///
/// Stable runs still own the final runtime state until the caller either
/// borrows it or materializes it with [`StableRun::into_result`].
pub struct StableRun<'program> {
    steps: StepCount,
    core: RunCore<'program>,
}

/// Terminal run state reached by `(return)`.
///
/// The output is a borrowed return output until the caller materializes the
/// terminal [`RunResult`] through [`ReturnedRun::into_result`].
#[derive(Clone, Copy)]
pub struct ReturnedRun<'program> {
    step: StepCount,
    rule: RuleView<'program>,
    output: ReturnOutputView<'program>,
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

impl core::fmt::Debug for RunSession<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RunSession")
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

impl core::fmt::Debug for StableRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("StableRun")
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

impl<'program> RunCore<'program> {
    /// Builds the mutable runtime core for one execution.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if input exceeds runtime state limits or per-run
    /// rule state allocation fails.
    fn new(
        program: &'program Program,
        input: RuntimeInput,
        limits: RunLimits,
    ) -> Result<Self, RunError> {
        let budget = RuntimeBudgetState::new(limits);
        let input = InitialStateBytes::from_runtime_input(input, budget)?;
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
        let applied = {
            let RunCore {
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
                    return StepTransition::Stable(StableRun {
                        steps,
                        core: self.core,
                    });
                }
            };

            apply_matched_rule(state, scratch, budget, matched)
        };

        let applied = match applied {
            Ok(applied) => applied,
            Err(error) => return StepTransition::Failed(FailedRun::new(error, self)),
        };

        applied.into_transition(self)
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
                StepTransition::Stable(stable) => {
                    return stable.into_result().map_err(TracedRunError::Run);
                }
                StepTransition::Returned(returned) => {
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

impl<'program> AppliedRule<'program> {
    fn into_transition(self, session: RunSession<'program>) -> StepTransition<'program> {
        match self.effect {
            AppliedRuleEffect::Continue => StepTransition::Applied(AppliedStep {
                step: self.step,
                rule: self.rule,
                session,
            }),
            AppliedRuleEffect::Return(output) => StepTransition::Returned(ReturnedRun {
                step: self.step,
                rule: self.rule,
                output,
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
    #[must_use]
    pub const fn rule(&self) -> RuleView<'program> {
        self.rule
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

impl StableRun<'_> {
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

impl<'program> ReturnedRun<'program> {
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

    /// Borrowed return output from runtime execution.
    #[must_use]
    pub const fn output(&self) -> ReturnOutputView<'program> {
        self.output
    }

    /// Materializes this returned run as a run result.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if return output materialization cannot allocate.
    pub fn into_result(self) -> Result<RunResult, RunError> {
        Ok(RunResult::from_return(
            materialize_return_output(self.output)?,
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
