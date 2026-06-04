use crate::bytes::RuntimeStateByteCount;
use crate::error::{RuleAttemptStepError, RunStepError};
use crate::inspect::RuleView;
use crate::limits::{RuleAttemptCount, StepCount};
use crate::policy::{ExecutionPolicy, RuleAttemptPolicy};
use crate::program::{ExecutableProgram, ReturnOutput, ReturnOutputView};
use crate::runtime::action::{AppliedRule, PreparedRuleStep, prepare_matched_rule};
use crate::runtime::budget::{RuleAttemptBudgetState, RuleAttemptReservation, RuntimeBudgetState};
use crate::runtime::matcher::{MatchedRuleApplication, RuleAttempt, RuleAttemptMiss};
use crate::runtime::once::{
    AfterMissContinuingRulePass, AfterMissFinalRulePass, AfterMissRuntimeRulePass,
    FirstContinuingRulePass, FirstFinalRulePass, RuntimeRulePassState, StartedRuntimeRulePass,
};
use crate::runtime::rewrite::RewriteScratch;
use crate::runtime::state::State;

use super::attempt::RuleMiss;
use super::engine::{
    AttemptRunCore, AttemptSession, AttemptSessionCursor, ContinuingAttemptSession,
    FinalAttemptSession, TerminalAttemptSession,
};

/// Continuing rule-attempt pass behavior shared by first and after-miss states.
trait ContinuingRuleAttemptPass<'program>: RuntimeRulePassState<'program> + Sized {
    /// Attempts this pass's current target.
    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttempt<'program, 'state, 'once>;

    /// Commits a miss and advances to the next typed pass.
    fn commit_attempt_miss(self) -> AfterMissRuntimeRulePass<'program>;

    /// Resets this pass after a committed rewrite.
    fn reset_attempt_after_rewrite(self) -> StartedRuntimeRulePass<'program>;
}

/// Final rule-attempt pass behavior shared by first and after-miss states.
trait FinalRuleAttemptPass<'program>: RuntimeRulePassState<'program> + Sized {
    /// Attempts this pass's current target.
    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttempt<'program, 'state, 'once>;

    /// Resets this pass after a committed rewrite.
    fn reset_attempt_after_rewrite(self) -> StartedRuntimeRulePass<'program>;
}

/// Program-bound result of consuming one continuing rule-attempt session step.
pub(super) enum CoreContinuingRuleAttemptStep<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// A non-applying rule line was consumed and the run can continue.
    Missed {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Non-applying rule information.
        miss: RuleMiss<'program>,
        /// Continuation session with the returned next cursor.
        continuation: AttemptSessionCursor<'program, E, A>,
    },
    /// A rewrite committed and the rule-attempt run can continue.
    Applied {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Committed rewrite step count.
        step: StepCount,
        /// Rule witness paired with the committed rewrite.
        rule: RuleView<'program>,
        /// Continuation session with a fresh cursor.
        continuation: AttemptSessionCursor<'program, E, A>,
    },
    /// A return rule committed and the run is terminal.
    Returned {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Committed return step count.
        step: StepCount,
        /// Rule witness paired with the committed return.
        rule: RuleView<'program>,
        /// Materialized return output.
        output: ReturnOutput,
        /// Terminal session with no resumable cursor.
        terminal: TerminalAttemptSession<'program>,
    },
    /// A candidate attempt failed before committing runtime state.
    Failed {
        /// Error that prevented commit.
        error: RuleAttemptStepError,
        /// Terminal session preserving the uncommitted state.
        terminal: TerminalAttemptSession<'program>,
    },
}

/// Program-bound result of consuming one final rule-attempt session step.
pub(super) enum CoreFinalRuleAttemptStep<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// A rewrite committed and the rule-attempt run can continue.
    Applied {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Committed rewrite step count.
        step: StepCount,
        /// Rule witness paired with the committed rewrite.
        rule: RuleView<'program>,
        /// Continuation session with a fresh cursor.
        continuation: AttemptSessionCursor<'program, E, A>,
    },
    /// A return rule committed and the run is terminal.
    Returned {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Committed return step count.
        step: StepCount,
        /// Rule witness paired with the committed return.
        rule: RuleView<'program>,
        /// Materialized return output.
        output: ReturnOutput,
        /// Terminal session with no resumable cursor.
        terminal: TerminalAttemptSession<'program>,
    },
    /// No rule in the current pass matched the current runtime state.
    Stable {
        /// Rule attempts consumed before stability.
        attempts: RuleAttemptCount,
        /// Final non-applying rule that exhausted the current pass.
        final_miss: RuleMiss<'program>,
        /// Terminal session with no resumable cursor.
        terminal: TerminalAttemptSession<'program>,
    },
    /// A candidate attempt failed before committing runtime state.
    Failed {
        /// Error that prevented commit.
        error: RuleAttemptStepError,
        /// Terminal session preserving the uncommitted state.
        terminal: TerminalAttemptSession<'program>,
    },
}

