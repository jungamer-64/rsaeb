use crate::error::{RunError, RunFinishError, RunStartError, TracedRunError};
use crate::input::AdmittedRun;
use crate::inspect::RuleView;
use crate::limits::{RuleAttemptCount, StepCount};
use crate::policy::{ExecutionPolicy, ParsePolicy, RuleAttemptPolicy};
use crate::program::{ExecutableProgram, ReturnOutput, ReturnOutputView, RunResult};
use crate::runtime::budget::{RuleAttemptBudgetState, RuntimeBudgetState};
use crate::runtime::once::{
    AfterMissContinuingRulePass, AfterMissFinalRulePass, FirstContinuingRulePass,
    FirstFinalRulePass, RuntimeRuleSearch, RuntimeRuleTable, StartedRuntimeRulePass,
};
use crate::runtime::rewrite::RewriteScratch;
use crate::runtime::state::State;
use crate::trace::{BorrowedTraceEffect, BorrowedTraceEvent, RuntimeStateView};

use super::advance::{
    BorrowedRunWitness, CoreAppliedRule, DiscardedRunWitness, RunRuleWitness,
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

/// Active mutable rule-attempt runtime state tied to one pass shape.
#[derive(Debug)]
pub(super) struct AttemptRunCore<'program, E: ExecutionPolicy, Pass> {
    /// Current runtime byte state.
    pub(super) state: State,
    /// Reusable buffer for candidate rewrites.
    pub(super) scratch: RewriteScratch,
    /// Runtime limits and completed-step count.
    pub(super) budget: RuntimeBudgetState<E>,
    /// Rule-attempt pass that owns current target and remaining scan state.
    pub(super) runtime_rules: Pass,
    /// Compile-time link between this pass and the parsed program lifetime.
    pub(super) program: core::marker::PhantomData<&'program ()>,
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

/// Terminal ordinary run session that cannot resume execution.
pub(super) struct TerminalSession<'program, P: ParsePolicy> {
    /// Borrowed parsed program.
    pub(super) program: BorrowedProgram<'program, P>,
    /// Terminal runtime state retained for observation.
    pub(super) core: TerminalRunCore,
}

/// Runtime rule-attempt session parameterized by its current pass shape.
pub(super) struct AttemptSession<
    'program,
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass,
> {
    /// Borrowed parsed program.
    pub(super) program: BorrowedProgram<'program, P>,
    /// Mutable execution state.
    pub(super) core: AttemptRunCore<'program, E, Pass>,
    /// Rule-attempt budget and consumed-attempt count.
    pub(super) attempt_budget: RuleAttemptBudgetState<A>,
}

