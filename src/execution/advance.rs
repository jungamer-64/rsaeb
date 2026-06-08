use crate::error::RuleAttemptStepError;
use crate::limits::RuleAttemptCount;
use crate::policy::{ExecutionPolicy, RuleAttemptPolicy};
use crate::program::ExecutableProgram;
use crate::runtime::action::{AppliedRule, PreparedRuleStep, prepare_matched_rule};
use crate::runtime::budget::RuleAttemptReservation;
use crate::runtime::matcher::{EvaluatedRuleMiss, RuleAttemptEvaluation};
use crate::runtime::once::{ContinuingRuntimeRulePass, FinalRuntimeRulePass};
use crate::runtime::rewrite::RewriteScratch;
use crate::runtime::state::State;

use super::engine::{
    AttemptRunCoreParts, ContinuingRuleAttemptCore, ContinuingRuleAttemptRun, FinalRuleAttemptCore,
    FinalRuleAttemptRun,
};
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
pub(super) fn advance_continuing_borrowed_rule_attempt<'program, E, A>(
    session: ContinuingRuleAttemptRun<'program, E, A>,
) -> BorrowedContinuingRuleAttemptTransition<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    advance_continuing_rule_attempt(session)
}

/// Advances a borrowed rule-attempt session whose current rule exhausts the pass.
pub(super) fn advance_final_borrowed_rule_attempt<'program, E, A>(
    session: FinalRuleAttemptRun<'program, E, A>,
) -> BorrowedFinalRuleAttemptTransition<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    advance_final_rule_attempt(session)
}

/// Advances a rule-attempt step whose selected rule is not final in the pass.
fn advance_continuing_rule_attempt<'program, E, A>(
    session: ContinuingRuleAttemptRun<'program, E, A>,
) -> BorrowedContinuingRuleAttemptTransition<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    let ContinuingRuleAttemptRun { program, core } = session;
    let ContinuingRuleAttemptCore {
        mut parts,
        runtime_rules: mut pass,
    } = core;

    let reservation = match parts
        .attempt_budget
        .reserve_next_attempt(parts.state.byte_count())
    {
        Ok(reservation) => reservation,
        Err(error) => {
            return failed_continuing_rule_attempt(program, parts, error);
        }
    };

    match pass.attempt_current_rule(&parts.state) {
        RuleAttemptEvaluation::Miss(miss) => {
            let attempt = reservation.commit();
            commit_continuing_miss(program, parts, pass, attempt, miss)
        }
        RuleAttemptEvaluation::Matched(matched) => {
            let state_len = parts.state.byte_count();
            let prepared = match prepare_matched_rule(
                &mut parts.scratch,
                &mut parts.budget,
                state_len,
                matched,
            ) {
                Ok(prepared) => prepared,
                Err(error) => {
                    return failed_continuing_rule_attempt(program, parts, error.into());
                }
            };
            let (attempt, applied) = commit_prepared_rule_attempt_application(
                &mut parts.state,
                &mut parts.scratch,
                reservation,
                prepared,
            );
            committed_continuing_rule_attempt_application(program, parts, pass, attempt, applied)
        }
    }
}

/// Advances a rule-attempt step whose selected rule exhausts the pass.
fn advance_final_rule_attempt<'program, E, A>(
    session: FinalRuleAttemptRun<'program, E, A>,
) -> BorrowedFinalRuleAttemptTransition<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    let FinalRuleAttemptRun { program, core } = session;
    let FinalRuleAttemptCore {
        mut parts,
        runtime_rules: mut pass,
    } = core;

    let reservation = match parts
        .attempt_budget
        .reserve_next_attempt(parts.state.byte_count())
    {
        Ok(reservation) => reservation,
        Err(error) => {
            return failed_final_rule_attempt(program, parts, error);
        }
    };

    match pass.attempt_current_rule(&parts.state) {
        RuleAttemptEvaluation::Miss(miss) => {
            let attempts = reservation.commit();
            commit_final_miss(program, parts, pass, attempts, miss)
        }
        RuleAttemptEvaluation::Matched(matched) => {
            let state_len = parts.state.byte_count();
            let prepared = match prepare_matched_rule(
                &mut parts.scratch,
                &mut parts.budget,
                state_len,
                matched,
            ) {
                Ok(prepared) => prepared,
                Err(error) => {
                    return failed_final_rule_attempt(program, parts, error.into());
                }
            };
            let (attempt, applied) = commit_prepared_rule_attempt_application(
                &mut parts.state,
                &mut parts.scratch,
                reservation,
                prepared,
            );
            committed_final_rule_attempt_application(program, parts, pass, attempt, applied)
        }
    }
}

