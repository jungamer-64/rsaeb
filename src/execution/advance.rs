use crate::bytes::RuntimeStateByteCount;
use crate::error::RuleAttemptStepError;
use crate::limits::RuleAttemptCount;
use crate::policy::{ExecutionPolicy, RuleAttemptPolicy};
use crate::program::ExecutableProgram;
use crate::runtime::action::{AppliedRule, PreparedRuleStep, prepare_matched_rule};
use crate::runtime::budget::{RuleAttemptReservation, RuntimeBudgetState};
use crate::runtime::matcher::{EvaluatedRuleMiss, MatchedRuleApplication, RuleAttemptEvaluation};
use crate::runtime::once::{ContinuingRuleAttemptPass, FinalRuleAttemptPass};
use crate::runtime::rewrite::RewriteScratch;

use super::engine::{AttemptRunCore, AttemptRunCoreParts, AttemptSession};
use super::session::BorrowedRuleAttemptCursor;
use super::transition::{
    BorrowedAlwaysReturnStateMismatchRuleAttempt, BorrowedAlwaysRewriteStateMismatchRuleAttempt,
    BorrowedContinuingRuleAttemptTransition, BorrowedFinalRuleAttemptTransition,
    BorrowedOnceReturnStateMismatchRuleAttempt, BorrowedOnceRewriteConsumedRuleAttempt,
    BorrowedOnceRewriteStateMismatchRuleAttempt, BorrowedRuleAttemptAlwaysReturnRun,
    BorrowedRuleAttemptAlwaysRewriteStep, BorrowedRuleAttemptFailedRun,
    BorrowedRuleAttemptOnceReturnRun, BorrowedRuleAttemptOnceRewriteStep,
    BorrowedRuleAttemptStableAfterAlwaysReturnStateMismatch,
    BorrowedRuleAttemptStableAfterAlwaysRewriteStateMismatch,
    BorrowedRuleAttemptStableAfterOnceReturnStateMismatch,
    BorrowedRuleAttemptStableAfterOnceRewriteConsumed,
    BorrowedRuleAttemptStableAfterOnceRewriteStateMismatch,
};

/// Advances a borrowed rule-attempt session whose current rule has successors.
pub(super) fn advance_continuing_borrowed_rule_attempt<'program, E, A, Pass>(
    session: AttemptSession<'program, E, A, Pass>,
) -> BorrowedContinuingRuleAttemptTransition<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass: ContinuingRuleAttemptPass<'program>,
{
    advance_continuing_rule_attempt(session)
}

/// Advances a borrowed rule-attempt session whose current rule exhausts the pass.
pub(super) fn advance_final_borrowed_rule_attempt<'program, E, A, Pass>(
    session: AttemptSession<'program, E, A, Pass>,
) -> BorrowedFinalRuleAttemptTransition<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass: FinalRuleAttemptPass<'program>,
{
    advance_final_rule_attempt(session)
}

/// Advances a rule-attempt step whose selected rule is not final in the pass.
fn advance_continuing_rule_attempt<'program, E, A, Pass>(
    session: AttemptSession<'program, E, A, Pass>,
) -> BorrowedContinuingRuleAttemptTransition<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass: ContinuingRuleAttemptPass<'program>,
{
    let AttemptSession { program, core } = session;
    let (mut parts, mut pass) = core.into_parts();

    let reservation = match parts
        .attempt_budget
        .reserve_next_attempt(parts.state.byte_count())
    {
        Ok(reservation) => reservation,
        Err(error) => {
            let core = parts.with_pass(pass);
            return failed_continuing_rule_attempt(program, core, error);
        }
    };

    match pass.attempt_current_rule(&parts.state) {
        RuleAttemptEvaluation::Miss(miss) => {
            let attempt = reservation.commit();
            commit_continuing_miss(program, parts, pass, attempt, miss)
        }
        RuleAttemptEvaluation::Matched(matched) => {
            let state_len = parts.state.byte_count();
            let (attempt, prepared) = match prepare_attempt_application(
                &mut parts.scratch,
                &mut parts.budget,
                state_len,
                reservation,
                matched,
            ) {
                Ok(committed) => committed,
                Err(error) => {
                    let core = parts.with_pass(pass);
                    return failed_continuing_rule_attempt(program, core, error);
                }
            };
            let applied = prepared.commit(&mut parts.state, &mut parts.scratch);
            let core = parts.with_pass(pass);
            committed_continuing_rule_attempt_application(program, core, attempt, applied)
        }
    }
}

