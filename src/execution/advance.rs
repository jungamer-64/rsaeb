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
    /// A reusable rewrite rule was available but did not match and the pass can resume.
    AlwaysRewriteStateMismatch(RuleAttemptAlwaysRewriteMissContinuation<'program, E, A>),
    /// A once-only rewrite rule was available but did not match and the pass can resume.
    OnceRewriteStateMismatch(RuleAttemptOnceRewriteMissContinuation<'program, E, A>),
    /// A reusable return rule was available but did not match and the pass can resume.
    AlwaysReturnStateMismatch(RuleAttemptAlwaysReturnMissContinuation<'program, E, A>),
    /// A once-only return rule was available but did not match and the pass can resume.
    OnceReturnStateMismatch(RuleAttemptOnceReturnMissContinuation<'program, E, A>),
    /// A consumed once-only rewrite rule was attempted and the pass can resume.
    OnceRewriteConsumed(RuleAttemptOnceRewriteConsumedContinuation<'program, E, A>),
    /// A reusable rewrite committed and rule-attempt execution can resume from the first rule.
    AlwaysRewritten(RuleAttemptAlwaysRewriteContinuation<'program, E, A>),
    /// A once-only rewrite committed and rule-attempt execution can resume from the first rule.
    OnceRewritten(RuleAttemptOnceRewriteContinuation<'program, E, A>),
    /// A reusable return committed and the run is terminal.
    AlwaysReturned(RuleAttemptAlwaysReturnTerminal<'program>),
    /// A once-only return committed and the run is terminal.
    OnceReturned(RuleAttemptOnceReturnTerminal<'program>),
    /// The attempted rule could not complete.
    Failed(RuleAttemptFailure<'program>),
}

/// Private result of advancing a final rule-attempt pass.
///
/// Final passes can stabilize after a miss, but they cannot return a
/// missed-rule continuation because the current target exhausts the pass.
pub(super) enum FinalRuleAttemptAdvance<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// The pass ended after a reusable rewrite rule did not match.
    StableAfterAlwaysRewriteStateMismatch(
        RuleAttemptStableAfterAlwaysRewriteStateMismatch<'program>,
    ),
    /// The pass ended after a once-only rewrite rule did not match.
    StableAfterOnceRewriteStateMismatch(RuleAttemptStableAfterOnceRewriteStateMismatch<'program>),
    /// The pass ended after a reusable return rule did not match.
    StableAfterAlwaysReturnStateMismatch(RuleAttemptStableAfterAlwaysReturnStateMismatch<'program>),
    /// The pass ended after a once-only return rule did not match.
    StableAfterOnceReturnStateMismatch(RuleAttemptStableAfterOnceReturnStateMismatch<'program>),
    /// The pass ended after a consumed once-only rewrite rule.
    StableAfterOnceRewriteConsumed(RuleAttemptStableAfterOnceRewriteConsumed<'program>),
    /// A reusable rewrite committed and rule-attempt execution can resume from the first rule.
    AlwaysRewritten(RuleAttemptAlwaysRewriteContinuation<'program, E, A>),
    /// A once-only rewrite committed and rule-attempt execution can resume from the first rule.
    OnceRewritten(RuleAttemptOnceRewriteContinuation<'program, E, A>),
    /// A reusable return committed and the run is terminal.
    AlwaysReturned(RuleAttemptAlwaysReturnTerminal<'program>),
    /// A once-only return committed and the run is terminal.
    OnceReturned(RuleAttemptOnceReturnTerminal<'program>),
    /// The attempted rule could not complete.
    Failed(RuleAttemptFailure<'program>),
}

/// Continuing-pass reusable rewrite miss after the next cursor has been selected.
pub(super) struct RuleAttemptAlwaysRewriteMissContinuation<
    'program,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
> {
    /// Parsed program used by the cursor projection.
    pub(super) program: &'program ExecutableProgram,
    /// Rule-attempt count committed by this transition.
    pub(super) attempt: RuleAttemptCount,
    /// Exact non-applying rule witness.
    pub(super) rule: AlwaysRewriteRuleView<'program>,
    /// Runtime state split from the selected pass.
    pub(super) parts: AttemptRunCoreParts<E, A>,
    /// Typed pass after consuming the missed non-final rule.
    pub(super) runtime_rules: MissedRuntimeRulePassCursor<'program>,
}

/// Continuing-pass once-only rewrite miss after the next cursor has been selected.
pub(super) struct RuleAttemptOnceRewriteMissContinuation<
    'program,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
> {
    /// Parsed program used by the cursor projection.
    pub(super) program: &'program ExecutableProgram,
    /// Rule-attempt count committed by this transition.
    pub(super) attempt: RuleAttemptCount,
    /// Exact non-applying rule witness.
    pub(super) rule: OnceRewriteRuleView<'program>,
    /// Runtime state split from the selected pass.
    pub(super) parts: AttemptRunCoreParts<E, A>,
    /// Typed pass after consuming the missed non-final rule.
    pub(super) runtime_rules: MissedRuntimeRulePassCursor<'program>,
}

/// Continuing-pass reusable return miss after the next cursor has been selected.
pub(super) struct RuleAttemptAlwaysReturnMissContinuation<
    'program,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
> {
    /// Parsed program used by the cursor projection.
    pub(super) program: &'program ExecutableProgram,
    /// Rule-attempt count committed by this transition.
    pub(super) attempt: RuleAttemptCount,
    /// Exact non-applying rule witness.
    pub(super) rule: AlwaysReturnRuleView<'program>,
    /// Runtime state split from the selected pass.
    pub(super) parts: AttemptRunCoreParts<E, A>,
    /// Typed pass after consuming the missed non-final rule.
    pub(super) runtime_rules: MissedRuntimeRulePassCursor<'program>,
}

/// Continuing-pass once-only return miss after the next cursor has been selected.
pub(super) struct RuleAttemptOnceReturnMissContinuation<
    'program,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
> {
    /// Parsed program used by the cursor projection.
    pub(super) program: &'program ExecutableProgram,
    /// Rule-attempt count committed by this transition.
    pub(super) attempt: RuleAttemptCount,
    /// Exact non-applying rule witness.
    pub(super) rule: OnceReturnRuleView<'program>,
    /// Runtime state split from the selected pass.
    pub(super) parts: AttemptRunCoreParts<E, A>,
    /// Typed pass after consuming the missed non-final rule.
    pub(super) runtime_rules: MissedRuntimeRulePassCursor<'program>,
}

/// Continuing-pass consumed once-only rewrite after the next cursor has been selected.
pub(super) struct RuleAttemptOnceRewriteConsumedContinuation<
    'program,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
> {
    /// Parsed program used by the cursor projection.
    pub(super) program: &'program ExecutableProgram,
    /// Rule-attempt count committed by this transition.
    pub(super) attempt: RuleAttemptCount,
    /// Exact consumed rule witness.
    pub(super) rule: OnceRewriteRuleView<'program>,
    /// Runtime state split from the selected pass.
    pub(super) parts: AttemptRunCoreParts<E, A>,
    /// Typed pass after consuming the missed non-final rule.
    pub(super) runtime_rules: MissedRuntimeRulePassCursor<'program>,
}

/// Terminal final-pass reusable rewrite miss.
pub(super) struct RuleAttemptStableAfterAlwaysRewriteStateMismatch<'program> {
    /// Exact final non-applying rule witness.
    pub(super) rule: AlwaysRewriteRuleView<'program>,
    /// Terminal state after the final rule line is consumed.
    pub(super) terminal: TerminalAttemptSession<'program>,
}

/// Terminal final-pass once-only rewrite miss.
pub(super) struct RuleAttemptStableAfterOnceRewriteStateMismatch<'program> {
    /// Exact final non-applying rule witness.
    pub(super) rule: OnceRewriteRuleView<'program>,
    /// Terminal state after the final rule line is consumed.
    pub(super) terminal: TerminalAttemptSession<'program>,
}

/// Terminal final-pass reusable return miss.
pub(super) struct RuleAttemptStableAfterAlwaysReturnStateMismatch<'program> {
    /// Exact final non-applying rule witness.
    pub(super) rule: AlwaysReturnRuleView<'program>,
    /// Terminal state after the final rule line is consumed.
    pub(super) terminal: TerminalAttemptSession<'program>,
}

/// Terminal final-pass once-only return miss.
pub(super) struct RuleAttemptStableAfterOnceReturnStateMismatch<'program> {
    /// Exact final non-applying rule witness.
    pub(super) rule: OnceReturnRuleView<'program>,
    /// Terminal state after the final rule line is consumed.
    pub(super) terminal: TerminalAttemptSession<'program>,
}

/// Terminal final-pass consumed once-only rewrite.
pub(super) struct RuleAttemptStableAfterOnceRewriteConsumed<'program> {
    /// Exact consumed rule witness.
    pub(super) rule: OnceRewriteRuleView<'program>,
    /// Terminal state after the final rule line is consumed.
    pub(super) terminal: TerminalAttemptSession<'program>,
}

/// Committed reusable rewrite before public transition projection.
pub(super) struct RuleAttemptAlwaysRewriteContinuation<
    'program,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
> {
    /// Parsed program used by the cursor projection.
    pub(super) program: &'program ExecutableProgram,
    /// Rule-attempt count committed by this transition.
    pub(super) attempt: RuleAttemptCount,
    /// Step number committed by this transition.
    pub(super) step: StepCount,
    /// Exact committed rule witness.
    pub(super) rule: AlwaysRewriteRuleView<'program>,
    /// Runtime state split from the selected pass.
    pub(super) parts: AttemptRunCoreParts<E, A>,
    /// Reset pass after the committed rewrite.
    pub(super) runtime_rules: FirstRuntimeRulePassCursor<'program>,
}

/// Committed once-only rewrite before public transition projection.
pub(super) struct RuleAttemptOnceRewriteContinuation<
    'program,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
> {
    /// Parsed program used by the cursor projection.
    pub(super) program: &'program ExecutableProgram,
    /// Rule-attempt count committed by this transition.
    pub(super) attempt: RuleAttemptCount,
    /// Step number committed by this transition.
    pub(super) step: StepCount,
    /// Exact committed rule witness.
    pub(super) rule: OnceRewriteRuleView<'program>,
    /// Runtime state split from the selected pass.
    pub(super) parts: AttemptRunCoreParts<E, A>,
    /// Reset pass after the committed rewrite.
    pub(super) runtime_rules: FirstRuntimeRulePassCursor<'program>,
}

/// Committed reusable return before public transition projection.
pub(super) struct RuleAttemptAlwaysReturnTerminal<'program> {
    /// Parsed program used by the terminal projection.
    pub(super) program: &'program ExecutableProgram,
    /// Rule-attempt count committed by this transition.
    pub(super) attempt: RuleAttemptCount,
    /// Step number that executed the return action.
    pub(super) step: StepCount,
    /// Exact committed rule witness.
    pub(super) rule: AlwaysReturnRuleView<'program>,
    /// Materialized return output.
    pub(super) output: ReturnOutput,
}

/// Committed once-only return before public transition projection.
pub(super) struct RuleAttemptOnceReturnTerminal<'program> {
    /// Parsed program used by the terminal projection.
    pub(super) program: &'program ExecutableProgram,
    /// Rule-attempt count committed by this transition.
    pub(super) attempt: RuleAttemptCount,
    /// Step number that executed the return action.
    pub(super) step: StepCount,
    /// Exact committed rule witness.
    pub(super) rule: OnceReturnRuleView<'program>,
    /// Materialized return output.
    pub(super) output: ReturnOutput,
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
) -> ContinuingRuleAttemptAdvance<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass: ContinuingRuleAttemptPass<'program>,
{
    let runtime_rules = pass.commit_attempt_miss();
    match miss {
        EvaluatedRuleMiss::AlwaysRewriteStateMismatch(rule) => {
            ContinuingRuleAttemptAdvance::AlwaysRewriteStateMismatch(
                RuleAttemptAlwaysRewriteMissContinuation {
                    program,
                    attempt,
                    rule,
                    parts,
                    runtime_rules,
                },
            )
        }
        EvaluatedRuleMiss::OnceRewriteStateMismatch(rule) => {
            ContinuingRuleAttemptAdvance::OnceRewriteStateMismatch(
                RuleAttemptOnceRewriteMissContinuation {
                    program,
                    attempt,
                    rule,
                    parts,
                    runtime_rules,
                },
            )
        }
        EvaluatedRuleMiss::AlwaysReturnStateMismatch(rule) => {
            ContinuingRuleAttemptAdvance::AlwaysReturnStateMismatch(
                RuleAttemptAlwaysReturnMissContinuation {
                    program,
                    attempt,
                    rule,
                    parts,
                    runtime_rules,
                },
            )
        }
        EvaluatedRuleMiss::OnceReturnStateMismatch(rule) => {
            ContinuingRuleAttemptAdvance::OnceReturnStateMismatch(
                RuleAttemptOnceReturnMissContinuation {
                    program,
                    attempt,
                    rule,
                    parts,
                    runtime_rules,
                },
            )
        }
        EvaluatedRuleMiss::OnceRewriteConsumed(rule) => {
            ContinuingRuleAttemptAdvance::OnceRewriteConsumed(
                RuleAttemptOnceRewriteConsumedContinuation {
                    program,
                    attempt,
                    rule,
                    parts,
                    runtime_rules,
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
) -> FinalRuleAttemptAdvance<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass: FinalRuleAttemptPass<'program>,
{
    let core = parts.with_pass(pass);
    let terminal = TerminalAttemptSession {
        program,
        core: core.into_terminal(),
        attempts,
    };
    match miss {
        EvaluatedRuleMiss::AlwaysRewriteStateMismatch(rule) => {
            FinalRuleAttemptAdvance::StableAfterAlwaysRewriteStateMismatch(
                RuleAttemptStableAfterAlwaysRewriteStateMismatch { rule, terminal },
            )
        }
        EvaluatedRuleMiss::OnceRewriteStateMismatch(rule) => {
            FinalRuleAttemptAdvance::StableAfterOnceRewriteStateMismatch(
                RuleAttemptStableAfterOnceRewriteStateMismatch { rule, terminal },
            )
        }
        EvaluatedRuleMiss::AlwaysReturnStateMismatch(rule) => {
            FinalRuleAttemptAdvance::StableAfterAlwaysReturnStateMismatch(
                RuleAttemptStableAfterAlwaysReturnStateMismatch { rule, terminal },
            )
        }
        EvaluatedRuleMiss::OnceReturnStateMismatch(rule) => {
            FinalRuleAttemptAdvance::StableAfterOnceReturnStateMismatch(
                RuleAttemptStableAfterOnceReturnStateMismatch { rule, terminal },
            )
        }
        EvaluatedRuleMiss::OnceRewriteConsumed(rule) => {
            FinalRuleAttemptAdvance::StableAfterOnceRewriteConsumed(
                RuleAttemptStableAfterOnceRewriteConsumed { rule, terminal },
            )
        }
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
            ContinuingRuleAttemptAdvance::AlwaysRewritten(RuleAttemptAlwaysRewriteContinuation {
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
            ContinuingRuleAttemptAdvance::OnceRewritten(RuleAttemptOnceRewriteContinuation {
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
            ContinuingRuleAttemptAdvance::AlwaysReturned(RuleAttemptAlwaysReturnTerminal {
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
            ContinuingRuleAttemptAdvance::OnceReturned(RuleAttemptOnceReturnTerminal {
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
            FinalRuleAttemptAdvance::AlwaysRewritten(RuleAttemptAlwaysRewriteContinuation {
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
            FinalRuleAttemptAdvance::OnceRewritten(RuleAttemptOnceRewriteContinuation {
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
            FinalRuleAttemptAdvance::AlwaysReturned(RuleAttemptAlwaysReturnTerminal {
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
            FinalRuleAttemptAdvance::OnceReturned(RuleAttemptOnceReturnTerminal {
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
