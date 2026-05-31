use crate::error::{
    OwnedRuleAttemptStepError, OwnedRunStepError, RuleAttemptStepError, RunError, RunFinishError,
    RunStartError, RunStepError, TracedRunError,
};
use crate::input::RunSeed;
use crate::inspect::RuleView;
use crate::limits::{RuleAttemptCount, StepCount};
use crate::policy::{ExecutionPolicy, ParsePolicy, RuleAttemptPolicy};
use crate::program::{
    Program, ReturnOutput, RuleAttemptTargetSelection, RuleCursor, RuleCursorAfterMiss, RunResult,
};
use crate::runtime::action::{AppliedRule, PreparedRuleApplication, prepare_matched_rule};
use crate::runtime::budget::{RuleAttemptBudgetState, RuntimeBudgetState};
use crate::runtime::matcher::{
    RuleAttempt, RuleSearch, attempt_rule, find_next_match, runtime_rule_for_target,
};
use crate::runtime::once::OnceStateSet;
use crate::runtime::rewrite::RewriteScratch;
use crate::runtime::state::State;
use crate::trace::{BorrowedTraceEffect, BorrowedTraceEvent, RuntimeStateView};

use super::{OwnedRuleWitness, RuleAttemptStableReason, RuleMiss};

/// Mutable runtime state independent of program ownership mode.
#[derive(Debug)]
pub(super) struct RunCore<E: ExecutionPolicy> {
    /// Current runtime byte state.
    state: State,
    /// Reusable buffer for candidate rewrites.
    scratch: RewriteScratch,
    /// Runtime limits and completed-step count.
    budget: RuntimeBudgetState<E>,
    /// Per-run consumption state for `(once)` rules.
    once_states: OnceStateSet,
}

/// Runtime session parameterized by program ownership.
pub(super) struct Session<P, E: ExecutionPolicy> {
    /// Borrowed or owned parsed program.
    pub(super) program: P,
    /// Mutable execution state.
    pub(super) core: RunCore<E>,
}

/// Runtime rule-attempt session parameterized by program ownership.
pub(super) struct AttemptSession<P, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// Borrowed or owned parsed program.
    pub(super) program: P,
    /// Mutable execution state.
    pub(super) core: RunCore<E>,
    /// Next executable rule line to evaluate.
    pub(super) cursor: RuleCursor,
    /// Rule-attempt budget and consumed-attempt count.
    pub(super) attempt_budget: RuleAttemptBudgetState<A>,
}

/// All data needed to commit one non-applying rule attempt.
struct MissCommit<'attempt, E: ExecutionPolicy, A: RuleAttemptPolicy, RuleWitness> {
    /// Cursor to advance when the miss is not the final executable rule.
    cursor: &'attempt mut RuleCursor,
    /// Rule-attempt budget after the miss has been committed.
    attempt_budget: &'attempt RuleAttemptBudgetState<A>,
    /// Runtime core observed by the attempted rule.
    core: &'attempt RunCore<E>,
    /// Committed attempt count assigned to this miss.
    attempt: RuleAttemptCount,
    /// Active cursor that selected the missed rule.
    after_miss: RuleCursorAfterMiss,
    /// Non-applying rule selected by the current cursor.
    miss: RuleMiss<RuleWitness>,
}

/// Mutable rule-attempt state needed to consume one executable rule line.
struct RuleAttemptContext<
    'attempt,
    'program,
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
> {
    /// Parsed program that owns the rule selected by the cursor.
    program: &'program Program<P>,
    /// Mutable runtime state observed by the attempted rule.
    core: &'attempt mut RunCore<E>,
    /// Next executable rule line to evaluate.
    cursor: &'attempt mut RuleCursor,
    /// Rule-attempt budget and consumed-attempt count.
    attempt_budget: &'attempt mut RuleAttemptBudgetState<A>,
}

