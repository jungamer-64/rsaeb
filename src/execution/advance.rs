use crate::bytes::RuntimeStateByteCount;
use crate::error::RuleAttemptStepError;
use crate::inspect::{
    AlwaysReturnRuleView, AlwaysRewriteRuleView, OnceReturnRuleView, OnceRewriteRuleView,
};
use crate::limits::RuleAttemptCount;
use crate::policy::{ExecutionPolicy, RuleAttemptPolicy};
use crate::program::ExecutableProgram;
use crate::runtime::action::{AppliedRule, PreparedRuleStep, prepare_matched_rule};
use crate::runtime::budget::{RuleAttemptReservation, RuntimeBudgetState};
use crate::runtime::matcher::{EvaluatedRuleMiss, MatchedRuleApplication, RuleAttemptEvaluation};
use crate::runtime::once::{
    AfterMissContinuingRulePass, AfterMissFinalRulePass, FirstContinuingRulePass,
    FirstFinalRulePass, FirstRuntimeRulePassCursor, MissedRuntimeRulePassCursor,
    RuntimeRulePassState,
};
use crate::runtime::rewrite::RewriteScratch;
use crate::runtime::state::State;

use super::engine::{AttemptRunCore, AttemptRunCoreParts, AttemptSession, TerminalAttemptSession};
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

/// Continuing rule-attempt pass behavior shared by first and after-miss states.
pub(super) trait ContinuingRuleAttemptPass<'program>:
    RuntimeRulePassState<'program> + Sized
{
    /// Attempts this pass's current target.
    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttemptEvaluation<'program, 'state, 'once>;

    /// Commits a miss and advances to the next typed pass.
    fn commit_attempt_miss(self) -> MissedRuntimeRulePassCursor<'program>;

    /// Resets this pass after a committed rewrite.
    fn reset_attempt_after_rewrite(self) -> FirstRuntimeRulePassCursor<'program>;
}

/// Final rule-attempt pass behavior shared by first and after-miss states.
pub(super) trait FinalRuleAttemptPass<'program>:
    RuntimeRulePassState<'program> + Sized
{
    /// Attempts this pass's current target.
    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttemptEvaluation<'program, 'state, 'once>;

    /// Resets this pass after a committed rewrite.
    fn reset_attempt_after_rewrite(self) -> FirstRuntimeRulePassCursor<'program>;
}

impl<'program> ContinuingRuleAttemptPass<'program> for FirstContinuingRulePass<'program> {
    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttemptEvaluation<'program, 'state, 'once> {
        self.attempt_current(state)
    }

    fn commit_attempt_miss(self) -> MissedRuntimeRulePassCursor<'program> {
        self.commit_miss()
    }

    fn reset_attempt_after_rewrite(self) -> FirstRuntimeRulePassCursor<'program> {
        self.reset_after_rewrite()
    }
}

impl<'program> ContinuingRuleAttemptPass<'program> for AfterMissContinuingRulePass<'program> {
    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttemptEvaluation<'program, 'state, 'once> {
        self.attempt_current(state)
    }

    fn commit_attempt_miss(self) -> MissedRuntimeRulePassCursor<'program> {
        self.commit_miss()
    }

    fn reset_attempt_after_rewrite(self) -> FirstRuntimeRulePassCursor<'program> {
        self.reset_after_rewrite()
    }
}

impl<'program> FinalRuleAttemptPass<'program> for FirstFinalRulePass<'program> {
    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttemptEvaluation<'program, 'state, 'once> {
        self.attempt_current(state)
    }

    fn reset_attempt_after_rewrite(self) -> FirstRuntimeRulePassCursor<'program> {
        self.reset_after_rewrite()
    }
}