/// Newly started rule-attempt session classified by pass shape.
pub(super) enum AttemptSessionCursor<
    'program,
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
> {
    /// Started with a current rule that has successors.
    Continuing(ContinuingAttemptSession<'program, P, E, A>),
    /// Started with the final rule in the pass.
    Final(FinalAttemptSession<'program, P, E, A>),
}

/// Continuing rule-attempt session classified by miss history.
pub(super) enum ContinuingAttemptSession<
    'program,
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
> {
    /// Continuing pass that has not missed any earlier rule in this scan.
    First(AttemptSession<'program, P, E, A, FirstContinuingRulePass<'program>>),
    /// Continuing pass after at least one miss.
    AfterMiss(AttemptSession<'program, P, E, A, AfterMissContinuingRulePass<'program>>),
}

/// Final rule-attempt session classified by miss history.
pub(super) enum FinalAttemptSession<
    'program,
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
> {
    /// Final pass that has not missed any earlier rule in this scan.
    First(AttemptSession<'program, P, E, A, FirstFinalRulePass<'program>>),
    /// Final pass after at least one miss.
    AfterMiss(AttemptSession<'program, P, E, A, AfterMissFinalRulePass<'program>>),
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

/// Result of consuming one active ordinary run session.
pub(super) enum CoreRunTransition<'program, P, E, RuleWitness, StepError>
where
    P: ParsePolicy,
    E: ExecutionPolicy,
{
    /// One rewrite rule committed and execution can continue.
    Applied {
        /// Committed step count.
        step: StepCount,
        /// Rule witness paired with the committed rewrite.
        rule: RuleWitness,
        /// Continuation session after the committed rewrite.
        continuation: Session<'program, P, E>,
    },
    /// One return rule committed and the run is terminal.
    Returned {
        /// Committed return step count.
        step: StepCount,
        /// Rule witness paired with the committed return.
        rule: RuleWitness,
        /// Borrowed return-output view for trace callbacks.
        output_view: ReturnOutputView<'program>,
        /// Materialized return output.
        output: ReturnOutput,
        /// Terminal run session.
        terminal: TerminalSession<'program, P>,
    },
    /// No rule matched the current runtime state.
    Stable {
        /// Terminal run session.
        terminal: TerminalSession<'program, P>,
    },
    /// A candidate step failed before committing runtime state.
    Failed {
        /// Error that prevented commit.
        error: StepError,
        /// Terminal run session preserving uncommitted state.
        terminal: TerminalSession<'program, P>,
    },
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
}

impl<'program, E: ExecutionPolicy, Pass> AttemptRunCore<'program, E, Pass> {
    /// Builds the mutable rule-attempt runtime core from a typed pass.
    fn new(runtime_rules: Pass, admitted: AdmittedRun<E>) -> Self {
        let (input, budget) = admitted.into_runtime_parts();
        let state = State::from_input(input);
        Self {
            state,
            scratch: RewriteScratch::new(),
            budget,
            runtime_rules,
            program: core::marker::PhantomData,
        }
    }

    /// Rebuilds the mutable rule-attempt runtime core from its typed parts.
    pub(super) fn from_parts(
        state: State,
        scratch: RewriteScratch,
        budget: RuntimeBudgetState<E>,
        runtime_rules: Pass,
    ) -> Self {
        Self {
            state,
            scratch,
            budget,
            runtime_rules,
            program: core::marker::PhantomData,
        }
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

    /// Consumes this session and advances it by one ordinary execution step.
    ///
    /// This is the only ordinary execution path that combines the parsed
    /// program scan with the run-local rule availability state.
    ///
    /// # Errors
    ///
    /// Failed preparation returns a terminal transition that preserves
    /// uncommitted runtime state.
    pub(super) fn advance_run_step<W>(
        self,
    ) -> CoreRunTransition<'program, P, E, W::Witness, W::Error>
    where
        W: RunRuleWitness<'program>,
    {
        let Session { program, core } = self;
        let ActiveRunCore {
            mut state,
            mut scratch,
            mut budget,
            mut runtime_rules,
        } = core;

        let matched = match runtime_rules.find_next_match(&state) {
            RuntimeRuleSearch::Matched(matched) => matched,
            RuntimeRuleSearch::Stable => {
                return CoreRunTransition::Stable {
                    terminal: TerminalSession {
                        program,
                        core: TerminalRunCore {
                            state,
                            steps: budget.completed_steps(),
                        },
                    },
                };
            }
        };
        let state_len = state.byte_count();
        let witnessed = match prepare_witnessed_run_application::<_, W>(
            &mut scratch,
            &mut budget,
            state_len,
            matched,
        ) {
            Ok(witnessed) => witnessed,
            Err(error) => {
                return CoreRunTransition::Failed {
                    error,
                    terminal: TerminalSession {
                        program,
                        core: TerminalRunCore {
                            state,
                            steps: budget.completed_steps(),
                        },
                    },
                };
            }
        };
        let applied = witnessed.commit(&mut state, &mut scratch);
        match applied {
            CoreAppliedRule::Continued { step, rule } => CoreRunTransition::Applied {
                step,
                rule,
                continuation: Session {
                    program,
                    core: ActiveRunCore {
                        state,
                        scratch,
                        budget,
                        runtime_rules,
                    },
                },
            },
            CoreAppliedRule::Terminal {
                step,
                rule,
                output_view,
                output,
            } => CoreRunTransition::Returned {
                step,
                rule,
                output_view,
                output,
                terminal: TerminalSession {
                    program,
                    core: TerminalRunCore {
                        state,
                        steps: budget.completed_steps(),
                    },
                },
            },
        }
    }

    /// Runs this session to completion.
    ///
    /// # Errors
    ///
    /// Returns `RunFinishError` when a later matching rule would exceed configured
    /// limits.
    pub(super) fn finish(self) -> Result<RunResult, RunFinishError> {
        let mut session = self;
        loop {
            match session.advance_run_step::<DiscardedRunWitness>() {
                CoreRunTransition::Applied { continuation, .. } => {
                    session = continuation;
                }
                CoreRunTransition::Returned {
                    step,
                    output_view: _,
                    output,
                    ..
                } => {
                    return Ok(RunResult::from_return(output, step));
                }
                CoreRunTransition::Stable { terminal } => {
                    return terminal.core.into_stable_result();
                }
                CoreRunTransition::Failed { error, .. } => {
                    return Err(RunFinishError::from(error));
                }
            }
        }
    }
}

impl<'program, P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy, Pass>
    AttemptSession<'program, P, E, A, Pass>
{
    /// Starts active rule-attempt execution from a typed pass.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if admitted runtime state cannot be initialized.
    fn from_pass(
        program: BorrowedProgram<'program, P>,
        admitted: AdmittedRun<E>,
        pass: Pass,
    ) -> Self {
        Self {
            program,
            core: AttemptRunCore::new(pass, admitted),
            attempt_budget: RuleAttemptBudgetState::new(),
        }
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

impl<'program, P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy>
    AttemptSessionCursor<'program, P, E, A>
{
    /// Starts active rule-attempt execution from an executable program witness.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule-attempt state fails.
    pub(super) fn new(
        program: BorrowedProgram<'program, P>,
        admitted: AdmittedRun<E>,
    ) -> Result<Self, RunStartError> {
        let runtime_rules = StartedRuntimeRulePass::from_program(program.program)?;
        Ok(started_session_from_pass(program, admitted, runtime_rules))
    }
}

/// Builds the private session classifier for a newly started rule-attempt pass.
fn started_session_from_pass<'program, P, E, A>(
    program: BorrowedProgram<'program, P>,
    admitted: AdmittedRun<E>,
    runtime_rules: StartedRuntimeRulePass<'program>,
) -> AttemptSessionCursor<'program, P, E, A>
where
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    match runtime_rules {
        StartedRuntimeRulePass::Continuing(pass) => AttemptSessionCursor::Continuing(
            ContinuingAttemptSession::First(AttemptSession::from_pass(program, admitted, pass)),
        ),
        StartedRuntimeRulePass::Final(pass) => AttemptSessionCursor::Final(
            FinalAttemptSession::First(AttemptSession::from_pass(program, admitted, pass)),
        ),
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
        self,
        mut trace: F,
    ) -> Result<RunResult, TracedRunError<TraceError>>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), TraceError>,
    {
        trace(BorrowedTraceEvent::Initial {
            state: self.state(),
        })
        .map_err(TracedRunError::Trace)?;

        let mut session = self;
        loop {
            match session.advance_run_step::<BorrowedRunWitness>() {
                CoreRunTransition::Applied {
                    step,
                    rule,
                    continuation,
                } => {
                    Self::emit_step_trace(
                        &mut trace,
                        step,
                        rule,
                        BorrowedTraceEffect::Continue {
                            state: continuation.state(),
                        },
                    )?;
                    session = continuation;
                }
                CoreRunTransition::Returned {
                    step,
                    rule,
                    output_view,
                    output,
                    ..
                } => {
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
                CoreRunTransition::Stable { terminal } => {
                    return terminal
                        .core
                        .into_stable_result()
                        .map_err(RunError::from)
                        .map_err(TracedRunError::Run);
                }
                CoreRunTransition::Failed { error, .. } => {
                    return Err(TracedRunError::Run(RunError::from(RunFinishError::from(
                        error,
                    ))));
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
