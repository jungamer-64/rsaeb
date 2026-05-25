use crate::error::{RunError, RunInvariantError, TracedRunError};
use crate::input::RunSeed;
use crate::inspect::{RuleCount, RulePosition, RuleView};
use crate::limits::{RuleAttemptCount, RuleAttemptLimit, StepCount};
use crate::program::{Program, ReturnOutput, RunResult};
use crate::runtime::action::{AppliedRule, apply_matched_rule, prepare_matched_rule};
use crate::runtime::budget::{RuleAttemptBudgetState, RuntimeBudgetState};
use crate::runtime::matcher::{RuleAttempt, RuleSearch, attempt_rule, find_next_match};
use crate::runtime::once::OnceStateSet;
use crate::runtime::rewrite::RewriteScratch;
use crate::runtime::state::State;
use crate::trace::{BorrowedTraceEffect, BorrowedTraceEvent, RuntimeStateView};

use super::{OwnedRuleWitness, RuleAttemptStableReason, RuleMiss};

/// Mutable runtime state independent of program ownership mode.
#[derive(Debug)]
pub(super) struct RunCore {
    /// Current runtime byte state.
    state: State,
    /// Reusable buffer for candidate rewrites.
    scratch: RewriteScratch,
    /// Runtime limits and completed-step count.
    budget: RuntimeBudgetState,
    /// Per-run consumption state for `(once)` rules.
    once_states: OnceStateSet,
}

/// Runtime session parameterized by program ownership.
pub(super) struct Session<P> {
    /// Borrowed or owned parsed program.
    pub(super) program: P,
    /// Mutable execution state.
    pub(super) core: RunCore,
}

/// Runtime rule-attempt session parameterized by program ownership.
pub(super) struct AttemptSession<P> {
    /// Borrowed or owned parsed program.
    pub(super) program: P,
    /// Mutable execution state.
    pub(super) core: RunCore,
    /// Next executable rule line to evaluate.
    pub(super) cursor: RuleCursor,
    /// Rule-attempt budget and consumed-attempt count.
    pub(super) attempt_budget: RuleAttemptBudgetState,
}

/// Cursor pointing to the next executable rule line in one rule-attempt run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuleCursor {
    /// Cursor points at the next executable rule index.
    Active(ActiveRuleCursor),
    /// No executable rule remains in this pass.
    Exhausted,
}

/// Zero-based executable rule index paired with its public position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct RuleIndex {
    /// Zero-based rule-table offset.
    zero_based: usize,
    /// Public one-based rule position for diagnostics.
    position: RulePosition,
}

/// Active cursor state for rule-attempt execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ActiveRuleCursor {
    /// Zero-based rule index to evaluate next.
    next_rule_index: RuleIndex,
    /// Final executable rule index in this program.
    final_rule_index: RuleIndex,
}

/// All data needed to commit one non-applying rule attempt.
struct MissCommit<'attempt, RuleWitness> {
    /// Cursor to advance when the miss is not the final executable rule.
    cursor: &'attempt mut RuleCursor,
    /// Rule-attempt budget after the miss has been committed.
    attempt_budget: &'attempt RuleAttemptBudgetState,
    /// Runtime core observed by the attempted rule.
    core: &'attempt RunCore,
    /// Committed attempt count assigned to this miss.
    attempt: RuleAttemptCount,
    /// Active cursor that selected the missed rule.
    active_cursor: ActiveRuleCursor,
    /// Non-applying rule selected by the current cursor.
    miss: RuleMiss<RuleWitness>,
}

/// Program ownership shape used by the internal runtime session.
pub(super) trait ProgramOwner {
    /// Borrows the parsed program.
    fn program(&self) -> &Program;
}

/// Borrowed program owner for run-to-completion and tracing.
#[derive(Debug, Clone, Copy)]
pub(super) struct BorrowedProgram<'program> {
    /// Parsed program borrowed by this run.
    pub(super) program: &'program Program,
}

/// Owned program owner for public stepwise execution.
#[derive(Debug)]
pub(super) struct OwnedProgram {
    /// Parsed program owned by the public run session.
    pub(super) program: Program,
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

impl ProgramOwner for BorrowedProgram<'_> {
    fn program(&self) -> &Program {
        self.program
    }
}

impl ProgramOwner for OwnedProgram {
    fn program(&self) -> &Program {
        &self.program
    }
}