/// Advances a rule-attempt step whose selected rule exhausts the pass.
fn advance_final_rule_attempt<'program, E, A, Pass>(
    session: AttemptSession<'program, E, A, Pass>,
) -> BorrowedFinalRuleAttemptTransition<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass: FinalRuleAttemptPass<'program>,
{
    let AttemptSession { program, core } = session;
    let (mut parts, mut pass) = core.into_parts();

    let reservation = match parts
        .attempt_budget
        .reserve_next_attempt(parts.state.byte_count())
    {
        Ok(reservation) => reservation,
        Err(error) => {
            let core = parts.with_pass(pass);
            return failed_final_rule_attempt(program, core, error);
        }
    };

    match pass.attempt_current_rule(&parts.state) {
        RuleAttemptEvaluation::Miss(miss) => {
            let attempts = reservation.commit();
            commit_final_miss(program, parts, pass, attempts, miss)
        }
        RuleAttemptEvaluation::Matched(matched) => {
            let state_len = parts.state.byte_count();
            let (attempt, prepared) = match prepare_attempt_application(
                &mut parts.scratch,
                &mut parts.budget,
                state_len,
                reservation,
                matched,
            ) {
                Ok(committed) => committed,
                Err(error) => {
                    let core = parts.with_pass(pass);
                    return failed_final_rule_attempt(program, core, error);
                }
            };
            let applied = prepared.commit(&mut parts.state, &mut parts.scratch);
            let core = parts.with_pass(pass);
            committed_final_rule_attempt_application(program, core, attempt, applied)
        }
    }
}

/// Reports a continuing-pass rule-attempt failure with the uncommitted runtime state.
fn failed_continuing_rule_attempt<'program, E, A, Pass>(
    program: &'program ExecutableProgram,
    core: AttemptRunCore<E, A, Pass>,
    error: RuleAttemptStepError,
) -> BorrowedContinuingRuleAttemptTransition<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    let attempts = core.completed_attempts();
    BorrowedContinuingRuleAttemptTransition::Failed(BorrowedRuleAttemptFailedRun::new(
        error,
        attempts,
        program,
        core.into_terminal(),
    ))
}

/// Reports a final-pass rule-attempt failure with the uncommitted runtime state.
fn failed_final_rule_attempt<'program, E, A, Pass>(
    program: &'program ExecutableProgram,
    core: AttemptRunCore<E, A, Pass>,
    error: RuleAttemptStepError,
) -> BorrowedFinalRuleAttemptTransition<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    let attempts = core.completed_attempts();
    BorrowedFinalRuleAttemptTransition::Failed(BorrowedRuleAttemptFailedRun::new(
        error,
        attempts,
        program,
        core.into_terminal(),
    ))
}

