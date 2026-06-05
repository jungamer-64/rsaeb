use crate::error::{RunError, RunFinishError, RunStartError, RunStepError, TracedRunError};
use crate::input::AdmittedRun;
use crate::inspect::{
    AlwaysReturnRuleView, AlwaysRewriteRuleView, OnceReturnRuleView, OnceRewriteRuleView,
};
use crate::limits::{RuleAttemptCount, StepCount};
use crate::policy::{ExecutionPolicy, RuleAttemptPolicy};
use crate::program::{ExecutableProgram, ReturnOutput, ReturnOutputView, RunResult};
use crate::runtime::action::{AppliedRule, prepare_matched_rule};
use crate::runtime::budget::{RuleAttemptBudgetState, RuntimeBudgetState};
use crate::runtime::once::{
    AfterMissContinuingRulePass, AfterMissFinalRulePass, FirstContinuingRulePass,
    FirstFinalRulePass, RuntimeRulePassState, RuntimeRuleSearch, RuntimeRuleTable,
    StartedRuntimeRulePass,
};
use crate::runtime::rewrite::RewriteScratch;
use crate::runtime::state::State;
use crate::trace::{BorrowedTraceEvent, RuntimeStateView};

/// Active mutable runtime state tied to one borrowed executable program.
#[derive(Debug)]
pub(super) struct ActiveRunCore<'program, E: ExecutionPolicy> {
    /// Current runtime byte state.
    pub(super) state: State,
    /// Reusable buffer for candidate rewrites.
    pub(super) scratch: RewriteScratch,
    /// Runtime limits and completed-step count.
    pub(super) budget: RuntimeBudgetState<E>,
    /// Per-run executable rules with fresh/consumed once-cell variants.
    runtime_rules: RuntimeRuleTable<'program>,
}

/// Active mutable rule-attempt runtime state tied to one pass shape.
#[derive(Debug)]
pub(super) struct AttemptRunCore<E: ExecutionPolicy, Pass> {
    /// Current runtime byte state.
    pub(super) state: State,
    /// Reusable buffer for candidate rewrites.
    pub(super) scratch: RewriteScratch,
    /// Runtime limits and completed-step count.
    pub(super) budget: RuntimeBudgetState<E>,
    /// Rule-attempt pass that owns current target and remaining scan state.
    pub(super) runtime_rules: Pass,
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
pub(super) struct Session<'program, E: ExecutionPolicy> {
    /// Borrowed parsed program.
    pub(super) program: &'program ExecutableProgram,
    /// Mutable execution state.
    pub(super) core: ActiveRunCore<'program, E>,
}

/// Terminal ordinary run session that cannot resume execution.
pub(super) struct TerminalSession<'program> {
    /// Borrowed parsed program.
    pub(super) program: &'program ExecutableProgram,
    /// Terminal runtime state retained for observation.
    pub(super) core: TerminalRunCore,
}

/// Runtime rule-attempt session parameterized by its current pass shape.
pub(super) struct AttemptSession<
    'program,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass: RuntimeRulePassState<'program>,
> {
    /// Borrowed parsed program.
    pub(super) program: &'program ExecutableProgram,
    /// Mutable execution state.
    pub(super) core: AttemptRunCore<E, Pass>,
    /// Rule-attempt budget and consumed-attempt count.
    pub(super) attempt_budget: RuleAttemptBudgetState<A>,
}

/// Newly started rule-attempt session classified by pass shape.
pub(super) enum AttemptSessionCursor<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// Started with a current rule that has successors.
    Continuing(ContinuingAttemptSession<'program, E, A>),
    /// Started with the final rule in the pass.
    Final(FinalAttemptSession<'program, E, A>),
}

/// Continuing rule-attempt session classified by miss history.
pub(super) enum ContinuingAttemptSession<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// Continuing pass that has not missed any earlier rule in this scan.
    First(AttemptSession<'program, E, A, FirstContinuingRulePass<'program>>),
    /// Continuing pass after at least one miss.
    AfterMiss(AttemptSession<'program, E, A, AfterMissContinuingRulePass<'program>>),
}

/// Final rule-attempt session classified by miss history.
pub(super) enum FinalAttemptSession<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// Final pass that has not missed any earlier rule in this scan.
    First(AttemptSession<'program, E, A, FirstFinalRulePass<'program>>),
    /// Final pass after at least one miss.
    AfterMiss(AttemptSession<'program, E, A, AfterMissFinalRulePass<'program>>),
}

