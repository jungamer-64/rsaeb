use crate::error::{RunError, RunFinishError, RunStartError, TracedRunError};
use crate::input::AdmittedRun;
use crate::inspect::RuleView;
use crate::limits::{RuleAttemptCount, StepCount};
use crate::policy::{ExecutionPolicy, ParsePolicy, RuleAttemptPolicy};
use crate::program::{ExecutableProgram, RunResult};
use crate::runtime::budget::{RuleAttemptBudgetState, RuntimeBudgetState};
use crate::runtime::once::{RuntimeRulePass, RuntimeRuleSearch, RuntimeRuleTable};
use crate::runtime::rewrite::RewriteScratch;
use crate::runtime::state::State;
use crate::trace::{BorrowedTraceEffect, BorrowedTraceEvent, RuntimeStateView};

use super::advance::{
    BorrowedRunWitness, CoreAppliedRule, CoreStep, DiscardedRunWitness, RunRuleWitness,
    prepare_witnessed_run_application,
};

/// Active mutable runtime state tied to one borrowed executable program.
#[derive(Debug)]
pub(super) struct ActiveRunCore<'program, E: ExecutionPolicy> {
    /// Current runtime byte state.
    pub(super) state: State,
    /// Reusable buffer for candidate rewrites.
    pub(super) scratch: RewriteScratch,
    /// Runtime limits and completed-step count.
    pub(super) budget: RuntimeBudgetState<E>,
    /// Per-run executable rules paired with their runtime availability states.
    runtime_rules: RuntimeRuleTable<'program>,
}

/// Active mutable rule-attempt runtime state.
#[derive(Debug)]
pub(super) struct AttemptRunCore<'program, E: ExecutionPolicy> {
    /// Current runtime byte state.
    pub(super) state: State,
    /// Reusable buffer for candidate rewrites.
    pub(super) scratch: RewriteScratch,
    /// Runtime limits and completed-step count.
    pub(super) budget: RuntimeBudgetState<E>,
    /// Rule-attempt pass that owns current target and remaining scan state.
    pub(super) runtime_rules: RuntimeRulePass<'program>,
}

/// Terminal runtime state after active execution can no longer resume.
#[derive(Debug)]
pub(super) struct TerminalRunCore {
    /// Runtime byte state preserved for terminal observation.
    state: State,
    /// Steps committed before the terminal boundary.
    steps: StepCount,
}

/// Runtime session that borrows one executable program.
pub(super) struct Session<'program, P: ParsePolicy, E: ExecutionPolicy> {
    /// Borrowed parsed program.
    pub(super) program: BorrowedProgram<'program, P>,
    /// Mutable execution state.
    pub(super) core: ActiveRunCore<'program, E>,
}

/// Runtime rule-attempt session parameterized by program ownership.
pub(super) struct AttemptSession<'program, P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy>
{
    /// Borrowed parsed program.
    pub(super) program: BorrowedProgram<'program, P>,
    /// Mutable execution state.
    pub(super) core: AttemptRunCore<'program, E>,
    /// Rule-attempt budget and consumed-attempt count.
    pub(super) attempt_budget: RuleAttemptBudgetState<A>,
}

/// Terminal rule-attempt state after the cursor can no longer resume.
pub(super) struct TerminalAttemptSession<'program, P: ParsePolicy> {
    /// Borrowed parsed program.
    pub(super) program: BorrowedProgram<'program, P>,
    /// Terminal runtime state retained for observation.
    pub(super) core: TerminalRunCore,
    /// Rule attempts consumed before terminal state.
    pub(super) attempts: RuleAttemptCount,
}

/// Borrowed program owner for run-to-completion and tracing.
#[derive(Debug, Clone, Copy)]
pub(super) struct BorrowedProgram<'program, P: ParsePolicy> {
    /// Parsed program borrowed by this run.
    pub(super) program: &'program ExecutableProgram<P>,
}

impl<'program, E: ExecutionPolicy> ActiveRunCore<'program, E> {
    /// Builds the mutable runtime core for one execution.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if per-run rule state allocation fails.
    fn new<P: ParsePolicy>(
        program: &'program ExecutableProgram<P>,
        admitted: AdmittedRun<E>,
    ) -> Result<Self, RunStartError> {
        let (input, budget) = admitted.into_runtime_parts();
        let state = State::from_input(input);
        let runtime_rules = RuntimeRuleTable::from_program(program)?;
        Ok(Self {
            state,
            scratch: RewriteScratch::new(),
            budget,
            runtime_rules,
        })
    }