impl<'program> FinalRuleAttemptPass<'program> for AfterMissFinalRulePass<'program> {
    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttemptEvaluation<'program, 'state, 'once> {
        self.attempt_current(state)
    }

    fn reset_attempt_after_rewrite(self) -> FirstRuntimeRulePassCursor<'program> {
        self.reset_after_rewrite()
    }
}

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
            commit_continuing_miss(program, parts, pass, attempt, miss).into_transition()
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
            commit_final_miss(program, parts, pass, attempts, miss).into_transition()
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

/// Committed continuing-pass miss that can only resume with another typed cursor.
struct CommittedContinuingRuleAttemptMiss<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// Rule-attempt count committed by this transition.
    attempt: RuleAttemptCount,
    /// Exact miss shape produced by evaluating the current rule.
    miss: EvaluatedRuleMiss<'program>,
    /// Cursor after consuming the non-final rule line.
    cursor: BorrowedRuleAttemptCursor<'program, E, A>,
}

/// Committed final-pass miss that can only terminate as stable.
struct CommittedFinalRuleAttemptMiss<'program> {
    /// Exact miss shape produced by evaluating the final rule.
    miss: EvaluatedRuleMiss<'program>,
    /// Terminal state after the final rule line is consumed.
    terminal: TerminalAttemptSession<'program>,
}

impl<'program, E: ExecutionPolicy, A: RuleAttemptPolicy>
    CommittedContinuingRuleAttemptMiss<'program, E, A>
{
    /// Converts this continuing miss into its only valid public transition.
    fn into_transition(self) -> BorrowedContinuingRuleAttemptTransition<'program, E, A> {
        let Self {
            attempt,
            miss,
            cursor,
        } = self;
        visit_evaluated_rule_miss(
            miss,
            ContinuingRuleAttemptMissTransition { attempt, cursor },
        )
    }
}

impl<'program> CommittedFinalRuleAttemptMiss<'program> {
    /// Converts this final miss into its only valid public transition.
    fn into_transition<E, A>(self) -> BorrowedFinalRuleAttemptTransition<'program, E, A>
    where
        E: ExecutionPolicy,
        A: RuleAttemptPolicy,
    {
        let Self { miss, terminal } = self;
        let TerminalAttemptSession {
            program,
            core,
            attempts,
        } = terminal;
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
}

/// Exact constructor target for one evaluated miss shape.
trait EvaluatedRuleMissVisitor<'program> {
    /// Transition type produced by this exact destination.
    type Output;