/// Terminal rule-attempt state after the cursor can no longer resume.
pub(super) struct TerminalAttemptSession<'program> {
    /// Borrowed parsed program.
    pub(super) program: &'program ExecutableProgram,
    /// Terminal runtime state retained for observation.
    pub(super) core: TerminalRunCore,
    /// Rule attempts consumed before terminal state.
    pub(super) attempts: RuleAttemptCount,
}

/// Result of consuming one active ordinary run session.
pub(super) enum CoreRunTransition<'program, E>
where
    E: ExecutionPolicy,
{
    /// One reusable rewrite rule committed and execution can continue.
    AlwaysRewritten {
        /// Committed step count.
        step: StepCount,
        /// Rule witness paired with the committed rewrite.
        rule: AlwaysRewriteRuleView<'program>,
        /// Continuation session after the committed rewrite.
        continuation: Session<'program, E>,
    },
    /// One once-only rewrite rule committed and execution can continue.
    OnceRewritten {
        /// Committed step count.
        step: StepCount,
        /// Rule witness paired with the committed rewrite.
        rule: OnceRewriteRuleView<'program>,
        /// Continuation session after the committed rewrite.
        continuation: Session<'program, E>,
    },
    /// One reusable return rule committed and the run is terminal.
    AlwaysReturned {
        /// Committed return step count.
        step: StepCount,
        /// Rule witness paired with the committed return.
        rule: AlwaysReturnRuleView<'program>,
        /// Borrowed return-output view for trace callbacks.
        output_view: ReturnOutputView<'program>,
        /// Materialized return output.
        output: ReturnOutput,
        /// Terminal run session.
        terminal: TerminalSession<'program>,
    },
    /// One once-only return rule committed and the run is terminal.
    OnceReturned {
        /// Committed return step count.
        step: StepCount,
        /// Rule witness paired with the committed return.
        rule: OnceReturnRuleView<'program>,
        /// Borrowed return-output view for trace callbacks.
        output_view: ReturnOutputView<'program>,
        /// Materialized return output.
        output: ReturnOutput,
        /// Terminal run session.
        terminal: TerminalSession<'program>,
    },
    /// No rule matched the current runtime state.
    Stable {
        /// Terminal run session.
        terminal: TerminalSession<'program>,
    },
    /// A candidate step failed before committing runtime state.
    Failed {
        /// Error that prevented commit.
        error: RunStepError,
        /// Terminal run session preserving uncommitted state.
        terminal: TerminalSession<'program>,
    },
}