/// Rule application after the public witness has been created but before runtime side effects commit.
struct WitnessedApplication<'program, 'once, 'budget, E: ExecutionPolicy, RuleWitness> {
    /// Failure-prone runtime preparation that must still be committed linearly.
    prepared: PreparedRuleApplication<'program, 'once, 'budget, E>,
    /// Public rule witness created before mutation commits.
    witness: RuleWitness,
}

/// Program ownership shape used by the internal runtime session.
pub(super) trait ProgramOwner {
    /// Parser policy selected for this owner.
    type Policy: ParsePolicy;
    /// Borrows the parsed program.
    fn program(&self) -> &Program<Self::Policy>;
}

/// Borrowed program owner for run-to-completion and tracing.
#[derive(Debug, Clone, Copy)]
pub(super) struct BorrowedProgram<'program, P: ParsePolicy> {
    /// Parsed program borrowed by this run.
    pub(super) program: &'program Program<P>,
}

/// Owned program owner for public stepwise execution.
#[derive(Debug)]
pub(super) struct OwnedProgram<P: ParsePolicy> {
    /// Parsed program owned by the public run session.
    pub(super) program: Program<P>,
}

/// Internal non-error result of one core step attempt.
pub(super) enum CoreStep<RuleWitness> {
    /// A rule committed and may have terminal side effects.
    Applied(CoreAppliedRule<RuleWitness>),
    /// No rule matched the current runtime state.
    Stable(StepCount),
}

/// Internal runtime step retaining committed parsed-rule borrows.
enum RuntimeStep<'program> {
    /// A rule committed and may have terminal side effects.
    Applied(AppliedRule<'program>),
    /// No rule matched the current runtime state.
    Stable(StepCount),
}

/// Internal committed application paired with its public rule witness.
pub(super) enum CoreAppliedRule<RuleWitness> {
    /// One rewrite rule committed and execution may continue.
    Rewrite {
        /// Committed step count.
        step: StepCount,
        /// Rule witness selected before runtime side effects committed.
        rule: RuleWitness,
    },
    /// One return rule committed and execution is terminal.
    Return {
        /// Committed step count.
        step: StepCount,
        /// Rule witness selected before runtime side effects committed.
        rule: RuleWitness,
        /// Materialized return output.
        output: ReturnOutput,
    },
}

/// Internal non-error result of one rule-attempt step.
pub(super) enum CoreRuleAttempt<RuleWitness> {
    /// A rule line was consumed without applying.
    Missed {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Non-applying rule information.
        miss: RuleMiss<RuleWitness>,
    },
    /// A rule committed and may have terminal side effects.
    Applied {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Applied rule effect.
        applied: CoreAppliedRule<RuleWitness>,
    },
    /// No rule in the current pass matched the current runtime state.
    Stable {
        /// Rule attempts consumed before stability.
        attempts: RuleAttemptCount,
        /// Rewrite steps committed before stability.
        steps: StepCount,
        /// Why the rule-attempt pass reached stability.
        stable_reason: RuleAttemptStableReason<RuleWitness>,
    },
}

impl<RuleWitness> CoreAppliedRule<RuleWitness> {
    /// Combines a committed runtime application with its pre-commit rule witness.
    fn from_applied_rule(applied: AppliedRule<'_>, rule: RuleWitness) -> Self {
        match applied {
            AppliedRule::Rewrite(committed) => Self::Rewrite {
                step: committed.step(),
                rule,
            },
            AppliedRule::Return(committed) => Self::Return {
                step: committed.step(),
                rule,
                output: committed.into_output(),
            },
        }
    }
}

