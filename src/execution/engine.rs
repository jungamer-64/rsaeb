use crate::bytes::RuntimeStateByteCount;
use crate::error::{
    OwnedRuleAttemptStepError, OwnedRunStepError, RuleAttemptStepError, RunError, RunFinishError,
    RunStartError, RunStepError, TracedRunError,
};
use crate::input::AdmittedRun;
use crate::inspect::RuleView;
use crate::limits::{RuleAttemptCount, StepCount};
use crate::policy::{ExecutionPolicy, ParsePolicy, RuleAttemptPolicy};
use crate::program::{Program, ReturnOutput, RuleCursor, RuleCursorAfterMiss, RunResult};
use crate::runtime::action::{AppliedRule, PreparedRuleApplication, prepare_matched_rule};
use crate::runtime::budget::{RuleAttemptBudgetState, RuleAttemptReservation, RuntimeBudgetState};
use crate::runtime::matcher::{
    MatchedRuleApplication, RuleAttempt, RuleSearch, attempt_rule, find_next_match,
};
use crate::runtime::once::{RuntimeRuleStates, RuntimeRuleTargetSelection};
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
    /// Per-run execution state aligned with the parsed rule table.
    rule_states: RuntimeRuleStates,
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

/// Terminal rule-attempt state after the cursor can no longer resume.
pub(super) struct TerminalAttemptSession<P, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// Borrowed or owned parsed program.
    pub(super) program: P,
    /// Mutable execution state retained for terminal observation.
    pub(super) core: RunCore<E>,
    /// Rule-attempt budget and consumed-attempt count.
    pub(super) attempt_budget: RuleAttemptBudgetState<A>,
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

/// Program-bound result of consuming one rule-attempt session step.
pub(super) enum CoreRuleAttemptStep<
    P,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    RuleWitness,
    StepError,