impl<'program> ContinuingRuleAttemptPass<'program> for FirstContinuingRulePass<'program> {
    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttempt<'program, 'state, 'once> {
        self.attempt_current(state)
    }

    fn commit_attempt_miss(self) -> AfterMissRuntimeRulePass<'program> {
        self.commit_miss()
    }

    fn reset_attempt_after_rewrite(self) -> StartedRuntimeRulePass<'program> {
        self.reset_after_rewrite()
    }
}

impl<'program> ContinuingRuleAttemptPass<'program> for AfterMissContinuingRulePass<'program> {
    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttempt<'program, 'state, 'once> {
        self.attempt_current(state)
    }

    fn commit_attempt_miss(self) -> AfterMissRuntimeRulePass<'program> {
        self.commit_miss()
    }

    fn reset_attempt_after_rewrite(self) -> StartedRuntimeRulePass<'program> {
        self.reset_after_rewrite()
    }
}

impl<'program> FinalRuleAttemptPass<'program> for FirstFinalRulePass<'program> {
    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttempt<'program, 'state, 'once> {
        self.attempt_current(state)
    }

    fn reset_attempt_after_rewrite(self) -> StartedRuntimeRulePass<'program> {
        self.reset_after_rewrite()
    }
}

impl<'program> FinalRuleAttemptPass<'program> for AfterMissFinalRulePass<'program> {
    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttempt<'program, 'state, 'once> {
        self.attempt_current(state)
    }

    fn reset_attempt_after_rewrite(self) -> StartedRuntimeRulePass<'program> {
        self.reset_after_rewrite()
    }
}

/// Advances a borrowed rule-attempt session whose current rule has successors.
pub(super) fn advance_continuing_borrowed_rule_attempt<'program, E, A>(
    session: ContinuingAttemptSession<'program, E, A>,
) -> CoreContinuingRuleAttemptStep<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    match session {
        ContinuingAttemptSession::First(session) => advance_continuing_rule_attempt(session),
        ContinuingAttemptSession::AfterMiss(session) => advance_continuing_rule_attempt(session),
    }
}

/// Advances a borrowed rule-attempt session whose current rule exhausts the pass.
pub(super) fn advance_final_borrowed_rule_attempt<'program, E, A>(
    session: FinalAttemptSession<'program, E, A>,
) -> CoreFinalRuleAttemptStep<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    match session {
        FinalAttemptSession::First(session) => advance_final_rule_attempt(session),
        FinalAttemptSession::AfterMiss(session) => advance_final_rule_attempt(session),
    }
}

/// Advances a rule-attempt step whose selected rule is not final in the pass.
fn advance_continuing_rule_attempt<'program, E, A, Pass>(
    session: AttemptSession<'program, E, A, Pass>,
) -> CoreContinuingRuleAttemptStep<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass: ContinuingRuleAttemptPass<'program>,
{
    let AttemptSession {
        program,
        core,
        mut attempt_budget,
    } = session;
    let AttemptRunCore {
        mut state,
        mut scratch,
        mut budget,
        runtime_rules: mut pass,
    } = core;

    let reservation = match attempt_budget.reserve_next_attempt(state.byte_count()) {
        Ok(reservation) => reservation,
        Err(error) => {
            let core = AttemptRunCore::from_parts(state, scratch, budget, pass);
            return failed_continuing_rule_attempt(program, core, &attempt_budget, error);
        }
    };

    match pass.attempt_current_rule(&state) {
        RuleAttempt::Missed(missed) => {
            let miss = public_rule_miss(missed);
            let attempt = reservation.commit();
            let runtime_rules = pass.commit_attempt_miss();
            let continuation = session_start_from_after_miss_pass(
                program,
                state,
                scratch,
                budget,
                runtime_rules,
                attempt_budget,
            );
            CoreContinuingRuleAttemptStep::Missed {
                attempt,
                miss,
                continuation,
            }
        }
        RuleAttempt::Matched(matched) => {
            let state_len = state.byte_count();
            let (attempt, witnessed) = match prepare_attempt_application(
                &mut scratch,
                &mut budget,
                state_len,
                reservation,
                matched,
            ) {
                Ok(committed) => committed,
                Err(error) => {
                    let core = AttemptRunCore::from_parts(state, scratch, budget, pass);
                    return failed_continuing_rule_attempt(program, core, &attempt_budget, error);
                }
            };
            let applied = witnessed.commit(&mut state, &mut scratch);
            let core = AttemptRunCore::from_parts(state, scratch, budget, pass);
            committed_continuing_rule_attempt_application(
                program,
                core,
                attempt_budget,
                attempt,
                applied,
            )
        }
    }
}