impl<'program, 'once, 'budget, E: ExecutionPolicy, RuleWitness>
    WitnessedApplication<'program, 'once, 'budget, E, RuleWitness>
{
    /// Pairs a prepared application with its public rule witness before commit.
    ///
    /// # Errors
    ///
    /// Returns `Error` if witness creation cannot retain the selected rule.
    fn new<Error>(
        prepared: PreparedRuleApplication<'program, 'once, 'budget, E>,
        make_witness: impl FnOnce(RuleView<'program>) -> Result<RuleWitness, Error>,
    ) -> Result<Self, Error> {
        let witness = make_witness(RuleView::new(prepared.rule()))?;
        Ok(Self { prepared, witness })
    }

    /// Commits prepared runtime side effects and publishes the paired witness.
    fn commit(
        self,
        state: &mut State,
        scratch: &mut RewriteScratch,
    ) -> CoreAppliedRule<RuleWitness> {
        let applied = self.prepared.commit(state, scratch);
        CoreAppliedRule::from_applied_rule(applied, self.witness)
    }
}

impl<P: ParsePolicy> ProgramOwner for BorrowedProgram<'_, P> {
    type Policy = P;

    fn program(&self) -> &Program<P> {
        self.program
    }
}

impl<P: ParsePolicy> ProgramOwner for OwnedProgram<P> {
    type Policy = P;

    fn program(&self) -> &Program<P> {
        &self.program
    }
}

impl<E: ExecutionPolicy> RunCore<E> {
    /// Builds the mutable runtime core for one execution.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if per-run rule state allocation fails.
    fn new<P: ParsePolicy>(program: &Program<P>, seed: RunSeed<E>) -> Result<Self, RunStartError> {
        let (input, budget) = seed.into_runtime_parts();
        let state = State::from_input(input);
        let once_states = OnceStateSet::new(program.once_rule_count())?;
        Ok(Self {
            state,
            scratch: RewriteScratch::new(),
            budget,
            once_states,
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

    /// Materializes a stable terminal result.
    ///
    /// # Errors
    ///
    /// Returns `RunFinishError` if final state materialization cannot allocate.
    pub(super) fn into_stable_result(self, steps: StepCount) -> Result<RunResult, RunFinishError> {
        let output = self
            .state
            .into_snapshot()
            .map_err(RunFinishError::FinalOutput)?;
        Ok(RunResult::stable(output, steps))
    }

    /// Advances the mutable runtime core against the supplied immutable program.
    ///
    /// # Errors
    ///
    /// Returns `RunStepError` if applying the matched rule exceeds limits or allocation fails.
    fn step_runtime<'program, P: ParsePolicy>(
        &mut self,
        program: &'program Program<P>,
    ) -> Result<RuntimeStep<'program>, RunStepError> {
        let matched = match find_next_match(program.rule_scan(), &mut self.once_states, &self.state)
        {
            RuleSearch::Matched(matched) => matched,
            RuleSearch::Stable => {
                return Ok(RuntimeStep::Stable(self.budget.completed_steps()));
            }
        };

        let state_len = self.state.byte_count();
        let prepared =
            prepare_matched_rule(&mut self.scratch, &mut self.budget, state_len, matched)?;
        let applied = prepared.commit(&mut self.state, &mut self.scratch);
        Ok(RuntimeStep::Applied(applied))
    }

    /// Advances the mutable runtime core against the supplied immutable program.
    ///
    /// # Errors
    ///
    /// Returns `RunStepError` if applying the matched rule exceeds limits or allocation fails.
    fn step<P: ParsePolicy>(&mut self, program: &Program<P>) -> Result<CoreStep<()>, RunStepError> {
        let applied = match self.step_runtime(program)? {
            RuntimeStep::Applied(applied) => applied,
            RuntimeStep::Stable(steps) => return Ok(CoreStep::Stable(steps)),
        };
        Ok(CoreStep::Applied(CoreAppliedRule::from_applied_rule(
            applied,
            (),
        )))
    }
}

impl<P: ProgramOwner, E: ExecutionPolicy> Session<P, E> {
    /// Starts a new run session for a parsed program and admitted run seed.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule state fails.
    pub(super) fn new(program: P, seed: RunSeed<E>) -> Result<Self, RunStartError> {
        let core = RunCore::new(program.program(), seed)?;
        Ok(Self { program, core })
    }

