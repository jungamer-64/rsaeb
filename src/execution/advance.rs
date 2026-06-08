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
    let ContinuingRuleAttemptRun { program, core } = session;
    let ContinuingRuleAttemptCore {
        parts,
        runtime_rules: pass,
    } = core;
    advance_rule_attempt(program, parts, ContinuingRuleAttemptDestination { pass })
}

/// Advances a borrowed rule-attempt session whose current rule exhausts the pass.
pub(super) fn advance_final_borrowed_rule_attempt<'program, E, A>(
    session: FinalRuleAttemptRun<'program, E, A>,
) -> BorrowedFinalRuleAttemptTransition<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    let FinalRuleAttemptRun { program, core } = session;
    let FinalRuleAttemptCore {
        parts,
        runtime_rules: pass,
    } = core;
    advance_rule_attempt(program, parts, FinalRuleAttemptDestination { pass })
}

/// Rule-attempt destination whose current runtime rule has a successor.
struct ContinuingRuleAttemptDestination<'program> {
    /// Runtime pass that can continue after a miss.
    pass: ContinuingRuntimeRulePass<'program>,
}

/// Rule-attempt destination whose current runtime rule exhausts the pass.
struct FinalRuleAttemptDestination<'program> {
    /// Runtime pass that can only stabilize after a miss.
    pass: FinalRuntimeRulePass<'program>,
}

/// Private destination boundary for rule-attempt transition projection.
trait RuleAttemptDestination<'program, E, A>: destination::Sealed
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    /// Public transition type selected by this destination shape.
    type Transition;

    /// Evaluates the destination's current runtime rule against `state`.
    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttemptEvaluation<'program, 'state, 'once>;

    /// Projects an uncommitted failure into this destination's terminal transition.
    fn project_failure(
        self,
        program: &'program ExecutableProgram,
        parts: AttemptRunCoreParts<E, A>,
        error: RuleAttemptStepError,
    ) -> Self::Transition;

    /// Projects a committed miss into this destination's legal miss transition.
    fn project_miss(
        self,
        program: &'program ExecutableProgram,
        parts: AttemptRunCoreParts<E, A>,
        attempt: RuleAttemptCount,
        miss: EvaluatedRuleMiss<'program>,
    ) -> Self::Transition;

    /// Projects a committed rule application into this destination's legal transition.
    fn project_applied(
        self,
        program: &'program ExecutableProgram,
        parts: AttemptRunCoreParts<E, A>,
        attempt: RuleAttemptCount,
        applied: AppliedRule<'program>,
    ) -> Self::Transition;
}

/// Seals rule-attempt destination projection to the two pass shapes.
mod destination {
    /// Marker implemented only for valid rule-attempt destinations.
    pub(super) trait Sealed {}
}

impl destination::Sealed for ContinuingRuleAttemptDestination<'_> {}
impl destination::Sealed for FinalRuleAttemptDestination<'_> {}

/// Advances one typed rule-attempt destination.
fn advance_rule_attempt<'program, E, A, D>(
    program: &'program ExecutableProgram,
    mut parts: AttemptRunCoreParts<E, A>,
    mut destination: D,
) -> D::Transition
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    D: RuleAttemptDestination<'program, E, A>,
{
    let reservation = match parts
        .attempt_budget
        .reserve_next_attempt(parts.state.byte_count())
    {
        Ok(reservation) => reservation,
        Err(error) => {
            return destination.project_failure(program, parts, error);
        }
    };

    match destination.attempt_current_rule(&parts.state) {
        RuleAttemptEvaluation::Miss(miss) => {
            let attempt = reservation.commit();
            destination.project_miss(program, parts, attempt, miss)
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
                    return destination.project_failure(program, parts, error.into());
                }
            };
            let (attempt, applied) = commit_prepared_rule_attempt_application(
                &mut parts.state,
                &mut parts.scratch,
                reservation,
                prepared,
            );
            destination.project_applied(program, parts, attempt, applied)
        }
    }
}

impl<'program, E, A> RuleAttemptDestination<'program, E, A>
    for ContinuingRuleAttemptDestination<'program>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    type Transition = BorrowedContinuingRuleAttemptTransition<'program, E, A>;

    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttemptEvaluation<'program, 'state, 'once> {
        self.pass.attempt_current_rule(state)
    }

    fn project_failure(
        self,
        program: &'program ExecutableProgram,
        parts: AttemptRunCoreParts<E, A>,
        error: RuleAttemptStepError,
    ) -> Self::Transition {
        let attempts = parts.completed_attempts();
        BorrowedContinuingRuleAttemptTransition::Failed(BorrowedRuleAttemptFailedRun::new(
            error,
            attempts,
            program,
            parts.into_terminal(),
        ))
    }

    fn project_miss(
        self,
        program: &'program ExecutableProgram,
        parts: AttemptRunCoreParts<E, A>,
        attempt: RuleAttemptCount,
        miss: EvaluatedRuleMiss<'program>,
    ) -> Self::Transition {
        let runtime_rules = self.pass.commit_miss();
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

    fn project_applied(
        self,
        program: &'program ExecutableProgram,
        parts: AttemptRunCoreParts<E, A>,
        attempt: RuleAttemptCount,
        applied: AppliedRule<'program>,
    ) -> Self::Transition {
        let pass = self.pass;
        match applied {
            AppliedRule::AlwaysRewritten(committed) => {
                let step = committed.step();
                let rule = committed.rule();
                let cursor = BorrowedRuleAttemptCursor::from_parts(
                    program,
                    parts,
                    pass.reset_after_rewrite(),
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
                let cursor = BorrowedRuleAttemptCursor::from_parts(
                    program,
                    parts,
                    pass.reset_after_rewrite(),
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
}

impl<'program, E, A> RuleAttemptDestination<'program, E, A>
    for FinalRuleAttemptDestination<'program>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    type Transition = BorrowedFinalRuleAttemptTransition<'program, E, A>;

    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttemptEvaluation<'program, 'state, 'once> {
        self.pass.attempt_current_rule(state)
    }

    fn project_failure(
        self,
        program: &'program ExecutableProgram,
        parts: AttemptRunCoreParts<E, A>,
        error: RuleAttemptStepError,
    ) -> Self::Transition {
        let attempts = parts.completed_attempts();
        BorrowedFinalRuleAttemptTransition::Failed(BorrowedRuleAttemptFailedRun::new(
            error,
            attempts,
            program,
            parts.into_terminal(),
        ))
    }

    fn project_miss(
        self,
        program: &'program ExecutableProgram,
        parts: AttemptRunCoreParts<E, A>,
        attempts: RuleAttemptCount,
        miss: EvaluatedRuleMiss<'program>,
    ) -> Self::Transition {
        let Self { pass: _pass } = self;
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

    fn project_applied(
        self,
        program: &'program ExecutableProgram,
        parts: AttemptRunCoreParts<E, A>,
        attempt: RuleAttemptCount,
        applied: AppliedRule<'program>,
    ) -> Self::Transition {
        let pass = self.pass;
        match applied {
            AppliedRule::AlwaysRewritten(committed) => {
                let step = committed.step();
                let rule = committed.rule();
                let cursor = BorrowedRuleAttemptCursor::from_parts(
                    program,
                    parts,
                    pass.reset_after_rewrite(),
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
                let cursor = BorrowedRuleAttemptCursor::from_parts(
                    program,
                    parts,
                    pass.reset_after_rewrite(),
                );
                BorrowedFinalRuleAttemptTransition::OnceRewritten(
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
                BorrowedFinalRuleAttemptTransition::AlwaysReturned(
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