/// Advances a rule-attempt step whose selected rule exhausts the pass.
fn advance_final_rule_attempt<'program, E, A, Pass>(
    session: AttemptSession<'program, E, A, Pass>,
) -> CoreFinalRuleAttemptStep<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass: FinalRuleAttemptPass<'program>,
{
    let AttemptSession {
        program,
        core,
        mut attempt_budget,
    } = session;
    let AttemptRunCore {
        mut state,
        mut scratch,
        mut budget,
        runtime_rules: mut pass,
    } = core;

    let reservation = match attempt_budget.reserve_next_attempt(state.byte_count()) {
        Ok(reservation) => reservation,
        Err(error) => {
            let core = AttemptRunCore::from_parts(state, scratch, budget, pass);
            return failed_final_rule_attempt(program, core, &attempt_budget, error);
        }
    };

    match pass.attempt_current_rule(&state) {
        RuleAttempt::Missed(missed) => {
            let miss = public_rule_miss(missed);
            let attempt = reservation.commit();
            let core = AttemptRunCore::from_parts(state, scratch, budget, pass);
            let attempts = attempt;
            let terminal = TerminalAttemptSession {
                program,
                core: core.into_terminal(),
                attempts,
            };
            CoreFinalRuleAttemptStep::Stable {
                attempts,
                final_miss: miss,
                terminal,
            }
        }
        RuleAttempt::Matched(matched) => {
            let state_len = state.byte_count();
            let (attempt, witnessed) = match prepare_attempt_application(
                &mut scratch,
                &mut budget,
                state_len,
                reservation,
                matched,
            ) {
                Ok(committed) => committed,
                Err(error) => {
                    let core = AttemptRunCore::from_parts(state, scratch, budget, pass);
                    return failed_final_rule_attempt(program, core, &attempt_budget, error);
                }
            };
            let applied = witnessed.commit(&mut state, &mut scratch);
            let core = AttemptRunCore::from_parts(state, scratch, budget, pass);
            committed_final_rule_attempt_application(
                program,
                core,
                attempt_budget,
                attempt,
                applied,
            )
        }
    }
}

/// Reports a continuing-pass rule-attempt failure with the uncommitted runtime state.
fn failed_continuing_rule_attempt<'program, E, A, Pass>(
    program: &'program ExecutableProgram,
    core: AttemptRunCore<E, Pass>,
    attempt_budget: &RuleAttemptBudgetState<A>,
    error: RuleAttemptStepError,
) -> CoreContinuingRuleAttemptStep<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    CoreContinuingRuleAttemptStep::Failed {
        error,
        terminal: TerminalAttemptSession {
            program,
            core: core.into_terminal(),
            attempts: attempt_budget.completed_attempts(),
        },
    }
}

/// Reports a final-pass rule-attempt failure with the uncommitted runtime state.
fn failed_final_rule_attempt<'program, E, A, Pass>(
    program: &'program ExecutableProgram,
    core: AttemptRunCore<E, Pass>,
    attempt_budget: &RuleAttemptBudgetState<A>,
    error: RuleAttemptStepError,
) -> CoreFinalRuleAttemptStep<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    CoreFinalRuleAttemptStep::Failed {
        error,
        terminal: TerminalAttemptSession {
            program,
            core: core.into_terminal(),
            attempts: attempt_budget.completed_attempts(),
        },
    }
}

/// Projects a continuing-pass committed rule application into the next rule-attempt state.
fn committed_continuing_rule_attempt_application<'program, E, A, Pass>(
    program: &'program ExecutableProgram,
    core: AttemptRunCore<E, Pass>,
    attempt_budget: RuleAttemptBudgetState<A>,
    attempt: RuleAttemptCount,
    applied: CoreAppliedRule<'program, RuleView<'program>>,
) -> CoreContinuingRuleAttemptStep<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass: ContinuingRuleAttemptPass<'program>,
{
    match applied {
        CoreAppliedRule::Continued { step, rule } => {
            let AttemptRunCore {
                state,
                scratch,
                budget,
                runtime_rules,
            } = core;
            let continuation = session_start_from_started_pass(
                program,
                state,
                scratch,
                budget,
                runtime_rules.reset_attempt_after_rewrite(),
                attempt_budget,
            );
            CoreContinuingRuleAttemptStep::Applied {
                attempt,
                step,
                rule,
                continuation,
            }
        }
        CoreAppliedRule::Terminal {
            step,
            rule,
            output_view: _,
            output,
        } => CoreContinuingRuleAttemptStep::Returned {
            attempt,
            step,
            rule,
            output,
            terminal: TerminalAttemptSession {
                program,
                core: core.into_terminal(),
                attempts: attempt_budget.completed_attempts(),
            },
        },
    }
}

