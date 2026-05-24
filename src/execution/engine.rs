use crate::error::{RunError, RunInvariantError, TracedRunError};
use crate::input::RunSeed;
use crate::inspect::RuleView;
use crate::limits::{RuleAttemptCount, RuleAttemptLimit, StepCount};
use crate::program::{Program, RunResult};
use crate::runtime::action::{AppliedRule, apply_matched_rule};
use crate::runtime::budget::{RuleAttemptBudgetState, RuntimeBudgetState};
use crate::runtime::matcher::{
    RuleAttempt, RuleAttemptMiss, RuleSearch, attempt_rule, find_next_match,
};
use crate::runtime::once::OnceStateSet;
use crate::runtime::rewrite::RewriteScratch;
use crate::runtime::state::State;
use crate::trace::{BorrowedTraceEffect, BorrowedTraceEvent, RuntimeStateView};

use super::{RuleAttemptStableReason, RuleMiss};

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
pub(super) struct RuleCursor {
    /// Zero-based rule index to evaluate next.
    next_rule_index: usize,
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
pub(super) enum CoreStep {
    /// A rule committed and may have terminal side effects.
    Applied(AppliedRule),
    /// No rule matched the current runtime state.
    Stable(StepCount),
}

/// Internal non-error result of one rule-attempt step.
pub(super) enum CoreRuleAttempt {
    /// A rule line was consumed without applying.
    Missed {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Non-applying rule information.
        miss: RuleMiss,
    },
    /// A rule committed and may have terminal side effects.
    Applied {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Applied rule effect.
        applied: AppliedRule,
    },
    /// No rule in the current pass matched the current runtime state.
    Stable {
        /// Rule attempts consumed before stability.
        attempts: RuleAttemptCount,
        /// Rewrite steps committed before stability.
        steps: StepCount,
        /// Why the rule-attempt pass reached stability.
        stable_reason: RuleAttemptStableReason,
    },
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
        let once_states = OnceStateSet::new(program.rule_slice())?;
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
    fn step(&mut self, program: &Program) -> Result<CoreStep, RunError> {
        let matched =
            match find_next_match(program.rule_slice(), &mut self.once_states, &self.state)? {
                RuleSearch::Matched(matched) => matched,
                RuleSearch::Stable => return Ok(CoreStep::Stable(self.budget.completed_steps())),
            };

        Ok(CoreStep::Applied(apply_matched_rule(
            &mut self.state,
            &mut self.scratch,
            &mut self.budget,
            matched,
        )?))
    }
}

impl RuleCursor {
    /// Starts rule-attempt execution at the first executable rule.
    const fn first() -> Self {
        Self { next_rule_index: 0 }
    }

    /// Whether this cursor points at the final executable rule.
    fn is_final_rule(self, rule_count: usize) -> bool {
        self.next_rule_index
            .checked_add(1)
            .is_none_or(|next_index| next_index >= rule_count)
    }

    /// Advances to the next executable rule after a non-final miss.
    fn advance_after_miss(&mut self) -> Option<()> {
        self.next_rule_index = self.next_rule_index.checked_add(1)?;
        Some(())
    }