impl RunCore {
    /// Builds the mutable runtime core for one execution.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if per-run rule state allocation fails.
    fn new(program: &Program, seed: RunSeed) -> Result<Self, RunError> {
        let (input, budget) = seed.into_runtime_parts();
        let state = State::from_input(input);
        let once_states = OnceStateSet::new(program.once_rule_slot_count())?;
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
    /// Returns `RunError` if final state materialization cannot allocate.
    pub(super) fn into_stable_result(self, steps: StepCount) -> Result<RunResult, RunError> {
        Ok(RunResult::stable(self.state.into_snapshot()?, steps))
    }

    /// Advances the mutable runtime core against the supplied immutable program.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if rule matching detects an internal invariant
    /// failure or if applying the matched rule exceeds limits or allocation
    /// fails.
    fn step_runtime<'program>(
        &mut self,
        program: &'program Program,
    ) -> Result<RuntimeStep<'program>, RunError> {
        let matched = match find_next_match(program.rule_slice(), &self.once_states, &self.state)? {
            RuleSearch::Matched(matched) => matched,
            RuleSearch::Stable => {
                return Ok(RuntimeStep::Stable(self.budget.completed_steps()));
            }
        };

        let applied = apply_matched_rule(
            &mut self.state,
            &mut self.scratch,
            &mut self.budget,
            &mut self.once_states,
            matched,
        )?;
        Ok(RuntimeStep::Applied(applied))
    }

    /// Advances the mutable runtime core against the supplied immutable program.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if rule matching detects an internal invariant
    /// failure or if applying the matched rule exceeds limits or allocation
    /// fails.
    fn step(&mut self, program: &Program) -> Result<CoreStep<()>, RunError> {
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

/// Cursor movement after a non-applying rule line has been consumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuleCursorAfterMiss {
    /// Cursor advanced to the next executable rule.
    Advanced(ActiveRuleCursor),
    /// The consumed miss was the final executable rule.
    Stable,
}

impl RuleCursor {
    /// Starts rule-attempt execution over a parsed program's executable rules.
    fn for_rule_count(rule_count: RuleCount) -> Self {
        let Some(final_rule_index) = RuleIndex::last_for(rule_count) else {
            return Self::Exhausted;
        };

        Self::Active(ActiveRuleCursor {
            next_rule_index: RuleIndex::first(),
            final_rule_index,
        })
    }

    /// Takes the active cursor state, leaving this cursor exhausted until the attempt commits.
    fn take_active(&mut self) -> Option<ActiveRuleCursor> {
        match core::mem::replace(self, Self::Exhausted) {
            Self::Active(active) => Some(active),
            Self::Exhausted => None,
        }
    }
}

impl ActiveRuleCursor {
    /// Current zero-based rule index.
    const fn current_index(&self) -> RuleIndex {
        self.next_rule_index
    }

    /// Advances after a miss or reports that the pass is stable.
    fn advance_after_miss(self) -> RuleCursorAfterMiss {
        if self.next_rule_index >= self.final_rule_index {
            return RuleCursorAfterMiss::Stable;
        }

        if let Some(next_rule_index) = self.next_rule_index.checked_next() {
            RuleCursorAfterMiss::Advanced(Self {
                next_rule_index,
                final_rule_index: self.final_rule_index,
            })
        } else {
            RuleCursorAfterMiss::Stable
        }
    }

    /// Resets to the first executable rule after a committed match.
    const fn reset_to_first(self) -> Self {
        Self {
            next_rule_index: RuleIndex::first(),
            final_rule_index: self.final_rule_index,
        }
    }
}

impl RuleIndex {
    /// First executable rule index.
    const fn first() -> Self {
        Self {
            zero_based: 0,
            position: RulePosition::FIRST,
        }
    }

    /// Builds an index from a zero-based rule-table offset.
    fn from_zero_based(zero_based: usize) -> Option<Self> {
        let position = RulePosition::from_zero_based(zero_based)?;
        Some(Self {
            zero_based,
            position,
        })
    }

    /// Final executable rule index for a parsed rule count.
    fn last_for(rule_count: RuleCount) -> Option<Self> {
        let zero_based = rule_count.get().checked_sub(1)?;
        Self::from_zero_based(zero_based)
    }

    /// Returns the checked next index.
    fn checked_next(self) -> Option<Self> {
        let zero_based = self.zero_based.checked_add(1)?;
        Self::from_zero_based(zero_based)
    }

    /// Zero-based rule-table offset.
    const fn get(self) -> usize {
        self.zero_based
    }

    /// Public rule position for diagnostics.
    const fn position(self) -> RulePosition {
        self.position
    }
}

