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
    advance_continuing_rule_attempt(program, parts, pass)
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
    advance_final_rule_attempt(program, parts, pass)
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