/// Reports a continuing-pass rule-attempt failure with the uncommitted runtime state.
fn failed_continuing_rule_attempt<'program, E, A>(
    program: &'program ExecutableProgram,
    parts: AttemptRunCoreParts<E, A>,
    error: RuleAttemptStepError,
) -> BorrowedContinuingRuleAttemptTransition<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    let attempts = parts.completed_attempts();
    BorrowedContinuingRuleAttemptTransition::Failed(BorrowedRuleAttemptFailedRun::new(
        error,
        attempts,
        program,
        parts.into_terminal(),
    ))
}

/// Reports a final-pass rule-attempt failure with the uncommitted runtime state.
fn failed_final_rule_attempt<'program, E, A>(
    program: &'program ExecutableProgram,
    parts: AttemptRunCoreParts<E, A>,
    error: RuleAttemptStepError,
) -> BorrowedFinalRuleAttemptTransition<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    let attempts = parts.completed_attempts();
    BorrowedFinalRuleAttemptTransition::Failed(BorrowedRuleAttemptFailedRun::new(
        error,
        attempts,
        program,
        parts.into_terminal(),
    ))
}

/// Commits a non-final rule-attempt miss and returns its resumable miss witness.
fn commit_continuing_miss<'program, E, A>(
    program: &'program ExecutableProgram,
    parts: AttemptRunCoreParts<E, A>,
    pass: ContinuingRuntimeRulePass<'program>,
    attempt: RuleAttemptCount,
    miss: EvaluatedRuleMiss<'program>,
) -> BorrowedContinuingRuleAttemptTransition<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    let runtime_rules = pass.commit_miss();
    let cursor = BorrowedRuleAttemptCursor::from_parts(program, parts, runtime_rules);
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
fn commit_final_miss<'program, E, A>(
    program: &'program ExecutableProgram,
    parts: AttemptRunCoreParts<E, A>,
    _pass: FinalRuntimeRulePass<'program>,
    attempts: RuleAttemptCount,
    miss: EvaluatedRuleMiss<'program>,
) -> BorrowedFinalRuleAttemptTransition<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    let core = parts.into_terminal();
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

/// Builds the continuing transition for one committed rule-attempt application.
fn committed_continuing_rule_attempt_application<'program, E, A>(
    program: &'program ExecutableProgram,
    parts: AttemptRunCoreParts<E, A>,
    pass: ContinuingRuntimeRulePass<'program>,
    attempt: RuleAttemptCount,
    applied: AppliedRule<'program>,
) -> BorrowedContinuingRuleAttemptTransition<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    match applied {
        AppliedRule::AlwaysRewritten(committed) => {
            let step = committed.step();
            let rule = committed.rule();
            let cursor = cursor_after_rewrite(program, parts, pass.reset_after_rewrite());
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
            let cursor = cursor_after_rewrite(program, parts, pass.reset_after_rewrite());
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

/// Builds the final transition for one committed rule-attempt application.
fn committed_final_rule_attempt_application<'program, E, A>(
    program: &'program ExecutableProgram,
    parts: AttemptRunCoreParts<E, A>,
    pass: FinalRuntimeRulePass<'program>,
    attempt: RuleAttemptCount,
    applied: AppliedRule<'program>,
) -> BorrowedFinalRuleAttemptTransition<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    match applied {
        AppliedRule::AlwaysRewritten(committed) => {
            let step = committed.step();
            let rule = committed.rule();
            let cursor = cursor_after_rewrite(program, parts, pass.reset_after_rewrite());
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
            let cursor = cursor_after_rewrite(program, parts, pass.reset_after_rewrite());
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

/// Rebuilds the public cursor after a rewrite resets the rule-attempt pass.
fn cursor_after_rewrite<'program, E, A>(
    program: &'program ExecutableProgram,
    parts: AttemptRunCoreParts<E, A>,
    cursor: crate::runtime::once::RuntimeRulePassCursor<'program>,
) -> BorrowedRuleAttemptCursor<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    BorrowedRuleAttemptCursor::from_parts(program, parts, cursor)
}

/// Commits one prepared rule-attempt application.
///
/// This function is called only after rule preparation succeeds. The
/// rule-attempt reservation commits first, followed by runtime step,
/// once-state, and state side effects.
fn commit_prepared_rule_attempt_application<'program, 'once, 'budget, E, A>(
    state: &mut State,
    scratch: &mut RewriteScratch,
    attempt_reservation: RuleAttemptReservation<'_, A>,
    prepared: PreparedRuleStep<'program, 'once, 'budget, E>,
) -> (RuleAttemptCount, AppliedRule<'program>)
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    let attempt = attempt_reservation.commit();
    let applied = prepared.commit(state, scratch);
    (attempt, applied)
}