impl<P: ProgramOwner> Session<P> {
    /// Starts a new run session for a parsed program and admitted run seed.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if allocating per-run rule state fails.
    pub(super) fn new(program: P, seed: RunSeed) -> Result<Self, RunError> {
        let core = RunCore::new(program.program(), seed)?;
        Ok(Self { program, core })
    }

    /// Borrows the parsed program.
    pub(super) fn program(&self) -> &Program {
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
    /// Returns `RunError` if rule matching or rule application fails.
    pub(super) fn step(&mut self) -> Result<CoreStep<()>, RunError> {
        self.core.step(self.program.program())
    }

    /// Runs this session to completion.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when a later matching rule would exceed configured
    /// limits.
    pub(super) fn finish(mut self) -> Result<RunResult, RunError> {
        loop {
            match self.step()? {
                CoreStep::Applied(CoreAppliedRule::Rewrite { .. }) => {}
                CoreStep::Applied(CoreAppliedRule::Return { step, output, .. }) => {
                    return Ok(RunResult::from_return(output, step));
                }
                CoreStep::Stable(steps) => return self.core.into_stable_result(steps),
            }
        }
    }
}

impl<P: ProgramOwner> AttemptSession<P> {
    /// Starts a new rule-attempt session for a parsed program and admitted run seed.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if allocating per-run rule state fails.
    pub(super) fn new(
        program: P,
        seed: RunSeed,
        limit: RuleAttemptLimit,
    ) -> Result<Self, RunError> {
        let cursor = RuleCursor::for_rule_count(program.program().rule_count());
        let core = RunCore::new(program.program(), seed)?;
        Ok(Self {
            program,
            core,
            cursor,
            attempt_budget: RuleAttemptBudgetState::new(limit),
        })
    }