    /// Borrows the parsed program.
    pub(super) fn program(&self) -> &Program<P::Policy> {
        self.program.program()
    }

    /// Number of execution steps that have already completed in this run.
    pub(super) const fn completed_steps(&self) -> StepCount {
        self.core.completed_steps()
    }

    /// Borrow the current runtime state.
    pub(super) fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }

    /// Advances this run by exactly one matching rule when possible.
    ///
    /// # Errors
    ///
    /// Returns `RunStepError` if rule matching or rule application fails.
    pub(super) fn step(&mut self) -> Result<CoreStep<()>, RunStepError> {
        self.core.step(self.program.program())
    }

    /// Runs this session to completion.
    ///
    /// # Errors
    ///
    /// Returns `RunFinishError` when a later matching rule would exceed configured
    /// limits.
    pub(super) fn finish(mut self) -> Result<RunResult, RunFinishError> {
        loop {
            match self.step().map_err(RunFinishError::from)? {
                CoreStep::Applied(CoreAppliedRule::Rewrite { .. }) => {}
                CoreStep::Applied(CoreAppliedRule::Return { step, output, .. }) => {
                    return Ok(RunResult::from_return(output, step));
                }
                CoreStep::Stable(steps) => return self.core.into_stable_result(steps),
            }
        }
    }
}

impl<P: ProgramOwner, E: ExecutionPolicy, A: RuleAttemptPolicy> AttemptSession<P, E, A> {
    /// Starts a new rule-attempt session for a parsed program and admitted run seed.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule state fails.
    pub(super) fn new(program: P, seed: RunSeed<E>) -> Result<Self, RunStartError> {
        let cursor = program.program().rule_attempt_cursor();
        let core = RunCore::new(program.program(), seed)?;
        Ok(Self {
            program,
            core,
            cursor,
            attempt_budget: RuleAttemptBudgetState::new(),
        })
    }