    /// Number of steps already committed in this core.
    pub(super) const fn completed_steps(&self) -> StepCount {
        self.budget.completed_steps()
    }

    /// Borrows the current runtime state.
    pub(super) fn state(&self) -> RuntimeStateView<'_> {
        self.state.view()
    }

    /// Converts active runtime state into terminal runtime state.
    pub(super) fn into_terminal(self) -> TerminalRunCore {
        let steps = self.completed_steps();
        TerminalRunCore {
            state: self.state,
            steps,
        }
    }

    /// Converts active runtime state into terminal runtime state with an explicit step count.
    pub(super) fn into_terminal_at(self, steps: StepCount) -> TerminalRunCore {
        TerminalRunCore {
            state: self.state,
            steps,
        }
    }
}

impl<'program, E: ExecutionPolicy> AttemptRunCore<'program, E> {
    /// Builds the mutable rule-attempt runtime core for one execution.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if per-run rule-attempt state allocation fails.
    fn new<P: ParsePolicy>(
        program: &'program ExecutableProgram<P>,
        admitted: AdmittedRun<E>,
    ) -> Result<Self, RunStartError> {
        let (input, budget) = admitted.into_runtime_parts();
        let state = State::from_input(input);
        let runtime_rules = RuntimeRulePass::from_program(program)?;
        Ok(Self {
            state,
            scratch: RewriteScratch::new(),
            budget,
            runtime_rules,
        })
    }

    /// Number of steps already committed in this core.
    pub(super) const fn completed_steps(&self) -> StepCount {
        self.budget.completed_steps()
    }

    /// Borrows the current runtime state.
    pub(super) fn state(&self) -> RuntimeStateView<'_> {
        self.state.view()
    }

    /// Converts active runtime state into terminal runtime state.
    pub(super) fn into_terminal(self) -> TerminalRunCore {
        let steps = self.completed_steps();
        TerminalRunCore {
            state: self.state,
            steps,
        }
    }

    /// Converts active runtime state into terminal runtime state with an explicit step count.
    pub(super) fn into_terminal_at(self, steps: StepCount) -> TerminalRunCore {
        TerminalRunCore {
            state: self.state,
            steps,
        }
    }
}

impl TerminalRunCore {
    /// Number of steps completed before the terminal boundary.
    pub(super) const fn completed_steps(&self) -> StepCount {
        self.steps
    }

    /// Borrows the terminal runtime state.
    pub(super) fn state(&self) -> RuntimeStateView<'_> {
        self.state.view()
    }

    /// Materializes this terminal core as a stable result.
    ///
    /// # Errors
    ///
    /// Returns `RunFinishError` if final state materialization cannot allocate.
    pub(super) fn into_stable_result(self) -> Result<RunResult, RunFinishError> {
        let output = self
            .state
            .into_snapshot()
            .map_err(RunFinishError::FinalOutput)?;
        Ok(RunResult::stable(output, self.steps))
    }
}

impl<'program, P: ParsePolicy, E: ExecutionPolicy> Session<'program, P, E> {
    /// Starts a new run session for a parsed program and admitted run witness.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule state fails.
    pub(super) fn new(
        program: BorrowedProgram<'program, P>,
        admitted: AdmittedRun<E>,
    ) -> Result<Self, RunStartError> {
        let core = ActiveRunCore::new(program.program, admitted)?;
        Ok(Self { program, core })
    }

