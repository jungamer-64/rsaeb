use crate::bytes::RuntimeStateByteCount;
use crate::error::RuleAttemptStepError;
use crate::inspect::{
    AlwaysReturnRuleView, AlwaysRewriteRuleView, OnceReturnRuleView, OnceRewriteRuleView,
};
use crate::limits::{RuleAttemptCount, StepCount};
use crate::policy::{ExecutionPolicy, RuleAttemptPolicy};
use crate::program::{ExecutableProgram, ReturnOutput};
use crate::runtime::action::{AppliedRule, PreparedRuleStep, prepare_matched_rule};
use crate::runtime::budget::{RuleAttemptReservation, RuntimeBudgetState};
use crate::runtime::matcher::{EvaluatedRuleMiss, MatchedRuleApplication, RuleAttemptEvaluation};
use crate::runtime::once::{
    ContinuingRuleAttemptPass, FinalRuleAttemptPass, FirstRuntimeRulePassCursor,
    MissedRuntimeRulePassCursor,
};
use crate::runtime::rewrite::RewriteScratch;

use super::engine::{AttemptRunCore, AttemptRunCoreParts, AttemptSession, TerminalAttemptSession};

/// Private result of advancing a continuing rule-attempt pass.
///
/// Continuing passes can miss and resume, but they cannot become stable at the
/// miss boundary because the current target has a successor.
pub(super) enum ContinuingRuleAttemptAdvance<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// The attempted rule could not complete.
    Failed(RuleAttemptFailure<'program>),
}

/// Private result of advancing a final rule-attempt pass.
///
/// Final passes can stabilize after a miss, but they cannot return a
/// missed-rule continuation because the current target exhausts the pass.
pub(super) enum FinalRuleAttemptAdvance<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// The attempted rule could not complete.
    Failed(RuleAttemptFailure<'program>),
}

/// Rule-attempt failure before public transition projection.
pub(super) struct RuleAttemptFailure<'program> {
    /// Runtime error that stopped the candidate attempt before commit.
    pub(super) error: RuleAttemptStepError,
    /// Number of rule attempts consumed before the failure was reported.
    pub(super) attempts: RuleAttemptCount,
    /// Parsed program borrowed by the failed terminal state.
    pub(super) program: &'program ExecutableProgram,
    /// Uncommitted runtime core retained for diagnostic inspection.
    pub(super) core: super::engine::TerminalRunCore,
}

/// Advances a borrowed rule-attempt session whose current rule has successors.
pub(super) fn advance_continuing_borrowed_rule_attempt<'program, E, A, Pass>(
    session: AttemptSession<'program, E, A, Pass>,
) -> ContinuingRuleAttemptAdvance<'program, E, A>
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
) -> FinalRuleAttemptAdvance<'program, E, A>
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
) -> ContinuingRuleAttemptAdvance<'program, E, A>
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
            ContinuingRuleAttemptAdvance::Miss(commit_continuing_miss(
                program, parts, pass, attempt, miss,
            ))
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
) -> FinalRuleAttemptAdvance<'program, E, A>
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
            FinalRuleAttemptAdvance::StableAfterMiss(commit_final_miss(
                program, parts, pass, attempts, miss,
            ))
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
) -> ContinuingRuleAttemptAdvance<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    let attempts = core.completed_attempts();
    ContinuingRuleAttemptAdvance::Failed(RuleAttemptFailure {
        error,
        attempts,
        program,
        core: core.into_terminal(),
    })
}

/// Reports a final-pass rule-attempt failure with the uncommitted runtime state.
fn failed_final_rule_attempt<'program, E, A, Pass>(
    program: &'program ExecutableProgram,
    core: AttemptRunCore<E, A, Pass>,
    error: RuleAttemptStepError,
) -> FinalRuleAttemptAdvance<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    let attempts = core.completed_attempts();
    FinalRuleAttemptAdvance::Failed(RuleAttemptFailure {
        error,
        attempts,
        program,
        core: core.into_terminal(),
    })
}

