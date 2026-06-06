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
use crate::runtime::once::{RuntimeRulePassState, RuntimeRuleSearch, RuntimeRuleTable};
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
pub(super) struct AttemptRunCore<E: ExecutionPolicy, A: RuleAttemptPolicy, Pass> {
    /// Current runtime byte state.
    pub(super) state: State,
    /// Reusable buffer for candidate rewrites.
    pub(super) scratch: RewriteScratch,
    /// Runtime limits and completed-step count.
    pub(super) budget: RuntimeBudgetState<E>,
    /// Rule-attempt limits and completed-attempt count.
    pub(super) attempt_budget: RuleAttemptBudgetState<A>,
    /// Rule-attempt pass that owns current target and remaining scan state.
    pub(super) runtime_rules: Pass,
}

/// Active rule-attempt runtime state without a selected pass.
pub(super) struct AttemptRunCoreParts<E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// Current runtime byte state.
    pub(super) state: State,
    /// Reusable buffer for candidate rewrites.
    pub(super) scratch: RewriteScratch,
    /// Runtime limits and completed-step count.
    pub(super) budget: RuntimeBudgetState<E>,
    /// Rule-attempt limits and completed-attempt count.
    pub(super) attempt_budget: RuleAttemptBudgetState<A>,
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
    pub(super) core: AttemptRunCore<E, A, Pass>,
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