    /// Borrows the parsed program.
    pub(super) fn program(&self) -> &Program {
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

impl<'program> Session<BorrowedProgram<'program>> {
    /// Advances this run by one matching rule with borrowed rule witnesses.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if matching, preparation, or application fails.
    pub(super) fn step_borrowed(&mut self) -> Result<CoreStep<RuleView<'program>>, RunError> {
        step_with_witness(&mut self.core, self.program.program, Ok)
    }
}

impl Session<OwnedProgram> {
    /// Advances this run by one matching rule with owned rule witnesses.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if matching, preparation, witness materialization,
    /// or application fails.
    pub(super) fn step_owned(&mut self) -> Result<CoreStep<OwnedRuleWitness>, RunError> {
        step_with_witness(&mut self.core, &self.program.program, |rule| {
            Ok(OwnedRuleWitness::from_rule_view(rule)?)
        })
    }
}

impl<'program> AttemptSession<BorrowedProgram<'program>> {
    /// Advances this rule-attempt run by one executable rule with borrowed rule witnesses.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if rule-attempt matching, preparation, or application
    /// fails.
    pub(super) fn step_borrowed(
        &mut self,
    ) -> Result<CoreRuleAttempt<RuleView<'program>>, RunError> {
        attempt_current_rule_with_witness(
            self.program.program,
            &mut self.core,
            &mut self.cursor,
            &mut self.attempt_budget,
            Ok,
        )
    }
}

impl AttemptSession<OwnedProgram> {
    /// Advances this rule-attempt run by one executable rule with owned rule witnesses.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if rule-attempt matching, preparation, witness
    /// materialization, or application fails.
    pub(super) fn step_owned(&mut self) -> Result<CoreRuleAttempt<OwnedRuleWitness>, RunError> {
        attempt_current_rule_with_witness(
            &self.program.program,
            &mut self.core,
            &mut self.cursor,
            &mut self.attempt_budget,
            |rule| Ok(OwnedRuleWitness::from_rule_view(rule)?),
        )
    }
}

/// Advances the current run using the supplied rule-witness boundary.
///
/// # Errors
///
/// Returns `RunError` if matching, rule preparation, rule-witness materialization,
/// or rule application fails.
fn step_with_witness<'program, RuleWitness>(
    core: &mut RunCore,
    program: &'program Program,
    make_witness: impl FnOnce(RuleView<'program>) -> Result<RuleWitness, RunError>,
) -> Result<CoreStep<RuleWitness>, RunError> {
    let matched = match find_next_match(program.rule_slice(), &core.once_states, &core.state)? {
        RuleSearch::Matched(matched) => matched,
        RuleSearch::Stable => return Ok(CoreStep::Stable(core.budget.completed_steps())),
    };
    let prepared = prepare_matched_rule(&core.state, &mut core.scratch, &mut core.budget, matched)?;
    let witness = make_witness(RuleView::new(prepared.rule()))?;
    let applied = prepared.commit(
        &mut core.state,
        &mut core.scratch,
        &mut core.budget,
        &mut core.once_states,
    )?;
    Ok(CoreStep::Applied(CoreAppliedRule::from_applied_rule(
        applied, witness,
    )))
}

/// Evaluates the current cursor against a parsed program.
///
/// # Errors
///
/// Returns `RunError` if rule-attempt, rule matching, or rule application
/// fails.
fn attempt_current_rule_with_witness<'program, RuleWitness>(
    program: &'program Program,
    core: &mut RunCore,
    cursor: &mut RuleCursor,
    attempt_budget: &mut RuleAttemptBudgetState,
    make_witness: impl FnOnce(RuleView<'program>) -> Result<RuleWitness, RunError>,
) -> Result<CoreRuleAttempt<RuleWitness>, RunError> {
    let rules = program.rule_slice();
    let Some(active_cursor) = cursor.take_active() else {
        return Ok(CoreRuleAttempt::Stable {
            attempts: attempt_budget.completed_attempts(),
            steps: core.completed_steps(),
            stable_reason: RuleAttemptStableReason::NoExecutableRules,
        });
    };
    let next_rule_index = active_cursor.current_index();

    let Some(rule) = rules.get(next_rule_index.get()) else {
        return Err(RunInvariantError::MissingRuleCursorTarget {
            rule: next_rule_index.position(),
            available_rules: program.rule_count(),
        }
        .into());
    };

    let permit = attempt_budget.reserve_next_attempt(core.state.byte_count())?;
    let attempted = attempt_rule(rule, &core.once_states, &core.state)?;

    match attempted {
        RuleAttempt::Missed(missed) => {
            let witness = make_witness(RuleView::new(missed.rule()))?;
            let attempt = attempt_budget.commit(permit)?;
            Ok(commit_miss(MissCommit {
                cursor,
                attempt_budget,
                core,
                attempt,
                active_cursor,
                miss: RuleMiss::new(witness, missed.reason()),
            }))
        }
        RuleAttempt::Matched(matched) => {
            let prepared =
                prepare_matched_rule(&core.state, &mut core.scratch, &mut core.budget, matched)?;
            let witness = make_witness(RuleView::new(prepared.rule()))?;
            let attempt = attempt_budget.commit(permit)?;
            let applied = prepared.commit(
                &mut core.state,
                &mut core.scratch,
                &mut core.budget,
                &mut core.once_states,
            )?;
            let applied = CoreAppliedRule::from_applied_rule(applied, witness);
            if matches!(applied, CoreAppliedRule::Rewrite { .. }) {
                *cursor = RuleCursor::Active(active_cursor.reset_to_first());
            }
            Ok(CoreRuleAttempt::Applied { attempt, applied })
        }
    }
}

/// Commits a non-applying rule attempt and decides whether the run is stable.
fn commit_miss<RuleWitness>(context: MissCommit<'_, RuleWitness>) -> CoreRuleAttempt<RuleWitness> {
    match context.active_cursor.advance_after_miss() {
        RuleCursorAfterMiss::Stable => CoreRuleAttempt::Stable {
            attempts: context.attempt_budget.completed_attempts(),
            steps: context.core.completed_steps(),
            stable_reason: RuleAttemptStableReason::FinalMiss(context.miss),
        },
        RuleCursorAfterMiss::Advanced(active_cursor) => {
            *context.cursor = RuleCursor::Active(active_cursor);
            CoreRuleAttempt::Missed {
                attempt: context.attempt,
                miss: context.miss,
            }
        }
    }
}

impl<'program> Session<BorrowedProgram<'program>> {
    /// Runs to completion while emitting borrowed trace events.
    ///
    /// # Errors
    ///
    /// Returns `TracedRunError::Trace` if the trace sink fails. Returns
    /// `TracedRunError::Run` if runtime execution fails.
    pub(super) fn run_with_borrowed_trace<F, E>(
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
            match self
                .core
                .step_runtime(self.program.program)
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

impl Session<OwnedProgram> {
    /// Splits an owned session into its program and mutable core.
    pub(super) fn into_program_core(self) -> (Program, RunCore) {
        (self.program.program, self.core)
    }
}

impl AttemptSession<OwnedProgram> {
    /// Splits an owned rule-attempt session into its program and mutable core.
    pub(super) fn into_program_core(self) -> (Program, RunCore) {
        (self.program.program, self.core)
    }
}