    /// Resets to the first executable rule after a committed match.
    const fn reset_to_first(&mut self) {
        self.next_rule_index = 0;
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

    /// Number of rewrite steps that have already completed in this run.
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
    pub(super) fn step(&mut self) -> Result<CoreStep, RunError> {
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
                CoreStep::Applied(AppliedRule::Rewrite(_)) => {}
                CoreStep::Applied(AppliedRule::Return(committed)) => {
                    return Ok(committed.into_result());
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
        let core = RunCore::new(program.program(), seed)?;
        Ok(Self {
            program,
            core,
            cursor: RuleCursor::first(),
            attempt_budget: RuleAttemptBudgetState::new(limit),
        })
    }

    /// Borrows the parsed program.
    pub(super) fn program(&self) -> &Program {
        self.program.program()
    }

    /// Number of rewrite steps that have already completed in this run.
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

    /// Advances this run by exactly one executable rule line when possible.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if rule-attempt, rule matching, or rule application
    /// fails.
    pub(super) fn step(&mut self) -> Result<CoreRuleAttempt, RunError> {
        let Self {
            program,
            core,
            cursor,
            attempt_budget,
        } = self;
        attempt_current_rule(program.program(), core, cursor, attempt_budget)
    }
}

/// Evaluates the current cursor against a parsed program.
///
/// # Errors
///
/// Returns `RunError` if rule-attempt, rule matching, or rule application
/// fails.
fn attempt_current_rule(
    program: &Program,
    core: &mut RunCore,
    cursor: &mut RuleCursor,
    attempt_budget: &mut RuleAttemptBudgetState,
) -> Result<CoreRuleAttempt, RunError> {
    let rules = program.rule_slice();
    let Some(rule) = rules.get(cursor.next_rule_index) else {
        return Ok(CoreRuleAttempt::Stable {
            attempts: attempt_budget.completed_attempts(),
            steps: core.completed_steps(),
            stable_reason: RuleAttemptStableReason::NoExecutableRules,
        });
    };

    let permit = attempt_budget.reserve_next_attempt(core.state.byte_count())?;
    let attempted = attempt_rule(rule, &mut core.once_states, &core.state)?;
    let attempt = attempt_budget.commit(permit);

    match attempted {
        RuleAttempt::Missed(missed) => {
            commit_miss(cursor, attempt_budget, core, attempt, rules.len(), missed)
        }
        RuleAttempt::Matched(matched) => {
            let applied = apply_matched_rule(
                &mut core.state,
                &mut core.scratch,
                &mut core.budget,
                matched,
            )?;
            if matches!(applied, AppliedRule::Rewrite(_)) {
                cursor.reset_to_first();
            }
            Ok(CoreRuleAttempt::Applied { attempt, applied })
        }
    }
}

/// Commits a non-applying rule attempt and decides whether the run is stable.
///
/// # Errors
///
/// Returns `RunError` if advancing the rule-attempt cursor would violate an
/// internal representation invariant.
fn commit_miss(
    cursor: &mut RuleCursor,
    attempt_budget: &RuleAttemptBudgetState,
    core: &RunCore,
    attempt: RuleAttemptCount,
    rule_count: usize,
    missed: RuleAttemptMiss<'_>,
) -> Result<CoreRuleAttempt, RunError> {
    let miss = RuleMiss::new(missed.rule().position(), missed.reason());
    if cursor.is_final_rule(rule_count) {
        Ok(CoreRuleAttempt::Stable {
            attempts: attempt_budget.completed_attempts(),
            steps: core.completed_steps(),
            stable_reason: RuleAttemptStableReason::FinalMiss(miss),
        })
    } else {
        cursor
            .advance_after_miss()
            .ok_or(RunInvariantError::RuleAttemptCursorOverflow {
                rule: miss.rule_position(),
            })?;
        Ok(CoreRuleAttempt::Missed { attempt, miss })
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
            match self.step().map_err(TracedRunError::Run)? {
                CoreStep::Applied(AppliedRule::Rewrite(committed)) => {
                    let rule_position = committed.rule_position();
                    let rule = self
                        .program
                        .program
                        .rule_view_at(rule_position)
                        .map_err(TracedRunError::Run)?;
                    Self::emit_step_trace(
                        &mut trace,
                        committed.step(),
                        rule,
                        BorrowedTraceEffect::Continue {
                            state: self.state(),
                        },
                    )?;
                }
                CoreStep::Applied(AppliedRule::Return(committed)) => {
                    let step = committed.step();
                    let rule_position = committed.rule_position();
                    let rule = self
                        .program
                        .program
                        .rule_view_at(rule_position)
                        .map_err(TracedRunError::Run)?;
                    let output = self
                        .program
                        .program
                        .return_output_view_at(rule_position)
                        .map_err(TracedRunError::Run)?;
                    Self::emit_step_trace(
                        &mut trace,
                        step,
                        rule,
                        BorrowedTraceEffect::Return { output },
                    )?;
                    return Ok(committed.into_result());
                }
                CoreStep::Stable(steps) => {
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