    /// Borrows the parsed program.
    pub(super) fn program(&self) -> &Program<P::Policy> {
        self.program.program()
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

impl<'program, P: ParsePolicy, E: ExecutionPolicy> Session<BorrowedProgram<'program, P>, E> {
    /// Advances this run by one matching rule with borrowed rule witnesses.
    ///
    /// # Errors
    ///
    /// Returns `RunStepError` if matching, preparation, or application fails.
    pub(super) fn step_borrowed(&mut self) -> Result<CoreStep<RuleView<'program>>, RunStepError> {
        step_with_witness(&mut self.core, self.program.program, Ok)
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> Session<OwnedProgram<P>, E> {
    /// Advances this run by one matching rule with owned rule witnesses.
    ///
    /// # Errors
    ///
    /// Returns `OwnedRunStepError` if matching, preparation, witness materialization,
    /// or application fails.
    pub(super) fn step_owned(&mut self) -> Result<CoreStep<OwnedRuleWitness>, OwnedRunStepError> {
        step_with_witness(&mut self.core, &self.program.program, |rule| {
            OwnedRuleWitness::from_rule_view(rule).map_err(OwnedRunStepError::RuleWitnessAllocation)
        })
    }
}

impl<'program, P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy>
    AttemptSession<BorrowedProgram<'program, P>, E, A>
{
    /// Advances this rule-attempt run by one executable rule with borrowed rule witnesses.
    ///
    /// # Errors
    ///
    /// Returns `RuleAttemptStepError` if rule-attempt matching, preparation, or application
    /// fails.
    pub(super) fn step_borrowed(
        &mut self,
    ) -> Result<CoreRuleAttempt<RuleView<'program>>, RuleAttemptStepError> {
        let mut context = RuleAttemptContext {
            program: self.program.program,
            core: &mut self.core,
            cursor: &mut self.cursor,
            attempt_budget: &mut self.attempt_budget,
        };
        attempt_current_rule_with_witness(&mut context, Ok)
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy>
    AttemptSession<OwnedProgram<P>, E, A>
{
    /// Advances this rule-attempt run by one executable rule with owned rule witnesses.
    ///
    /// # Errors
    ///
    /// Returns `OwnedRuleAttemptStepError` if rule-attempt matching, preparation, witness
    /// materialization, or application fails.
    pub(super) fn step_owned(
        &mut self,
    ) -> Result<CoreRuleAttempt<OwnedRuleWitness>, OwnedRuleAttemptStepError> {
        let mut context = RuleAttemptContext {
            program: &self.program.program,
            core: &mut self.core,
            cursor: &mut self.cursor,
            attempt_budget: &mut self.attempt_budget,
        };
        attempt_current_rule_with_witness(&mut context, |rule| {
            OwnedRuleWitness::from_rule_view(rule)
                .map_err(OwnedRuleAttemptStepError::RuleWitnessAllocation)
        })
    }
}

/// Advances the current run using the supplied rule-witness boundary.
///
/// # Errors
///
/// Returns `Error` if matching, rule preparation, rule-witness materialization,
/// or rule application fails.
fn step_with_witness<'program, P: ParsePolicy, E: ExecutionPolicy, RuleWitness, Error>(
    core: &mut RunCore<E>,
    program: &'program Program<P>,
    make_witness: impl FnOnce(RuleView<'program>) -> Result<RuleWitness, Error>,
) -> Result<CoreStep<RuleWitness>, Error>
where
    Error: From<RunStepError>,
{
    let matched = match find_next_match(program.rule_scan(), &mut core.once_states, &core.state) {
        RuleSearch::Matched(matched) => matched,
        RuleSearch::Stable => return Ok(CoreStep::Stable(core.budget.completed_steps())),
    };
    let state_len = core.state.byte_count();
    let prepared = prepare_matched_rule(&mut core.scratch, &mut core.budget, state_len, matched)?;
    let witnessed = WitnessedApplication::new(prepared, make_witness)?;
    let applied = witnessed.commit(&mut core.state, &mut core.scratch);
    Ok(CoreStep::Applied(applied))
}

/// Evaluates the current cursor against a parsed program.
///
/// # Errors
///
/// Returns `Error` if rule-attempt, rule matching, or rule application
/// fails.
fn attempt_current_rule_with_witness<
    'program,
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    RuleWitness,
    Error,
>(
    context: &mut RuleAttemptContext<'_, 'program, P, E, A>,
    make_witness: impl FnOnce(RuleView<'program>) -> Result<RuleWitness, Error>,
) -> Result<CoreRuleAttempt<RuleWitness>, Error>
where
    Error: From<RuleAttemptStepError> + From<RunStepError>,
{
    let target = match context.program.select_attempt_target(context.cursor) {
        RuleAttemptTargetSelection::Target(target) => target,
        RuleAttemptTargetSelection::NoExecutableRules => return Ok(no_executable_rules(context)),
    };
    let (after_miss, target) = target.into_parts();
    let runtime_rule = runtime_rule_for_target(&mut context.core.once_states, target);

    let reservation = context
        .attempt_budget
        .reserve_next_attempt(context.core.state.byte_count())?;
    let attempted = attempt_rule(runtime_rule, &context.core.state);

    match attempted {
        RuleAttempt::Missed(missed) => {
            let witness = make_witness(RuleView::new(missed.rule()))?;
            let attempt = reservation.commit();
            Ok(commit_miss(MissCommit {
                cursor: &mut *context.cursor,
                attempt_budget: &*context.attempt_budget,
                core: &*context.core,
                attempt,
                after_miss,
                miss: RuleMiss::new(witness, missed.reason()),
            }))
        }
        RuleAttempt::Matched(matched) => {
            let state_len = context.core.state.byte_count();
            let prepared = prepare_matched_rule(
                &mut context.core.scratch,
                &mut context.core.budget,
                state_len,
                matched,
            )?;
            let witnessed = WitnessedApplication::new(prepared, make_witness)?;
            let attempt = reservation.commit();
            let applied = witnessed.commit(&mut context.core.state, &mut context.core.scratch);
            if matches!(applied, CoreAppliedRule::Rewrite { .. }) {
                *context.cursor = RuleCursor::first();
            }
            Ok(CoreRuleAttempt::Applied { attempt, applied })
        }
    }
}

/// Materializes a stable rule-attempt result when the cursor has no executable target.
fn no_executable_rules<P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy, RuleWitness>(
    context: &RuleAttemptContext<'_, '_, P, E, A>,
) -> CoreRuleAttempt<RuleWitness> {
    CoreRuleAttempt::Stable {
        attempts: context.attempt_budget.completed_attempts(),
        steps: context.core.completed_steps(),
        stable_reason: RuleAttemptStableReason::NoExecutableRules,
    }
}

/// Commits a non-applying rule attempt and decides whether the run is stable.
fn commit_miss<E: ExecutionPolicy, A: RuleAttemptPolicy, RuleWitness>(
    context: MissCommit<'_, E, A, RuleWitness>,
) -> CoreRuleAttempt<RuleWitness> {
    match context.after_miss {
        RuleCursorAfterMiss::Stable => CoreRuleAttempt::Stable {
            attempts: context.attempt_budget.completed_attempts(),
            steps: context.core.completed_steps(),
            stable_reason: RuleAttemptStableReason::FinalMiss(context.miss),
        },
        RuleCursorAfterMiss::Advanced(cursor) => {
            *context.cursor = cursor;
            CoreRuleAttempt::Missed {
                attempt: context.attempt,
                miss: context.miss,
            }
        }
    }
}

impl<'program, P: ParsePolicy, E: ExecutionPolicy> Session<BorrowedProgram<'program, P>, E> {
    /// Runs to completion while emitting borrowed trace events.
    ///
    /// # Errors
    ///
    /// Returns `TracedRunError::Trace` if the trace sink fails. Returns
    /// `TracedRunError::Run` if runtime execution fails.
    pub(super) fn run_with_borrowed_trace<F, TraceError>(
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
                .core
                .step_runtime(self.program.program)
                .map_err(RunFinishError::from)
                .map_err(RunError::from)
                .map_err(TracedRunError::Run)?
            {
                RuntimeStep::Applied(AppliedRule::Rewrite(committed)) => {
                    let step = committed.step();
                    let rule = RuleView::new(committed.rule());
                    Self::emit_step_trace(
                        &mut trace,
                        step,
                        rule,
                        BorrowedTraceEffect::Continue {
                            state: self.state(),
                        },
                    )?;
                }
                RuntimeStep::Applied(AppliedRule::Return(committed)) => {
                    let step = committed.step();
                    let rule = RuleView::new(committed.rule());
                    let output = committed.output_view();
                    Self::emit_step_trace(
                        &mut trace,
                        step,
                        rule,
                        BorrowedTraceEffect::Return { output },
                    )?;
                    return Ok(RunResult::from_return(committed.into_output(), step));
                }
                RuntimeStep::Stable(steps) => {
                    return self
                        .core
                        .into_stable_result(steps)
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

impl<P: ParsePolicy, E: ExecutionPolicy> Session<OwnedProgram<P>, E> {
    /// Splits an owned session into its program and mutable core.
    pub(super) fn into_program_core(self) -> (Program<P>, RunCore<E>) {
        (self.program.program, self.core)
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy>
    AttemptSession<OwnedProgram<P>, E, A>
{
    /// Splits an owned rule-attempt session into its program and mutable core.
    pub(super) fn into_program_core(self) -> (Program<P>, RunCore<E>) {
        (self.program.program, self.core)
    }
}