/// Commits a non-final rule-attempt miss and returns its resumable miss witness.
fn commit_continuing_miss<'program, E, A, Pass>(
    program: &'program ExecutableProgram,
    parts: AttemptRunCoreParts<E, A>,
    pass: Pass,
    attempt: RuleAttemptCount,
    miss: EvaluatedRuleMiss<'program>,
) -> BorrowedContinuingRuleAttemptTransition<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass: ContinuingRuleAttemptPass<'program>,
{
    let runtime_rules = pass.commit_attempt_miss();
    let cursor = BorrowedRuleAttemptCursor::from_after_miss_parts(program, parts, runtime_rules);
    match miss {
        EvaluatedRuleMiss::AlwaysRewriteStateMismatch(rule) => {
            BorrowedContinuingRuleAttemptTransition::AlwaysRewriteStateMismatch(
                BorrowedAlwaysRewriteStateMismatchRuleAttempt {
                    attempt,
                    rule,
                    cursor,
                },
            )
        }
        EvaluatedRuleMiss::OnceRewriteStateMismatch(rule) => {
            BorrowedContinuingRuleAttemptTransition::OnceRewriteStateMismatch(
                BorrowedOnceRewriteStateMismatchRuleAttempt {
                    attempt,
                    rule,
                    cursor,
                },
            )
        }
        EvaluatedRuleMiss::AlwaysReturnStateMismatch(rule) => {
            BorrowedContinuingRuleAttemptTransition::AlwaysReturnStateMismatch(
                BorrowedAlwaysReturnStateMismatchRuleAttempt {
                    attempt,
                    rule,
                    cursor,
                },
            )
        }
        EvaluatedRuleMiss::OnceReturnStateMismatch(rule) => {
            BorrowedContinuingRuleAttemptTransition::OnceReturnStateMismatch(
                BorrowedOnceReturnStateMismatchRuleAttempt {
                    attempt,
                    rule,
                    cursor,
                },
            )
        }
        EvaluatedRuleMiss::OnceRewriteConsumed(rule) => {
            BorrowedContinuingRuleAttemptTransition::OnceRewriteConsumed(
                BorrowedOnceRewriteConsumedRuleAttempt {
                    attempt,
                    rule,
                    cursor,
                },
            )
        }
    }
}

/// Commits a final rule-attempt miss and returns its terminal miss witness.
fn commit_final_miss<'program, E, A, Pass>(
    program: &'program ExecutableProgram,
    parts: AttemptRunCoreParts<E, A>,
    pass: Pass,
    attempts: RuleAttemptCount,
    miss: EvaluatedRuleMiss<'program>,
) -> BorrowedFinalRuleAttemptTransition<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass: FinalRuleAttemptPass<'program>,
{
    let core = parts.with_pass(pass);
    let core = core.into_terminal();
    match miss {
        EvaluatedRuleMiss::AlwaysRewriteStateMismatch(rule) => {
            BorrowedFinalRuleAttemptTransition::StableAfterAlwaysRewriteStateMismatch(
                BorrowedRuleAttemptStableAfterAlwaysRewriteStateMismatch {
                    attempts,
                    rule,
                    program,
                    core,
                },
            )
        }
        EvaluatedRuleMiss::OnceRewriteStateMismatch(rule) => {
            BorrowedFinalRuleAttemptTransition::StableAfterOnceRewriteStateMismatch(
                BorrowedRuleAttemptStableAfterOnceRewriteStateMismatch {
                    attempts,
                    rule,
                    program,
                    core,
                },
            )
        }
        EvaluatedRuleMiss::AlwaysReturnStateMismatch(rule) => {
            BorrowedFinalRuleAttemptTransition::StableAfterAlwaysReturnStateMismatch(
                BorrowedRuleAttemptStableAfterAlwaysReturnStateMismatch {
                    attempts,
                    rule,
                    program,
                    core,
                },
            )
        }
        EvaluatedRuleMiss::OnceReturnStateMismatch(rule) => {
            BorrowedFinalRuleAttemptTransition::StableAfterOnceReturnStateMismatch(
                BorrowedRuleAttemptStableAfterOnceReturnStateMismatch {
                    attempts,
                    rule,
                    program,
                    core,
                },
            )
        }
        EvaluatedRuleMiss::OnceRewriteConsumed(rule) => {
            BorrowedFinalRuleAttemptTransition::StableAfterOnceRewriteConsumed(
                BorrowedRuleAttemptStableAfterOnceRewriteConsumed {
                    attempts,
                    rule,
                    program,
                    core,
                },
            )
        }
    }
}

