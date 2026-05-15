use super::budget::StepBudget;
use super::input::{InitialStateBytes, RuntimeInput};
use super::matcher::{RuleSearch, find_next_match};
use super::once::OnceRunStates;
use super::rewrite::RewriteScratch;
use super::state::State;
use crate::error::RunError;
use crate::program::{Program, RunLimits, StepCount};
use crate::trace::RuntimeStateView;

/// Stateful execution of one parsed program against one runtime input.
///
/// An execution owns the mutable runtime state, rewrite scratch buffer,
/// completed-step budget, and per-run `(once)` state for one invocation.
#[derive(Debug)]
pub struct RunningExecution<'program> {
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
}

impl ExecutionCore {
    pub(crate) fn new(
        program: &Program,
        input: &RuntimeInput,
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
        })
    }

    pub(super) const fn completed_steps(&self) -> StepCount {
        self.step_budget.completed_steps()
    }

    pub(super) fn state(&self) -> RuntimeStateView<'_> {
        self.state.view()
    }

    pub(super) fn find_next_match<'program>(
        &self,
        program: &'program Program,
    ) -> Result<RuleSearch<'program>, RunError> {
        find_next_match(program.rule_slice(), &self.state, &self.once_states)
    }
}

impl<'program> RunningExecution<'program> {
    pub(crate) fn new(
        program: &'program Program,
        input: &RuntimeInput,
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

    #[cfg(test)]
    pub(super) fn find_next_match(&self) -> Result<RuleSearch<'program>, RunError> {
        self.core.find_next_match(self.program)
    }
}