impl<'program, E: ExecutionPolicy> ActiveRunCore<'program, E> {
    /// Builds the mutable runtime core for one execution.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if per-run rule state allocation fails.
    fn new(
        program: &'program ExecutableProgram,
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

impl<E: ExecutionPolicy, Pass> AttemptRunCore<E, Pass> {
    /// Builds the mutable rule-attempt runtime core from a typed pass.
    fn new(runtime_rules: Pass, admitted: AdmittedRun<E>) -> Self {
        let (input, budget) = admitted.into_runtime_parts();
        let state = State::from_input(input);
        Self {
            state,
            scratch: RewriteScratch::new(),
            budget,
            runtime_rules,
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

impl<'program, E: ExecutionPolicy> Session<'program, E> {
    /// Starts a new run session for a parsed program and admitted run witness.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule state fails.
    pub(super) fn new(
        program: &'program ExecutableProgram,
        admitted: AdmittedRun<E>,
    ) -> Result<Self, RunStartError> {
        let core = ActiveRunCore::new(program, admitted)?;
        Ok(Self { program, core })
    }

    /// Borrows the parsed program.
    pub(super) const fn program(&self) -> &'program ExecutableProgram {
        self.program
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
    /// program scan with run-local once-cell state.
    ///
    /// # Errors
    ///
    /// Failed preparation returns a terminal transition that preserves
    /// uncommitted runtime state.
    pub(super) fn advance_run_step(self) -> CoreRunTransition<'program, E> {
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
        let prepared = match prepare_matched_rule(&mut scratch, &mut budget, state_len, matched) {
            Ok(prepared) => prepared,
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
        let applied = prepared.commit(&mut state, &mut scratch);
        match applied {
            AppliedRule::AlwaysRewritten(committed) => CoreRunTransition::AlwaysRewritten {
                step: committed.step(),
                rule: committed.rule(),
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
            AppliedRule::OnceRewritten(committed) => CoreRunTransition::OnceRewritten {
                step: committed.step(),
                rule: committed.rule(),
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
            AppliedRule::AlwaysReturned(committed) => {
                let step = committed.step();
                let rule = committed.rule();
                let output_view = committed.output_view();
                let output = committed.into_output();
                CoreRunTransition::AlwaysReturned {
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
                }
            }
            AppliedRule::OnceReturned(committed) => {
                let step = committed.step();
                let rule = committed.rule();
                let output_view = committed.output_view();
                let output = committed.into_output();
                CoreRunTransition::OnceReturned {
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
                }
            }
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
            match session.advance_run_step() {
                CoreRunTransition::AlwaysRewritten { continuation, .. }
                | CoreRunTransition::OnceRewritten { continuation, .. } => {
                    session = continuation;
                }
                CoreRunTransition::AlwaysReturned {
                    step,
                    output_view: _,
                    output,
                    ..
                }
                | CoreRunTransition::OnceReturned {
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

impl<'program, E: ExecutionPolicy, A: RuleAttemptPolicy, Pass: RuntimeRulePassState<'program>>
    AttemptSession<'program, E, A, Pass>
{
    /// Starts active rule-attempt execution from a typed pass.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if admitted runtime state cannot be initialized.
    fn from_pass(
        program: &'program ExecutableProgram,
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

impl<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> AttemptSessionCursor<'program, E, A> {
    /// Starts active rule-attempt execution from an executable program witness.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule-attempt state fails.
    pub(super) fn new(
        program: &'program ExecutableProgram,
        admitted: AdmittedRun<E>,
    ) -> Result<Self, RunStartError> {
        let runtime_rules = StartedRuntimeRulePass::from_program(program)?;
        Ok(started_session_from_pass(program, admitted, runtime_rules))
    }
}

/// Builds the private session classifier for a newly started rule-attempt pass.
fn started_session_from_pass<'program, E, A>(
    program: &'program ExecutableProgram,
    admitted: AdmittedRun<E>,
    runtime_rules: crate::runtime::once::StartedRuntimeRuleTable<'program>,
) -> AttemptSessionCursor<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    let runtime_rules = runtime_rules.into_pass();
    match runtime_rules {
        StartedRuntimeRulePass::Continuing(pass) => AttemptSessionCursor::Continuing(
            ContinuingAttemptSession::First(AttemptSession::from_pass(program, admitted, pass)),
        ),
        StartedRuntimeRulePass::Final(pass) => AttemptSessionCursor::Final(
            FinalAttemptSession::First(AttemptSession::from_pass(program, admitted, pass)),
        ),
    }
}

impl<'program, E: ExecutionPolicy> Session<'program, E> {
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
            match session.advance_run_step() {
                CoreRunTransition::AlwaysRewritten {
                    step,
                    rule,
                    continuation,
                } => {
                    trace(BorrowedTraceEvent::AlwaysRewritten {
                        step,
                        rule,
                        state: continuation.state(),
                    })
                    .map_err(TracedRunError::Trace)?;
                    session = continuation;
                }
                CoreRunTransition::OnceRewritten {
                    step,
                    rule,
                    continuation,
                } => {
                    trace(BorrowedTraceEvent::OnceRewritten {
                        step,
                        rule,
                        state: continuation.state(),
                    })
                    .map_err(TracedRunError::Trace)?;
                    session = continuation;
                }
                CoreRunTransition::AlwaysReturned {
                    step,
                    rule,
                    output_view,
                    output,
                    ..
                } => {
                    trace(BorrowedTraceEvent::AlwaysReturned {
                        step,
                        rule,
                        output: output_view,
                    })
                    .map_err(TracedRunError::Trace)?;
                    return Ok(RunResult::from_return(output, step));
                }
                CoreRunTransition::OnceReturned {
                    step,
                    rule,
                    output_view,
                    output,
                    ..
                } => {
                    trace(BorrowedTraceEvent::OnceReturned {
                        step,
                        rule,
                        output: output_view,
                    })
                    .map_err(TracedRunError::Trace)?;
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
}