/// Builds the canonical continuing-pass transition for a committed rule application.
fn committed_continuing_rule_attempt_application<'program, E, A, Pass>(
    program: &'program ExecutableProgram,
    core: AttemptRunCore<E, A, Pass>,
    attempt: RuleAttemptCount,
    applied: AppliedRule<'program>,
) -> BorrowedContinuingRuleAttemptTransition<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass: ContinuingRuleAttemptPass<'program>,
{
    match applied {
        AppliedRule::AlwaysRewritten(committed) => {
            let step = committed.step();
            let rule = committed.rule();
            let (parts, runtime_rules) = core.into_parts();
            let cursor = BorrowedRuleAttemptCursor::from_first_parts(
                program,
                parts,
                runtime_rules.reset_attempt_after_rewrite(),
            );
            BorrowedContinuingRuleAttemptTransition::AlwaysRewritten(
                BorrowedRuleAttemptAlwaysRewriteStep {
                    attempt,
                    step,
                    rule,
                    cursor,
                },
            )
        }
        AppliedRule::OnceRewritten(committed) => {
            let step = committed.step();
            let rule = committed.rule();
            let (parts, runtime_rules) = core.into_parts();
            let cursor = BorrowedRuleAttemptCursor::from_first_parts(
                program,
                parts,
                runtime_rules.reset_attempt_after_rewrite(),
            );
            BorrowedContinuingRuleAttemptTransition::OnceRewritten(
                BorrowedRuleAttemptOnceRewriteStep {
                    attempt,
                    step,
                    rule,
                    cursor,
                },
            )
        }
        AppliedRule::AlwaysReturned(committed) => {
            let step = committed.step();
            let rule = committed.rule();
            let output = committed.into_output();
            BorrowedContinuingRuleAttemptTransition::AlwaysReturned(
                BorrowedRuleAttemptAlwaysReturnRun {
                    attempt,
                    step,
                    rule,
                    program,
                    output,
                },
            )
        }
        AppliedRule::OnceReturned(committed) => {
            let step = committed.step();
            let rule = committed.rule();
            let output = committed.into_output();
            BorrowedContinuingRuleAttemptTransition::OnceReturned(
                BorrowedRuleAttemptOnceReturnRun {
                    attempt,
                    step,
                    rule,
                    program,
                    output,
                },
            )
        }
    }
}

/// Builds the canonical final-pass transition for a committed rule application.
fn committed_final_rule_attempt_application<'program, E, A, Pass>(
    program: &'program ExecutableProgram,
    core: AttemptRunCore<E, A, Pass>,
    attempt: RuleAttemptCount,
    applied: AppliedRule<'program>,
) -> BorrowedFinalRuleAttemptTransition<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass: FinalRuleAttemptPass<'program>,
{
    match applied {
        AppliedRule::AlwaysRewritten(committed) => {
            let step = committed.step();
            let rule = committed.rule();
            let (parts, runtime_rules) = core.into_parts();
            let cursor = BorrowedRuleAttemptCursor::from_first_parts(
                program,
                parts,
                runtime_rules.reset_attempt_after_rewrite(),
            );
            BorrowedFinalRuleAttemptTransition::AlwaysRewritten(
                BorrowedRuleAttemptAlwaysRewriteStep {
                    attempt,
                    step,
                    rule,
                    cursor,
                },
            )
        }
        AppliedRule::OnceRewritten(committed) => {
            let step = committed.step();
            let rule = committed.rule();
            let (parts, runtime_rules) = core.into_parts();
            let cursor = BorrowedRuleAttemptCursor::from_first_parts(
                program,
                parts,
                runtime_rules.reset_attempt_after_rewrite(),
            );
            BorrowedFinalRuleAttemptTransition::OnceRewritten(BorrowedRuleAttemptOnceRewriteStep {
                attempt,
                step,
                rule,
                cursor,
            })
        }
        AppliedRule::AlwaysReturned(committed) => {
            let step = committed.step();
            let rule = committed.rule();
            let output = committed.into_output();
            BorrowedFinalRuleAttemptTransition::AlwaysReturned(BorrowedRuleAttemptAlwaysReturnRun {
                attempt,
                step,
                rule,
                program,
                output,
            })
        }
        AppliedRule::OnceReturned(committed) => {
            let step = committed.step();
            let rule = committed.rule();
            let output = committed.into_output();
            BorrowedFinalRuleAttemptTransition::OnceReturned(BorrowedRuleAttemptOnceReturnRun {
                attempt,
                step,
                rule,
                program,
                output,
            })
        }
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
) -> Result<
    (
        RuleAttemptCount,
        PreparedRuleStep<'program, 'once, 'budget, E>,
    ),
    RuleAttemptStepError,
>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    let prepared = prepare_matched_rule(scratch, budget, state_len, matched)?;
    let attempt = attempt_reservation.commit();
    Ok((attempt, prepared))
}