    /// Borrows the parsed program.
    pub(super) const fn program(&self) -> &'program ExecutableProgram<P> {
        self.program.program
    }

    /// Number of execution steps that have already completed in this run.
    pub(super) const fn completed_steps(&self) -> StepCount {
        self.core.completed_steps()
    }

    /// Borrow the current runtime state.
    pub(super) fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }

    /// Advances this session by one ordinary execution step.
    ///
    /// This is the only ordinary execution path that combines the parsed
    /// program scan with the run-local rule availability state.
    ///
    /// # Errors
    ///
    /// Returns the selected witness policy's error if preparation or witness
    /// construction fails.
    pub(super) fn advance_run_step<W>(&mut self) -> Result<CoreStep<'program, W::Witness>, W::Error>
    where
        W: RunRuleWitness<'program>,
    {
        let matched = match self.core.runtime_rules.find_next_match(&self.core.state) {
            RuntimeRuleSearch::Matched(matched) => matched,
            RuntimeRuleSearch::Stable => {
                return Ok(CoreStep::Stable(self.core.budget.completed_steps()));
            }
        };
        let state_len = self.core.state.byte_count();
        let witnessed = prepare_witnessed_run_application::<_, W>(
            &mut self.core.scratch,
            &mut self.core.budget,
            state_len,
            matched,
        )?;
        let applied = witnessed.commit(&mut self.core.state, &mut self.core.scratch);
        Ok(CoreStep::Applied(applied))
    }

    /// Runs this session to completion.
    ///
    /// # Errors
    ///
    /// Returns `RunFinishError` when a later matching rule would exceed configured
    /// limits.
    pub(super) fn finish(mut self) -> Result<RunResult, RunFinishError> {
        loop {
            match self
                .advance_run_step::<DiscardedRunWitness>()
                .map_err(RunFinishError::from)?
            {
                CoreStep::Applied(CoreAppliedRule::Rewrite { .. }) => {}
                CoreStep::Applied(CoreAppliedRule::Return {
                    step,
                    output_view: _,
                    output,
                    ..
                }) => {
                    return Ok(RunResult::from_return(output, step));
                }
                CoreStep::Stable(steps) => {
                    return self.core.into_terminal_at(steps).into_stable_result();
                }
            }
        }
    }
}

impl<'program, P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy>
    AttemptSession<'program, P, E, A>
{
    /// Starts active rule-attempt execution from an executable program witness.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule state fails.
    pub(super) fn new(
        program: BorrowedProgram<'program, P>,
        admitted: AdmittedRun<E>,
    ) -> Result<Self, RunStartError> {
        let core = AttemptRunCore::new(program.program, admitted)?;
        Ok(Self {
            program,
            core,
            attempt_budget: RuleAttemptBudgetState::new(),
        })
    }

    /// Number of execution steps that have already completed in this run.
    pub(super) const fn completed_steps(&self) -> StepCount {
        self.core.completed_steps()
    }

    /// Number of executable rule-line attempts consumed so far.
    pub(super) const fn completed_attempts(&self) -> RuleAttemptCount {
        self.attempt_budget.completed_attempts()
    }

    /// Borrow the current runtime state.
    pub(super) fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }
}

impl<'program, P: ParsePolicy, E: ExecutionPolicy> Session<'program, P, E> {
    /// Runs to completion while emitting borrowed trace events.
    ///
    /// # Errors
    ///
    /// Returns `TracedRunError::Trace` if the trace sink fails. Returns
    /// `TracedRunError::Run` if runtime execution fails.
    pub(super) fn trace_events<F, TraceError>(
        mut self,
        mut trace: F,
    ) -> Result<RunResult, TracedRunError<TraceError>>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), TraceError>,
    {
        trace(BorrowedTraceEvent::Initial {
            state: self.state(),
        })
        .map_err(TracedRunError::Trace)?;

        loop {
            match self
                .advance_run_step::<BorrowedRunWitness>()
                .map_err(RunFinishError::from)
                .map_err(RunError::from)
                .map_err(TracedRunError::Run)?
            {
                CoreStep::Applied(CoreAppliedRule::Rewrite { step, rule }) => {
                    Self::emit_step_trace(
                        &mut trace,
                        step,
                        rule,
                        BorrowedTraceEffect::Continue {
                            state: self.state(),
                        },
                    )?;
                }
                CoreStep::Applied(CoreAppliedRule::Return {
                    step,
                    rule,
                    output_view,
                    output,
                }) => {
                    Self::emit_step_trace(
                        &mut trace,
                        step,
                        rule,
                        BorrowedTraceEffect::Return {
                            output: output_view,
                        },
                    )?;
                    return Ok(RunResult::from_return(output, step));
                }
                CoreStep::Stable(steps) => {
                    return self
                        .core
                        .into_terminal_at(steps)
                        .into_stable_result()
                        .map_err(RunError::from)
                        .map_err(TracedRunError::Run);
                }
            }
        }
    }

    /// Emits one borrowed step trace event.
    ///
    /// # Errors
    ///
    /// Returns `TracedRunError::Trace` if the trace sink rejects the event.
    fn emit_step_trace<F, TraceError>(
        trace: &mut F,
        step: StepCount,
        rule: RuleView<'program>,
        effect: BorrowedTraceEffect<'program, '_>,
    ) -> Result<(), TracedRunError<TraceError>>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), TraceError>,
    {
        trace(BorrowedTraceEvent::Step { step, rule, effect }).map_err(TracedRunError::Trace)
    }
}