/// Projects a final-pass committed rule application into the next rule-attempt state.
fn committed_final_rule_attempt_application<'program, E, A, Pass>(
    program: &'program ExecutableProgram,
    core: AttemptRunCore<E, Pass>,
    attempt_budget: RuleAttemptBudgetState<A>,
    attempt: RuleAttemptCount,
    applied: CoreAppliedRule<'program, RuleView<'program>>,
) -> CoreFinalRuleAttemptStep<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass: FinalRuleAttemptPass<'program>,
{
    match applied {
        CoreAppliedRule::Continued { step, rule } => {
            let AttemptRunCore {
                state,
                scratch,
                budget,
                runtime_rules,
            } = core;
            let continuation = session_start_from_started_pass(
                program,
                state,
                scratch,
                budget,
                runtime_rules.reset_attempt_after_rewrite(),
                attempt_budget,
            );
            CoreFinalRuleAttemptStep::Applied {
                attempt,
                step,
                rule,
                continuation,
            }
        }
        CoreAppliedRule::Terminal {
            step,
            rule,
            output_view: _,
            output,
        } => CoreFinalRuleAttemptStep::Returned {
            attempt,
            step,
            rule,
            output,
            terminal: TerminalAttemptSession {
                program,
                core: core.into_terminal(),
                attempts: attempt_budget.completed_attempts(),
            },
        },
    }
}

/// Rebuilds a typed continuation session from a reset or newly started pass.
fn session_start_from_started_pass<'program, E, A>(
    program: &'program ExecutableProgram,
    state: State,
    scratch: RewriteScratch,
    budget: RuntimeBudgetState<E>,
    runtime_rules: StartedRuntimeRulePass<'program>,
    attempt_budget: RuleAttemptBudgetState<A>,
) -> AttemptSessionCursor<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    match runtime_rules {
        StartedRuntimeRulePass::Continuing(pass) => {
            AttemptSessionCursor::Continuing(ContinuingAttemptSession::First(AttemptSession {
                program,
                core: AttemptRunCore::from_parts(state, scratch, budget, pass),
                attempt_budget,
            }))
        }
        StartedRuntimeRulePass::Final(pass) => {
            AttemptSessionCursor::Final(FinalAttemptSession::First(AttemptSession {
                program,
                core: AttemptRunCore::from_parts(state, scratch, budget, pass),
                attempt_budget,
            }))
        }
    }
}

/// Rebuilds a typed continuation session after a committed miss.
fn session_start_from_after_miss_pass<'program, E, A>(
    program: &'program ExecutableProgram,
    state: State,
    scratch: RewriteScratch,
    budget: RuntimeBudgetState<E>,
    runtime_rules: AfterMissRuntimeRulePass<'program>,
    attempt_budget: RuleAttemptBudgetState<A>,
) -> AttemptSessionCursor<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    match runtime_rules {
        AfterMissRuntimeRulePass::Continuing(pass) => {
            AttemptSessionCursor::Continuing(ContinuingAttemptSession::AfterMiss(AttemptSession {
                program,
                core: AttemptRunCore::from_parts(state, scratch, budget, pass),
                attempt_budget,
            }))
        }
        AfterMissRuntimeRulePass::Final(pass) => {
            AttemptSessionCursor::Final(FinalAttemptSession::AfterMiss(AttemptSession {
                program,
                core: AttemptRunCore::from_parts(state, scratch, budget, pass),
                attempt_budget,
            }))
        }
    }
}

/// Projects runtime miss shapes into the public typed miss API.
fn public_rule_miss<'program>(miss: RuleAttemptMiss<'program>) -> RuleMiss<'program> {
    match miss {
        RuleAttemptMiss::StateMismatch(rule) => RuleMiss::state_mismatch(rule),
        RuleAttemptMiss::OnceRewriteConsumed(rule) => RuleMiss::once_rewrite_consumed(rule),
        RuleAttemptMiss::OnceReturnConsumed(rule) => RuleMiss::once_return_consumed(rule),
    }
}

/// Prepares a matched rule-attempt application and commits its consumed-attempt count.
///
/// # Errors
///
/// Returns `RuleAttemptStepError` if step preparation fails.
fn prepare_attempt_application<'program, 'once, 'budget, E, A>(
    scratch: &mut RewriteScratch,
    budget: &'budget mut RuntimeBudgetState<E>,
    state_len: RuntimeStateByteCount,
    attempt_reservation: RuleAttemptReservation<'_, A>,
    matched: MatchedRuleApplication<'program, '_, 'once>,
) -> Result<WitnessedRuleAttempt<'program, 'once, 'budget, E>, RuleAttemptStepError>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    let prepared = prepare_matched_rule(scratch, budget, state_len, matched)?;
    let witnessed = WitnessedApplication::new(prepared, |rule| rule);
    let attempt = attempt_reservation.commit();
    Ok((attempt, witnessed))
}