/// Control-axis result of consuming one active ordinary run session.
pub(super) enum RunAdvance<'program, E>
where
    E: ExecutionPolicy,
{
    /// A rewrite committed and ordinary execution may continue.
    Rewritten(RunRewrite<'program, E>),
    /// A return committed and ordinary execution is terminal.
    Returned(RunReturn<'program>),
    /// No rule matched the current runtime state.
    Stable(TerminalSession<'program>),
    /// A candidate step failed before committing runtime state.
    Failed(RunFailure<'program>),
}

/// Exact committed rewrite payload for ordinary execution.
pub(super) enum RunRewrite<'program, E: ExecutionPolicy> {
    /// One reusable rewrite rule committed.
    Always {
        /// Committed step count.
        step: StepCount,
        /// Rule witness paired with the committed rewrite.
        rule: AlwaysRewriteRuleView<'program>,
        /// Continuation session after the committed rewrite.
        continuation: Session<'program, E>,
    },
    /// One once-only rewrite rule committed.
    Once {
        /// Committed step count.
        step: StepCount,
        /// Rule witness paired with the committed rewrite.
        rule: OnceRewriteRuleView<'program>,
        /// Continuation session after the committed rewrite.
        continuation: Session<'program, E>,
    },
}

/// Exact committed return payload for ordinary execution.
pub(super) enum RunReturn<'program> {
    /// One reusable return rule committed.
    Always {
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
    /// One once-only return rule committed.
    Once {
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
}

/// Runtime failure that preserves the uncommitted terminal state.
pub(super) struct RunFailure<'program> {
    /// Error that prevented commit.
    pub(super) error: RunStepError,
    /// Terminal run session preserving uncommitted state.
    pub(super) terminal: TerminalSession<'program>,
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

    /// Converts active runtime state into terminal runtime state.
    fn into_terminal(self) -> TerminalRunCore {
        TerminalRunCore {
            steps: self.budget.completed_steps(),
            state: self.state,
        }
    }
}

impl<E: ExecutionPolicy, A: RuleAttemptPolicy, Pass> AttemptRunCore<E, A, Pass> {
    /// Builds the mutable rule-attempt runtime core from a typed pass.
    fn new(runtime_rules: Pass, admitted: AdmittedRun<E>) -> Self {
        let (input, budget) = admitted.into_runtime_parts();
        let state = State::from_input(input);
        Self {
            state,
            scratch: RewriteScratch::new(),
            budget,
            attempt_budget: RuleAttemptBudgetState::new(),
            runtime_rules,
        }
    }

    /// Rebuilds the mutable rule-attempt runtime core from its typed parts.
    pub(super) fn from_parts(parts: AttemptRunCoreParts<E, A>, runtime_rules: Pass) -> Self {
        Self {
            state: parts.state,
            scratch: parts.scratch,
            budget: parts.budget,
            attempt_budget: parts.attempt_budget,
            runtime_rules,
        }
    }

    /// Splits this core into pass-independent runtime state and the selected pass.
    pub(super) fn into_parts(self) -> (AttemptRunCoreParts<E, A>, Pass) {
        let Self {
            state,
            scratch,
            budget,
            attempt_budget,
            runtime_rules,
        } = self;
        (
            AttemptRunCoreParts {
                state,
                scratch,
                budget,
                attempt_budget,
            },
            runtime_rules,
        )
    }

    /// Number of steps already committed in this core.
    pub(super) const fn completed_steps(&self) -> StepCount {
        self.budget.completed_steps()
    }

    /// Number of rule attempts already consumed in this core.
    pub(super) const fn completed_attempts(&self) -> RuleAttemptCount {
        self.attempt_budget.completed_attempts()
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

impl<E: ExecutionPolicy, A: RuleAttemptPolicy> AttemptRunCoreParts<E, A> {
    /// Reattaches a typed pass to these active runtime parts.
    pub(super) fn with_pass<Pass>(self, runtime_rules: Pass) -> AttemptRunCore<E, A, Pass> {
        AttemptRunCore::from_parts(self, runtime_rules)
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
    pub(super) fn advance_run_step(self) -> RunAdvance<'program, E> {
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
                return RunAdvance::Stable(TerminalSession {
                    program,
                    core: TerminalRunCore {
                        state,
                        steps: budget.completed_steps(),
                    },
                });
            }
        };
        let state_len = state.byte_count();
        let prepared = match prepare_matched_rule(&mut scratch, &mut budget, state_len, matched) {
            Ok(prepared) => prepared,
            Err(error) => {
                return RunAdvance::Failed(RunFailure {
                    error,
                    terminal: TerminalSession {
                        program,
                        core: TerminalRunCore {
                            state,
                            steps: budget.completed_steps(),
                        },
                    },
                });
            }
        };
        let applied = prepared.commit(&mut state, &mut scratch);
        let core = ActiveRunCore {
            state,
            scratch,
            budget,
            runtime_rules,
        };
        run_advance_from_applied(program, core, applied)
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
                RunAdvance::Rewritten(rewrite) => session = rewrite.into_continuation(),
                RunAdvance::Returned(returned) => return Ok(returned.into_result()),
                RunAdvance::Stable(terminal) => {
                    return terminal.core.into_stable_result();
                }
                RunAdvance::Failed(failure) => return Err(failure.into_finish_error()),
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
    pub(super) fn from_pass(
        program: &'program ExecutableProgram,
        admitted: AdmittedRun<E>,
        pass: Pass,
    ) -> Self {
        Self {
            program,
            core: AttemptRunCore::new(pass, admitted),
        }
    }

    /// Number of execution steps that have already completed in this run.
    pub(super) const fn completed_steps(&self) -> StepCount {
        self.core.completed_steps()
    }

    /// Number of executable rule-line attempts consumed so far.
    pub(super) const fn completed_attempts(&self) -> RuleAttemptCount {
        self.core.completed_attempts()
    }

    /// Borrow the current runtime state.
    pub(super) fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
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
                RunAdvance::Rewritten(rewrite) => {
                    session = rewrite.trace(&mut trace).map_err(TracedRunError::Trace)?;
                }
                RunAdvance::Returned(returned) => {
                    return returned.trace(&mut trace).map_err(TracedRunError::Trace);
                }
                RunAdvance::Stable(terminal) => {
                    return terminal
                        .core
                        .into_stable_result()
                        .map_err(RunError::from)
                        .map_err(TracedRunError::Run);
                }
                RunAdvance::Failed(failure) => {
                    return Err(TracedRunError::Run(RunError::from(
                        failure.into_finish_error(),
                    )));
                }
            }
        }
    }
}

impl<'program, E: ExecutionPolicy> RunRewrite<'program, E> {
    /// Consumes this rewrite payload into its continuation session.
    pub(super) fn into_continuation(self) -> Session<'program, E> {
        match self {
            Self::Always { continuation, .. } | Self::Once { continuation, .. } => continuation,
        }
    }

    /// Emits the exact borrowed trace event for this rewrite and returns its continuation.
    ///
    /// # Errors
    ///
    /// Returns `TraceError` if the trace sink rejects the emitted rewrite event.
    fn trace<F, TraceError>(self, trace: &mut F) -> Result<Session<'program, E>, TraceError>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), TraceError>,
    {
        match self {
            Self::Always {
                step,
                rule,
                continuation,
            } => {
                trace(BorrowedTraceEvent::AlwaysRewritten {
                    step,
                    rule,
                    state: continuation.state(),
                })?;
                Ok(continuation)
            }
            Self::Once {
                step,
                rule,
                continuation,
            } => {
                trace(BorrowedTraceEvent::OnceRewritten {
                    step,
                    rule,
                    state: continuation.state(),
                })?;
                Ok(continuation)
            }
        }
    }
}

