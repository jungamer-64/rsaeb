use crate::error::RuleAttemptStepError;
use crate::limits::RuleAttemptCount;
use crate::policy::{ExecutionPolicy, RuleAttemptPolicy};
use crate::program::ExecutableProgram;
use crate::runtime::action::{AppliedRule, PreparedRuleStep, prepare_matched_rule};
use crate::runtime::budget::RuleAttemptReservation;
use crate::runtime::matcher::{EvaluatedRuleMiss, RuleAttemptEvaluation};
use crate::runtime::once::{ContinuingRuleAttemptPass, FinalRuleAttemptPass, RuleAttemptPass};
use crate::runtime::rewrite::RewriteScratch;
use crate::runtime::state::State;

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
            let prepared = match prepare_matched_rule(
                &mut parts.scratch,
                &mut parts.budget,
                state_len,
                matched,
            ) {
                Ok(prepared) => prepared,
                Err(error) => {
                    let core = parts.with_pass(pass);
                    return failed_continuing_rule_attempt(program, core, error.into());
                }
            };
            let (attempt, applied) = commit_prepared_rule_attempt_application(
                &mut parts.state,
                &mut parts.scratch,
                reservation,
                prepared,
            );
            let core = parts.with_pass(pass);
            committed_rule_attempt_application::<E, A, Pass, ContinuingRuleAttemptSuccessTarget>(
                program, core, attempt, applied,
            )
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
            let prepared = match prepare_matched_rule(
                &mut parts.scratch,
                &mut parts.budget,
                state_len,
                matched,
            ) {
                Ok(prepared) => prepared,
                Err(error) => {
                    let core = parts.with_pass(pass);
                    return failed_final_rule_attempt(program, core, error.into());
                }
            };
            let (attempt, applied) = commit_prepared_rule_attempt_application(
                &mut parts.state,
                &mut parts.scratch,
                reservation,
                prepared,
            );
            let core = parts.with_pass(pass);
            committed_rule_attempt_application::<E, A, Pass, FinalRuleAttemptSuccessTarget>(
                program, core, attempt, applied,
            )
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

/// Selected public transition target for committed rule-attempt successes.
trait RuleAttemptSuccessTarget<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// Public transition enum built by this target.
    type Transition;

    /// Wraps a committed reusable rewrite witness.
    fn always_rewritten(
        step: BorrowedRuleAttemptAlwaysRewriteStep<'program, E, A>,
    ) -> Self::Transition;

    /// Wraps a committed once-only rewrite witness.
    fn once_rewritten(step: BorrowedRuleAttemptOnceRewriteStep<'program, E, A>)
    -> Self::Transition;

    /// Wraps a committed reusable return witness.
    fn always_returned(run: BorrowedRuleAttemptAlwaysReturnRun<'program>) -> Self::Transition;

    /// Wraps a committed once-only return witness.
    fn once_returned(run: BorrowedRuleAttemptOnceReturnRun<'program>) -> Self::Transition;
}

/// Continuing-pass public success transition target.
struct ContinuingRuleAttemptSuccessTarget;

/// Final-pass public success transition target.
struct FinalRuleAttemptSuccessTarget;

impl<'program, E, A> RuleAttemptSuccessTarget<'program, E, A> for ContinuingRuleAttemptSuccessTarget
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    type Transition = BorrowedContinuingRuleAttemptTransition<'program, E, A>;

    fn always_rewritten(
        step: BorrowedRuleAttemptAlwaysRewriteStep<'program, E, A>,
    ) -> Self::Transition {
        BorrowedContinuingRuleAttemptTransition::AlwaysRewritten(step)
    }

    fn once_rewritten(
        step: BorrowedRuleAttemptOnceRewriteStep<'program, E, A>,
    ) -> Self::Transition {
        BorrowedContinuingRuleAttemptTransition::OnceRewritten(step)
    }

    fn always_returned(run: BorrowedRuleAttemptAlwaysReturnRun<'program>) -> Self::Transition {
        BorrowedContinuingRuleAttemptTransition::AlwaysReturned(run)
    }

    fn once_returned(run: BorrowedRuleAttemptOnceReturnRun<'program>) -> Self::Transition {
        BorrowedContinuingRuleAttemptTransition::OnceReturned(run)
    }
}

impl<'program, E, A> RuleAttemptSuccessTarget<'program, E, A> for FinalRuleAttemptSuccessTarget
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    type Transition = BorrowedFinalRuleAttemptTransition<'program, E, A>;

    fn always_rewritten(
        step: BorrowedRuleAttemptAlwaysRewriteStep<'program, E, A>,
    ) -> Self::Transition {
        BorrowedFinalRuleAttemptTransition::AlwaysRewritten(step)
    }

    fn once_rewritten(
        step: BorrowedRuleAttemptOnceRewriteStep<'program, E, A>,
    ) -> Self::Transition {
        BorrowedFinalRuleAttemptTransition::OnceRewritten(step)
    }

    fn always_returned(run: BorrowedRuleAttemptAlwaysReturnRun<'program>) -> Self::Transition {
        BorrowedFinalRuleAttemptTransition::AlwaysReturned(run)
    }

    fn once_returned(run: BorrowedRuleAttemptOnceReturnRun<'program>) -> Self::Transition {
        BorrowedFinalRuleAttemptTransition::OnceReturned(run)
    }
}

/// Builds the canonical transition for one committed rule-attempt application.
fn committed_rule_attempt_application<'program, E, A, Pass, Target>(
    program: &'program ExecutableProgram,
    core: AttemptRunCore<E, A, Pass>,
    attempt: RuleAttemptCount,
    applied: AppliedRule<'program>,
) -> Target::Transition
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass: RuleAttemptPass<'program>,
    Target: RuleAttemptSuccessTarget<'program, E, A>,
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
            Target::always_rewritten(BorrowedRuleAttemptAlwaysRewriteStep {
                attempt,
                step,
                rule,
                cursor,
            })
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
            Target::once_rewritten(BorrowedRuleAttemptOnceRewriteStep {
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
            Target::always_returned(BorrowedRuleAttemptAlwaysReturnRun {
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
            Target::once_returned(BorrowedRuleAttemptOnceReturnRun {
                attempt,
                step,
                rule,
                program,
                output,
            })
        }
    }
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