/// Commits a non-final rule-attempt miss and returns its resumable miss witness.
fn commit_continuing_miss<'program, E, A, Pass>(
    program: &'program ExecutableProgram,
    parts: AttemptRunCoreParts<E, A>,
    pass: Pass,
    attempt: RuleAttemptCount,
    miss: EvaluatedRuleMiss<'program>,
) -> ContinuingRuleAttemptMiss<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass: ContinuingRuleAttemptPass<'program>,
{
    let runtime_rules = pass.commit_attempt_miss();
    ContinuingRuleAttemptMiss {
        program,
        attempt,
        miss,
        parts,
        runtime_rules,
    }
}

/// Commits a final rule-attempt miss and returns its terminal miss witness.
fn commit_final_miss<'program, E, A, Pass>(
    program: &'program ExecutableProgram,
    parts: AttemptRunCoreParts<E, A>,
    pass: Pass,
    attempts: RuleAttemptCount,
    miss: EvaluatedRuleMiss<'program>,
) -> FinalRuleAttemptStable<'program>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass: FinalRuleAttemptPass<'program>,
{
    let core = parts.with_pass(pass);
    FinalRuleAttemptStable {
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
) -> ContinuingRuleAttemptAdvance<'program, E, A>
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
            ContinuingRuleAttemptAdvance::AlwaysRewritten(ContinuingRuleAttemptAlwaysRewrite {
                program,
                attempt,
                step,
                rule,
                parts,
                runtime_rules: runtime_rules.reset_attempt_after_rewrite(),
            })
        }
        AppliedRule::OnceRewritten(committed) => {
            let step = committed.step();
            let rule = committed.rule();
            let (parts, runtime_rules) = core.into_parts();
            ContinuingRuleAttemptAdvance::OnceRewritten(ContinuingRuleAttemptOnceRewrite {
                program,
                attempt,
                step,
                rule,
                parts,
                runtime_rules: runtime_rules.reset_attempt_after_rewrite(),
            })
        }
        AppliedRule::AlwaysReturned(committed) => {
            let step = committed.step();
            let rule = committed.rule();
            let output = committed.into_output();
            ContinuingRuleAttemptAdvance::AlwaysReturned(ContinuingRuleAttemptAlwaysReturn {
                program,
                attempt,
                step,
                rule,
                output,
            })
        }
        AppliedRule::OnceReturned(committed) => {
            let step = committed.step();
            let rule = committed.rule();
            let output = committed.into_output();
            ContinuingRuleAttemptAdvance::OnceReturned(ContinuingRuleAttemptOnceReturn {
                program,
                attempt,
                step,
                rule,
                output,
            })
        }
    }
}

/// Projects a final-pass committed rule application into the next rule-attempt state.
fn committed_final_rule_attempt_application<'program, E, A, Pass>(
    program: &'program ExecutableProgram,
    core: AttemptRunCore<E, A, Pass>,
    attempt: RuleAttemptCount,
    applied: AppliedRule<'program>,
) -> FinalRuleAttemptAdvance<'program, E, A>
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
            FinalRuleAttemptAdvance::AlwaysRewritten(FinalRuleAttemptAlwaysRewrite {
                program,
                attempt,
                step,
                rule,
                parts,
                runtime_rules: runtime_rules.reset_attempt_after_rewrite(),
            })
        }
        AppliedRule::OnceRewritten(committed) => {
            let step = committed.step();
            let rule = committed.rule();
            let (parts, runtime_rules) = core.into_parts();
            FinalRuleAttemptAdvance::OnceRewritten(FinalRuleAttemptOnceRewrite {
                program,
                attempt,
                step,
                rule,
                parts,
                runtime_rules: runtime_rules.reset_attempt_after_rewrite(),
            })
        }
        AppliedRule::AlwaysReturned(committed) => {
            let step = committed.step();
            let rule = committed.rule();
            let output = committed.into_output();
            FinalRuleAttemptAdvance::AlwaysReturned(FinalRuleAttemptAlwaysReturn {
                program,
                attempt,
                step,
                rule,
                output,
            })
        }
        AppliedRule::OnceReturned(committed) => {
            let step = committed.step();
            let rule = committed.rule();
            let output = committed.into_output();
            FinalRuleAttemptAdvance::OnceReturned(FinalRuleAttemptOnceReturn {
                program,
                attempt,
                step,
                rule,
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