    /// Constructs the target from a reusable rewrite state mismatch.
    fn always_rewrite_state_mismatch(self, rule: AlwaysRewriteRuleView<'program>) -> Self::Output;

    /// Constructs the target from a once-only rewrite state mismatch.
    fn once_rewrite_state_mismatch(self, rule: OnceRewriteRuleView<'program>) -> Self::Output;

    /// Constructs the target from a reusable return state mismatch.
    fn always_return_state_mismatch(self, rule: AlwaysReturnRuleView<'program>) -> Self::Output;

    /// Constructs the target from a once-only return state mismatch.
    fn once_return_state_mismatch(self, rule: OnceReturnRuleView<'program>) -> Self::Output;

    /// Constructs the target from an already-consumed once-only rewrite rule.
    fn once_rewrite_consumed(self, rule: OnceRewriteRuleView<'program>) -> Self::Output;
}

/// Visits an exact evaluated miss without exposing a generic transition helper.
fn visit_evaluated_rule_miss<'program, V>(
    miss: EvaluatedRuleMiss<'program>,
    visitor: V,
) -> V::Output
where
    V: EvaluatedRuleMissVisitor<'program>,
{
    match miss {
        EvaluatedRuleMiss::AlwaysRewriteStateMismatch(rule) => {
            visitor.always_rewrite_state_mismatch(rule)
        }
        EvaluatedRuleMiss::OnceRewriteStateMismatch(rule) => {
            visitor.once_rewrite_state_mismatch(rule)
        }
        EvaluatedRuleMiss::AlwaysReturnStateMismatch(rule) => {
            visitor.always_return_state_mismatch(rule)
        }
        EvaluatedRuleMiss::OnceReturnStateMismatch(rule) => {
            visitor.once_return_state_mismatch(rule)
        }
        EvaluatedRuleMiss::OnceRewriteConsumed(rule) => visitor.once_rewrite_consumed(rule),
    }
}

/// Continuing transition target for evaluated rule-attempt misses.
struct ContinuingRuleAttemptMissTransition<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// Rule-attempt count committed by this transition.
    attempt: RuleAttemptCount,
    /// Cursor after consuming the non-final rule line.
    cursor: BorrowedRuleAttemptCursor<'program, E, A>,
}

impl<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> EvaluatedRuleMissVisitor<'program>
    for ContinuingRuleAttemptMissTransition<'program, E, A>
{
    type Output = BorrowedContinuingRuleAttemptTransition<'program, E, A>;

    fn always_rewrite_state_mismatch(self, rule: AlwaysRewriteRuleView<'program>) -> Self::Output {
        BorrowedContinuingRuleAttemptTransition::AlwaysRewriteStateMismatch(
            BorrowedAlwaysRewriteStateMismatchRuleAttempt {
                attempt: self.attempt,
                rule,
                cursor: self.cursor,
            },
        )
    }

    fn once_rewrite_state_mismatch(self, rule: OnceRewriteRuleView<'program>) -> Self::Output {
        BorrowedContinuingRuleAttemptTransition::OnceRewriteStateMismatch(
            BorrowedOnceRewriteStateMismatchRuleAttempt {
                attempt: self.attempt,
                rule,
                cursor: self.cursor,
            },
        )
    }

    fn always_return_state_mismatch(self, rule: AlwaysReturnRuleView<'program>) -> Self::Output {
        BorrowedContinuingRuleAttemptTransition::AlwaysReturnStateMismatch(
            BorrowedAlwaysReturnStateMismatchRuleAttempt {
                attempt: self.attempt,
                rule,
                cursor: self.cursor,
            },
        )
    }

    fn once_return_state_mismatch(self, rule: OnceReturnRuleView<'program>) -> Self::Output {
        BorrowedContinuingRuleAttemptTransition::OnceReturnStateMismatch(
            BorrowedOnceReturnStateMismatchRuleAttempt {
                attempt: self.attempt,
                rule,
                cursor: self.cursor,
            },
        )
    }

    fn once_rewrite_consumed(self, rule: OnceRewriteRuleView<'program>) -> Self::Output {
        BorrowedContinuingRuleAttemptTransition::OnceRewriteConsumed(
            BorrowedOnceRewriteConsumedRuleAttempt {
                attempt: self.attempt,
                rule,
                cursor: self.cursor,
            },
        )
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
) -> CommittedContinuingRuleAttemptMiss<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass: ContinuingRuleAttemptPass<'program>,
{
    let runtime_rules = pass.commit_attempt_miss();
    CommittedContinuingRuleAttemptMiss {
        attempt,
        miss,
        cursor: BorrowedRuleAttemptCursor::from_after_miss_parts(program, parts, runtime_rules),
    }
}

/// Commits a final rule-attempt miss and returns its terminal miss witness.
fn commit_final_miss<'program, E, A, Pass>(
    program: &'program ExecutableProgram,
    parts: AttemptRunCoreParts<E, A>,
    pass: Pass,
    attempts: RuleAttemptCount,
    miss: EvaluatedRuleMiss<'program>,
) -> CommittedFinalRuleAttemptMiss<'program>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass: FinalRuleAttemptPass<'program>,
{
    let core = parts.with_pass(pass);
    CommittedFinalRuleAttemptMiss {
        miss,
        terminal: TerminalAttemptSession {
            program,
            core: core.into_terminal(),
            attempts,
        },
    }
}

/// Projects a continuing-pass committed rule application into the next rule-attempt state.
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

/// Projects a final-pass committed rule application into the next rule-attempt state.
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
