use super::action::StepApplication;
use super::budget::StepBudget;
use super::input::{InitialStateBytes, RuntimeInput};
use super::matcher::{RuleSearch, find_next_match};
use super::once::OnceRunStates;
use super::rewrite::RewriteScratch;
use super::state::State;
use super::terminal::ExecutionTerminal;
use crate::error::RunError;
use crate::program::{Program, RunLimits, StepCount};
use crate::rule::{PayloadView, RuleView};
use crate::trace::RuntimeStateView;

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
    pub(super) program: &'program Program,
    pub(super) core: ExecutionCore,
}

#[derive(Debug)]
pub(crate) struct ExecutionCore {
    pub(super) state: State,
    pub(super) scratch: RewriteScratch,
    pub(super) step_budget: StepBudget,
    pub(super) once_states: OnceRunStates,
    pub(super) limits: RunLimits,
    pub(super) terminal: ExecutionTerminal,
}

/// Owned stateful execution of one parsed program against one runtime input.
///
/// This execution owns the parsed program as well as the mutable runtime state,
/// so hosts can store it as a single object without building a self-referential
/// structure.
#[derive(Debug)]
pub struct OwnedExecution {
    pub(super) program: Program,
    pub(super) core: ExecutionCore,
}

impl ExecutionCore {
    pub(crate) fn new(
        program: &Program,
        input: RuntimeInput<'_>,
        limits: RunLimits,
    ) -> Result<Self, RunError> {
        let input = InitialStateBytes::materialize(input, limits)?;
        let state = State::from_input(input);
        let once_states = OnceRunStates::new(program.once_slot_count())?;
        let scratch = RewriteScratch::new();

        Ok(Self {
            state,
            scratch,
            step_budget: StepBudget::new(limits.step_limit()),
            once_states,
            limits,
            terminal: ExecutionTerminal::Running,
        })
    }

    pub(super) const fn completed_steps(&self) -> StepCount {
        self.step_budget.completed_steps()
    }

    pub(super) fn state(&self) -> RuntimeStateView<'_> {
        self.state.view()
    }

    pub(super) fn step<'program>(
        &mut self,
        program: &'program Program,
    ) -> Result<ExecutionStep<'program, '_>, RunError> {
        match self.terminal {
            ExecutionTerminal::Running => {}
            ExecutionTerminal::Stable => {
                return Ok(ExecutionStep::Stable {
                    steps: self.step_budget.completed_steps(),
                    state: self.state.view(),
                });
            }
            ExecutionTerminal::Return {
                step,
                rule_position,
            } => {
                let (rule, output) = program.return_rule_at(rule_position)?;
                return Ok(ExecutionStep::Return {
                    step,
                    rule: rule.view(),
                    output,
                });
            }
        }

        let matched = match self.find_next_match(program)? {
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

    pub(super) fn find_next_match<'program>(
        &self,
        program: &'program Program,
    ) -> Result<RuleSearch<'program>, RunError> {
        find_next_match(program.rule_slice(), &self.state, &self.once_states)
    }
}

impl<'program> Execution<'program> {
    pub(crate) fn new(
        program: &'program Program,
        input: RuntimeInput<'_>,
        limits: RunLimits,
    ) -> Result<Self, RunError> {
        Ok(Self {
            program,
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
        self.core.step(self.program)
    }

    #[cfg(test)]
    pub(super) fn find_next_match(&self) -> Result<RuleSearch<'program>, RunError> {
        self.core.find_next_match(self.program)
    }
}

impl OwnedExecution {
    pub(crate) const fn new(program: Program, core: ExecutionCore) -> Self {
        Self { program, core }
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

    /// Advances this owned execution by at most one matching rule.
    ///
    /// The returned views borrow this execution, so a caller that needs to keep
    /// data after the next step must materialize it explicitly.
    ///
    /// # Errors
    ///
    /// Returns `RunError` for the same runtime failures as [`Execution::step`].
    pub fn step(&mut self) -> Result<ExecutionStep<'_, '_>, RunError> {
        self.core.step(&self.program)
    }
}