impl<'program> RunReturn<'program> {
    /// Materializes this return payload as a completed run result.
    pub(super) fn into_result(self) -> RunResult {
        match self {
            Self::Always { step, output, .. } | Self::Once { step, output, .. } => {
                RunResult::from_return(output, step)
            }
        }
    }

    /// Emits the exact borrowed trace event for this return and returns the completed result.
    ///
    /// # Errors
    ///
    /// Returns `TraceError` if the trace sink rejects the emitted return event.
    fn trace<F, TraceError>(self, trace: &mut F) -> Result<RunResult, TraceError>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), TraceError>,
    {
        match self {
            Self::Always {
                step,
                rule,
                output_view,
                output,
                terminal: _,
            } => {
                trace(BorrowedTraceEvent::AlwaysReturned {
                    step,
                    rule,
                    output: output_view,
                })?;
                Ok(RunResult::from_return(output, step))
            }
            Self::Once {
                step,
                rule,
                output_view,
                output,
                terminal: _,
            } => {
                trace(BorrowedTraceEvent::OnceReturned {
                    step,
                    rule,
                    output: output_view,
                })?;
                Ok(RunResult::from_return(output, step))
            }
        }
    }
}

impl RunFailure<'_> {
    /// Converts this failed step into the finish-layer error surface.
    fn into_finish_error(self) -> RunFinishError {
        RunFinishError::from(self.error)
    }
}

/// Projects the single committed runtime action into the ordinary execution control axis.
fn run_advance_from_applied<'program, E>(
    program: &'program ExecutableProgram,
    core: ActiveRunCore<'program, E>,
    applied: AppliedRule<'program>,
) -> RunAdvance<'program, E>
where
    E: ExecutionPolicy,
{
    match applied {
        AppliedRule::AlwaysRewritten(committed) => RunAdvance::Rewritten(RunRewrite::Always {
            step: committed.step(),
            rule: committed.rule(),
            continuation: Session { program, core },
        }),
        AppliedRule::OnceRewritten(committed) => RunAdvance::Rewritten(RunRewrite::Once {
            step: committed.step(),
            rule: committed.rule(),
            continuation: Session { program, core },
        }),
        AppliedRule::AlwaysReturned(committed) => {
            let step = committed.step();
            let rule = committed.rule();
            let output_view = committed.output_view();
            let output = committed.into_output();
            RunAdvance::Returned(RunReturn::Always {
                step,
                rule,
                output_view,
                output,
                terminal: TerminalSession {
                    program,
                    core: core.into_terminal(),
                },
            })
        }
        AppliedRule::OnceReturned(committed) => {
            let step = committed.step();
            let rule = committed.rule();
            let output_view = committed.output_view();
            let output = committed.into_output();
            RunAdvance::Returned(RunReturn::Once {
                step,
                rule,
                output_view,
                output,
                terminal: TerminalSession {
                    program,
                    core: core.into_terminal(),
                },
            })
        }
    }
}
