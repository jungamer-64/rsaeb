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

/// Private result of advancing a continuing rule-attempt pass.
///
/// Continuing passes can miss and resume, but they cannot become stable at the
/// miss boundary because the current target has a successor.
pub(super) enum ContinuingRuleAttemptAdvance<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// A non-final rule did not apply and the pass can resume at a typed cursor.
    Miss(ContinuingRuleAttemptMiss<'program, E, A>),
    /// A rewrite committed and rule-attempt execution can resume from the first rule.
    Rewritten(RuleAttemptRewrite<'program, E, A>),
    /// A return committed and the run is terminal.
    Returned(RuleAttemptReturn<'program>),
    /// The attempted rule could not complete.
    Failed(RuleAttemptFailure<'program>),
}

/// Private result of advancing a final rule-attempt pass.
///
/// Final passes can stabilize after a miss, but they cannot return a
/// missed-rule continuation because the current target exhausts the pass.
pub(super) enum FinalRuleAttemptAdvance<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// The final rule did not apply and the pass stabilized.
    StableAfterMiss(FinalRuleAttemptStable<'program>),
    /// A rewrite committed and rule-attempt execution can resume from the first rule.
    Rewritten(RuleAttemptRewrite<'program, E, A>),
    /// A return committed and the run is terminal.
    Returned(RuleAttemptReturn<'program>),
    /// The attempted rule could not complete.
    Failed(RuleAttemptFailure<'program>),
}

/// Continuing-pass miss with the next typed pass still unprojected.
pub(super) struct ContinuingRuleAttemptMiss<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// Parsed program used by the cursor projection.
    pub(super) program: &'program ExecutableProgram,
    /// Rule-attempt count committed by this transition.
    pub(super) attempt: RuleAttemptCount,
    /// Exact miss shape produced by evaluating the current rule.
    pub(super) miss: EvaluatedRuleMiss<'program>,
    /// Runtime state split from the selected pass.
    pub(super) parts: AttemptRunCoreParts<E, A>,
    /// Typed pass after consuming the missed non-final rule.
    pub(super) runtime_rules: MissedRuntimeRulePassCursor<'program>,
}

/// Terminal final-pass miss before public stable projection.
pub(super) struct FinalRuleAttemptStable<'program> {
    /// Exact miss shape produced by evaluating the final rule.
    pub(super) miss: EvaluatedRuleMiss<'program>,
    /// Terminal state after the final rule line is consumed.
    pub(super) terminal: TerminalAttemptSession<'program>,
}

/// Rule-attempt rewrite before public transition projection.
pub(super) enum RuleAttemptRewrite<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// A reusable rewrite committed.
    Always {
        /// Parsed program used by the cursor projection.
        program: &'program ExecutableProgram,
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Step number committed by this transition.
        step: crate::limits::StepCount,
        /// Exact committed rule witness.
        rule: AlwaysRewriteRuleView<'program>,
        /// Runtime state split from the selected pass.
        parts: AttemptRunCoreParts<E, A>,
        /// Reset pass after the committed rewrite.
        runtime_rules: FirstRuntimeRulePassCursor<'program>,
    },
    /// A once-only rewrite committed.
    Once {
        /// Parsed program used by the cursor projection.
        program: &'program ExecutableProgram,
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Step number committed by this transition.
        step: crate::limits::StepCount,
        /// Exact committed rule witness.
        rule: OnceRewriteRuleView<'program>,
        /// Runtime state split from the selected pass.
        parts: AttemptRunCoreParts<E, A>,
        /// Reset pass after the committed rewrite.
        runtime_rules: FirstRuntimeRulePassCursor<'program>,
    },
}

/// Rule-attempt return before public transition projection.
pub(super) enum RuleAttemptReturn<'program> {
    /// A reusable return committed.
    Always {
        /// Parsed program used by the terminal projection.
        program: &'program ExecutableProgram,
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Step number that executed the return action.
        step: crate::limits::StepCount,
        /// Exact committed rule witness.
        rule: AlwaysReturnRuleView<'program>,
        /// Materialized return output.
        output: crate::program::ReturnOutput,
    },
    /// A once-only return committed.
    Once {
        /// Parsed program used by the terminal projection.
        program: &'program ExecutableProgram,
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Step number that executed the return action.
        step: crate::limits::StepCount,
        /// Exact committed rule witness.
        rule: OnceReturnRuleView<'program>,
        /// Materialized return output.
        output: crate::program::ReturnOutput,
    },
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
            ContinuingRuleAttemptAdvance::Rewritten(RuleAttemptRewrite::Always {
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
            ContinuingRuleAttemptAdvance::Rewritten(RuleAttemptRewrite::Once {
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
            ContinuingRuleAttemptAdvance::Returned(RuleAttemptReturn::Always {
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
            ContinuingRuleAttemptAdvance::Returned(RuleAttemptReturn::Once {
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
            FinalRuleAttemptAdvance::Rewritten(RuleAttemptRewrite::Always {
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
            FinalRuleAttemptAdvance::Rewritten(RuleAttemptRewrite::Once {
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
            FinalRuleAttemptAdvance::Returned(RuleAttemptReturn::Always {
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
            FinalRuleAttemptAdvance::Returned(RuleAttemptReturn::Once {
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