> {
    /// A non-applying rule line was consumed and the run can continue.
    Missed {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Non-applying rule information.
        miss: RuleMiss<RuleWitness>,
        /// Continuation session with the returned next cursor.
        continuation: AttemptSession<P, E, A>,
    },
    /// A rewrite committed and the rule-attempt run can continue.
    Applied {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Committed rewrite step count.
        step: StepCount,
        /// Rule witness paired with the committed rewrite.
        rule: RuleWitness,
        /// Continuation session with a fresh cursor.
        continuation: AttemptSession<P, E, A>,
    },
    /// A return rule committed and the run is terminal.
    Returned {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Committed return step count.
        step: StepCount,
        /// Rule witness paired with the committed return.
        rule: RuleWitness,
        /// Materialized return output.
        output: ReturnOutput,
        /// Terminal session with no resumable cursor.
        terminal: TerminalAttemptSession<P, E, A>,
    },
    /// No rule in the current pass matched the current runtime state.
    Stable {
        /// Rule attempts consumed before stability.
        attempts: RuleAttemptCount,
        /// Rewrite steps committed before stability.
        steps: StepCount,
        /// Why the rule-attempt pass reached stability.
        stable_reason: RuleAttemptStableReason<RuleWitness>,
        /// Terminal session with no resumable cursor.
        terminal: TerminalAttemptSession<P, E, A>,
    },
    /// A candidate attempt failed before committing runtime state.
    Failed {
        /// Error that prevented commit.
        error: StepError,
        /// Terminal session preserving the uncommitted state.
        terminal: TerminalAttemptSession<P, E, A>,
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
    fn new<P: ParsePolicy>(
        program: &Program<P>,
        admitted: AdmittedRun<E>,
    ) -> Result<Self, RunStartError> {
        let (input, budget) = admitted.into_runtime_parts();
        let state = State::from_input(input);
        let rule_states = RuntimeRuleStates::new(program.rule_scan())?;
        Ok(Self {
            state,
            scratch: RewriteScratch::new(),
            budget,
            rule_states,
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
}

impl<P: ProgramOwner, E: ExecutionPolicy> Session<P, E> {
    /// Starts a new run session for a parsed program and admitted run witness.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule state fails.
    pub(super) fn new(program: P, admitted: AdmittedRun<E>) -> Result<Self, RunStartError> {
        let core = RunCore::new(program.program(), admitted)?;
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

    /// Advances this program-bound runtime state against its own parsed program.
    ///
    /// # Errors
    ///
    /// Returns `RunStepError` if applying the matched rule exceeds limits or allocation fails.
    fn step_runtime(&mut self) -> Result<RuntimeStep<'_>, RunStepError> {
        let program = self.program.program();
        let matched = match find_next_match(
            program.rule_scan(),
            &mut self.core.rule_states,
            &self.core.state,
        ) {
            RuleSearch::Matched(matched) => matched,
            RuleSearch::Stable => {
                return Ok(RuntimeStep::Stable(self.core.budget.completed_steps()));
            }
        };

        let state_len = self.core.state.byte_count();
        let prepared = prepare_matched_rule(
            &mut self.core.scratch,
            &mut self.core.budget,
            state_len,
            matched,
        )?;
        let applied = prepared.commit(&mut self.core.state, &mut self.core.scratch);
        Ok(RuntimeStep::Applied(applied))
    }

    /// Advances this run by exactly one matching rule when possible.
    ///
    /// # Errors
    ///
    /// Returns `RunStepError` if rule matching or rule application fails.
    pub(super) fn step(&mut self) -> Result<CoreStep<()>, RunStepError> {
        let applied = match self.step_runtime()? {
            RuntimeStep::Applied(applied) => applied,
            RuntimeStep::Stable(steps) => return Ok(CoreStep::Stable(steps)),
        };
        Ok(CoreStep::Applied(CoreAppliedRule::from_applied_rule(
            applied,
            (),
        )))
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
    /// Starts a new rule-attempt session for a parsed program and admitted run witness.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule state fails.
    pub(super) fn new(program: P, admitted: AdmittedRun<E>) -> Result<Self, RunStartError> {
        let cursor = RuleCursor::first();
        let core = RunCore::new(program.program(), admitted)?;
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

/// Builds terminal rule-attempt state after the cursor has no valid continuation.
fn terminal_rule_attempt_session<P, E, A>(
    program: P,
    core: RunCore<E>,
    attempt_budget: RuleAttemptBudgetState<A>,
) -> TerminalAttemptSession<P, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    TerminalAttemptSession {
        program,
        core,
        attempt_budget,
    }
}

/// Builds a resumable rule-attempt session from transition parts.
fn continuing_rule_attempt_session<P, E, A>(
    program: P,
    core: RunCore<E>,
    cursor: RuleCursor,
    attempt_budget: RuleAttemptBudgetState<A>,
) -> AttemptSession<P, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    AttemptSession {
        program,
        core,
        cursor,
        attempt_budget,
    }
}

/// Reports stability when a cursor cannot select any executable rule.
fn no_executable_rule_attempt<P, E, A, RuleWitness, StepError>(
    program: P,
    core: RunCore<E>,
    attempt_budget: RuleAttemptBudgetState<A>,
) -> CoreRuleAttemptStep<P, E, A, RuleWitness, StepError>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    let attempts = attempt_budget.completed_attempts();
    let steps = core.completed_steps();
    let terminal = terminal_rule_attempt_session(program, core, attempt_budget);
    CoreRuleAttemptStep::Stable {
        attempts,
        steps,
        stable_reason: RuleAttemptStableReason::NoExecutableRules,
        terminal,
    }
}

/// Reports a rule-attempt failure with the uncommitted runtime state.
fn failed_rule_attempt<P, E, A, RuleWitness, StepError>(
    program: P,
    core: RunCore<E>,
    attempt_budget: RuleAttemptBudgetState<A>,
    error: StepError,
) -> CoreRuleAttemptStep<P, E, A, RuleWitness, StepError>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    let terminal = terminal_rule_attempt_session(program, core, attempt_budget);
    CoreRuleAttemptStep::Failed { error, terminal }
}

/// Commits a non-applying rule attempt and returns the next typed state.
fn committed_rule_miss<P, E, A, RuleWitness, StepError>(
    program: P,
    core: RunCore<E>,
    attempt_budget: RuleAttemptBudgetState<A>,
    attempt: RuleAttemptCount,
    after_miss: RuleCursorAfterMiss,
    miss: RuleMiss<RuleWitness>,
) -> CoreRuleAttemptStep<P, E, A, RuleWitness, StepError>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    match after_miss {
        RuleCursorAfterMiss::Stable => {
            let attempts = attempt_budget.completed_attempts();
            let steps = core.completed_steps();
            let terminal = terminal_rule_attempt_session(program, core, attempt_budget);
            CoreRuleAttemptStep::Stable {
                attempts,
                steps,
                stable_reason: RuleAttemptStableReason::FinalMiss(miss),
                terminal,
            }
        }
        RuleCursorAfterMiss::Advanced(cursor) => CoreRuleAttemptStep::Missed {
            attempt,
            miss,
            continuation: continuing_rule_attempt_session(program, core, cursor, attempt_budget),
        },
    }
}

/// Projects a committed rule application into the next rule-attempt state.
fn committed_rule_attempt_application<P, E, A, RuleWitness, StepError>(
    program: P,
    core: RunCore<E>,
    attempt_budget: RuleAttemptBudgetState<A>,
    attempt: RuleAttemptCount,
    applied: CoreAppliedRule<RuleWitness>,
) -> CoreRuleAttemptStep<P, E, A, RuleWitness, StepError>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    match applied {
        CoreAppliedRule::Rewrite { step, rule } => CoreRuleAttemptStep::Applied {
            attempt,
            step,
            rule,
            continuation: continuing_rule_attempt_session(
                program,
                core,
                RuleCursor::first(),
                attempt_budget,
            ),
        },
        CoreAppliedRule::Return { step, rule, output } => {
            let terminal = terminal_rule_attempt_session(program, core, attempt_budget);
            CoreRuleAttemptStep::Returned {
                attempt,
                step,
                rule,
                output,
                terminal,
            }
        }
    }
}

/// Prepares a matched rule-attempt application and commits its consumed-attempt count.
///
/// # Errors
///
/// Returns `Error` if step preparation or rule-witness materialization fails.
fn prepare_witnessed_rule_attempt<'program, 'once, 'budget, E, A, RuleWitness, Error>(
    scratch: &mut RewriteScratch,
    budget: &'budget mut RuntimeBudgetState<E>,
    state_len: RuntimeStateByteCount,
    attempt_reservation: RuleAttemptReservation<'_, A>,
    matched: MatchedRuleApplication<'program, '_, 'once>,
    make_witness: impl FnOnce(RuleView<'program>) -> Result<RuleWitness, Error>,
) -> Result<
    (
        RuleAttemptCount,
        WitnessedApplication<'program, 'once, 'budget, E, RuleWitness>,
    ),
    Error,
>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Error: From<RunStepError>,
{
    let prepared = prepare_matched_rule(scratch, budget, state_len, matched)?;
    let witnessed = WitnessedApplication::new(prepared, make_witness)?;
    let attempt = attempt_reservation.commit();
    Ok((attempt, witnessed))
}

impl<'program, P: ParsePolicy, E: ExecutionPolicy> Session<BorrowedProgram<'program, P>, E> {
    /// Advances this borrowed program-bound runtime state without materializing a rule witness.
    ///
    /// # Errors
    ///
    /// Returns `RunStepError` if applying the matched rule exceeds limits or allocation fails.
    fn step_runtime_borrowed(&mut self) -> Result<RuntimeStep<'program>, RunStepError> {
        let program = self.program.program;
        let matched = match find_next_match(
            program.rule_scan(),
            &mut self.core.rule_states,
            &self.core.state,
        ) {
            RuleSearch::Matched(matched) => matched,
            RuleSearch::Stable => {
                return Ok(RuntimeStep::Stable(self.core.budget.completed_steps()));
            }
        };

        let state_len = self.core.state.byte_count();
        let prepared = prepare_matched_rule(
            &mut self.core.scratch,
            &mut self.core.budget,
            state_len,
            matched,
        )?;
        let applied = prepared.commit(&mut self.core.state, &mut self.core.scratch);
        Ok(RuntimeStep::Applied(applied))
    }

    /// Advances this run by one matching rule with borrowed rule witnesses.
    ///
    /// # Errors
    ///
    /// Returns `RunStepError` if matching, preparation, or application fails.
    pub(super) fn step_borrowed(&mut self) -> Result<CoreStep<RuleView<'program>>, RunStepError> {
        let matched = match find_next_match(
            self.program.program.rule_scan(),
            &mut self.core.rule_states,
            &self.core.state,
        ) {
            RuleSearch::Matched(matched) => matched,
            RuleSearch::Stable => return Ok(CoreStep::Stable(self.core.budget.completed_steps())),
        };
        let state_len = self.core.state.byte_count();
        let prepared = prepare_matched_rule(
            &mut self.core.scratch,
            &mut self.core.budget,
            state_len,
            matched,
        )?;
        let witnessed = WitnessedApplication::new(prepared, Ok::<_, RunStepError>)?;
        let applied = witnessed.commit(&mut self.core.state, &mut self.core.scratch);
        Ok(CoreStep::Applied(applied))
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
        let matched = match find_next_match(
            self.program.program.rule_scan(),
            &mut self.core.rule_states,
            &self.core.state,
        ) {
            RuleSearch::Matched(matched) => matched,
            RuleSearch::Stable => return Ok(CoreStep::Stable(self.core.budget.completed_steps())),
        };
        let state_len = self.core.state.byte_count();
        let prepared = prepare_matched_rule(
            &mut self.core.scratch,
            &mut self.core.budget,
            state_len,
            matched,
        )?;
        let witnessed = WitnessedApplication::new(prepared, |rule| {
            OwnedRuleWitness::from_rule_view(rule).map_err(OwnedRunStepError::RuleWitnessAllocation)
        })?;
        let applied = witnessed.commit(&mut self.core.state, &mut self.core.scratch);
        Ok(CoreStep::Applied(applied))
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
        self,
    ) -> CoreRuleAttemptStep<
        BorrowedProgram<'program, P>,
        E,
        A,
        RuleView<'program>,
        RuleAttemptStepError,
    > {
        let Self {
            program,
            mut core,
            cursor,
            mut attempt_budget,
        } = self;

        let target = match core
            .rule_states
            .select_attempt_target(program.program.rule_scan(), &cursor)
        {
            RuntimeRuleTargetSelection::Target(target) => target,
            RuntimeRuleTargetSelection::NoExecutableRules => {
                return no_executable_rule_attempt(program, core, attempt_budget);
            }
        };
        let (after_miss, runtime_rule) = target.into_parts();

        let reservation = match attempt_budget.reserve_next_attempt(core.state.byte_count()) {
            Ok(reservation) => reservation,
            Err(error) => {
                return failed_rule_attempt(program, core, attempt_budget, error);
            }
        };
        let attempted = attempt_rule(runtime_rule, &core.state);

        match attempted {
            RuleAttempt::Missed(missed) => {
                let miss = RuleMiss::new(RuleView::new(missed.rule()), missed.reason());
                let attempt = reservation.commit();
                committed_rule_miss(program, core, attempt_budget, attempt, after_miss, miss)
            }
            RuleAttempt::Matched(matched) => {
                let state_len = core.state.byte_count();
                let (attempt, witnessed) = match prepare_witnessed_rule_attempt(
                    &mut core.scratch,
                    &mut core.budget,
                    state_len,
                    reservation,
                    matched,
                    Ok,
                ) {
                    Ok(committed) => committed,
                    Err(error) => {
                        return failed_rule_attempt(program, core, attempt_budget, error);
                    }
                };
                let applied = witnessed.commit(&mut core.state, &mut core.scratch);
                committed_rule_attempt_application(program, core, attempt_budget, attempt, applied)
            }
        }
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
        self,
    ) -> CoreRuleAttemptStep<OwnedProgram<P>, E, A, OwnedRuleWitness, OwnedRuleAttemptStepError>
    {
        let Self {
            program,
            mut core,
            cursor,
            mut attempt_budget,
        } = self;

        let target = match core
            .rule_states
            .select_attempt_target(program.program.rule_scan(), &cursor)
        {
            RuntimeRuleTargetSelection::Target(target) => target,
            RuntimeRuleTargetSelection::NoExecutableRules => {
                return no_executable_rule_attempt(program, core, attempt_budget);
            }
        };
        let (after_miss, runtime_rule) = target.into_parts();

        let reservation = match attempt_budget.reserve_next_attempt(core.state.byte_count()) {
            Ok(reservation) => reservation,
            Err(error) => {
                return failed_rule_attempt(program, core, attempt_budget, error.into());
            }
        };
        let attempted = attempt_rule(runtime_rule, &core.state);

        match attempted {
            RuleAttempt::Missed(missed) => {
                let witness = match OwnedRuleWitness::from_rule_view(RuleView::new(missed.rule())) {
                    Ok(witness) => witness,
                    Err(error) => {
                        return failed_rule_attempt(
                            program,
                            core,
                            attempt_budget,
                            OwnedRuleAttemptStepError::RuleWitnessAllocation(error),
                        );
                    }
                };
                let miss = RuleMiss::new(witness, missed.reason());
                let attempt = reservation.commit();
                committed_rule_miss(program, core, attempt_budget, attempt, after_miss, miss)
            }
            RuleAttempt::Matched(matched) => {
                let state_len = core.state.byte_count();
                let (attempt, witnessed) = match prepare_witnessed_rule_attempt(
                    &mut core.scratch,
                    &mut core.budget,
                    state_len,
                    reservation,
                    matched,
                    |rule| {
                        OwnedRuleWitness::from_rule_view(rule)
                            .map_err(OwnedRuleAttemptStepError::RuleWitnessAllocation)
                    },
                ) {
                    Ok(committed) => committed,
                    Err(error) => {
                        return failed_rule_attempt(program, core, attempt_budget, error);
                    }
                };
                let applied = witnessed.commit(&mut core.state, &mut core.scratch);
                committed_rule_attempt_application(program, core, attempt_budget, attempt, applied)
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
                .step_runtime_borrowed()
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
