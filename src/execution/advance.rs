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
use crate::runtime::matcher::{MatchedRuleApplication, RuleAttempt};
use crate::runtime::once::{
    AfterMissContinuingRulePass, AfterMissFinalRulePass, FirstContinuingRulePass,
    FirstFinalRulePass, FirstRuntimeRulePassCursor, MissedRuntimeRulePassCursor,
    RuntimeRulePassState,
};
use crate::runtime::rewrite::RewriteScratch;
use crate::runtime::state::State;

use super::engine::{AttemptRunCore, AttemptRunCoreParts, AttemptSession, TerminalAttemptSession};
use super::session::BorrowedRuleAttemptCursor;

/// Continuing rule-attempt pass behavior shared by first and after-miss states.
pub(super) trait ContinuingRuleAttemptPass<'program>:
    RuntimeRulePassState<'program> + Sized
{
    /// Attempts this pass's current target.
    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttempt<'program, 'state, 'once>;

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
    ) -> RuleAttempt<'program, 'state, 'once>;

    /// Resets this pass after a committed rewrite.
    fn reset_attempt_after_rewrite(self) -> FirstRuntimeRulePassCursor<'program>;
}

impl<'program> ContinuingRuleAttemptPass<'program> for FirstContinuingRulePass<'program> {
    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttempt<'program, 'state, 'once> {
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
    ) -> RuleAttempt<'program, 'state, 'once> {
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
    ) -> RuleAttempt<'program, 'state, 'once> {
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
    ) -> RuleAttempt<'program, 'state, 'once> {
        self.attempt_current(state)
    }

    fn reset_attempt_after_rewrite(self) -> FirstRuntimeRulePassCursor<'program> {
        self.reset_after_rewrite()
    }
}

/// Advances a borrowed rule-attempt session whose current rule has successors.
pub(super) fn advance_continuing_borrowed_rule_attempt<'program, E, A, Pass>(
    session: AttemptSession<'program, E, A, Pass>,
) -> CoreContinuingRuleAttemptStep<'program, E, A>
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
) -> CoreFinalRuleAttemptStep<'program, E, A>
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
) -> CoreContinuingRuleAttemptStep<'program, E, A>
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
        RuleAttempt::AlwaysRewriteStateMismatch(rule) => {
            let attempt = reservation.commit();
            let continuation = commit_continuing_miss(program, parts, pass);
            CoreContinuingRuleAttemptStep::AlwaysRewriteStateMismatch {
                attempt,
                rule,
                continuation,
            }
        }
        RuleAttempt::OnceRewriteStateMismatch(rule) => {
            let attempt = reservation.commit();
            let continuation = commit_continuing_miss(program, parts, pass);
            CoreContinuingRuleAttemptStep::OnceRewriteStateMismatch {
                attempt,
                rule,
                continuation,
            }
        }
        RuleAttempt::AlwaysReturnStateMismatch(rule) => {
            let attempt = reservation.commit();
            let continuation = commit_continuing_miss(program, parts, pass);
            CoreContinuingRuleAttemptStep::AlwaysReturnStateMismatch {
                attempt,
                rule,
                continuation,
            }
        }
        RuleAttempt::OnceReturnStateMismatch(rule) => {
            let attempt = reservation.commit();
            let continuation = commit_continuing_miss(program, parts, pass);
            CoreContinuingRuleAttemptStep::OnceReturnStateMismatch {
                attempt,
                rule,
                continuation,
            }
        }
        RuleAttempt::OnceRewriteConsumed(rule) => {
            let attempt = reservation.commit();
            let continuation = commit_continuing_miss(program, parts, pass);
            CoreContinuingRuleAttemptStep::OnceRewriteConsumed {
                attempt,
                rule,
                continuation,
            }
        }
        RuleAttempt::OnceReturnConsumed(rule) => {
            let attempt = reservation.commit();
            let continuation = commit_continuing_miss(program, parts, pass);
            CoreContinuingRuleAttemptStep::OnceReturnConsumed {
                attempt,
                rule,
                continuation,
            }
        }
        RuleAttempt::Matched(matched) => {
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
) -> CoreFinalRuleAttemptStep<'program, E, A>
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
        RuleAttempt::AlwaysRewriteStateMismatch(rule) => {
            let attempts = reservation.commit();
            let terminal = commit_final_miss(program, parts, pass, attempts);
            CoreFinalRuleAttemptStep::StableAfterAlwaysRewriteStateMismatch {
                attempts,
                rule,
                terminal,
            }
        }
        RuleAttempt::OnceRewriteStateMismatch(rule) => {
            let attempts = reservation.commit();
            let terminal = commit_final_miss(program, parts, pass, attempts);
            CoreFinalRuleAttemptStep::StableAfterOnceRewriteStateMismatch {
                attempts,
                rule,
                terminal,
            }
        }
        RuleAttempt::AlwaysReturnStateMismatch(rule) => {
            let attempts = reservation.commit();
            let terminal = commit_final_miss(program, parts, pass, attempts);
            CoreFinalRuleAttemptStep::StableAfterAlwaysReturnStateMismatch {
                attempts,
                rule,
                terminal,
            }
        }
        RuleAttempt::OnceReturnStateMismatch(rule) => {
            let attempts = reservation.commit();
            let terminal = commit_final_miss(program, parts, pass, attempts);
            CoreFinalRuleAttemptStep::StableAfterOnceReturnStateMismatch {
                attempts,
                rule,
                terminal,
            }
        }
        RuleAttempt::OnceRewriteConsumed(rule) => {
            let attempts = reservation.commit();
            let terminal = commit_final_miss(program, parts, pass, attempts);
            CoreFinalRuleAttemptStep::StableAfterOnceRewriteConsumed {
                attempts,
                rule,
                terminal,
            }
        }
        RuleAttempt::OnceReturnConsumed(rule) => {
            let attempts = reservation.commit();
            let terminal = commit_final_miss(program, parts, pass, attempts);
            CoreFinalRuleAttemptStep::StableAfterOnceReturnConsumed {
                attempts,
                rule,
                terminal,
            }
        }
        RuleAttempt::Matched(matched) => {
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
) -> CoreContinuingRuleAttemptStep<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    let attempts = core.completed_attempts();
    CoreContinuingRuleAttemptStep::Failed {
        error,
        terminal: TerminalAttemptSession {
            program,
            core: core.into_terminal(),
            attempts,
        },
    }
}

/// Reports a final-pass rule-attempt failure with the uncommitted runtime state.
fn failed_final_rule_attempt<'program, E, A, Pass>(
    program: &'program ExecutableProgram,
    core: AttemptRunCore<E, A, Pass>,
    error: RuleAttemptStepError,
) -> CoreFinalRuleAttemptStep<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    let attempts = core.completed_attempts();
    CoreFinalRuleAttemptStep::Failed {
        error,
        terminal: TerminalAttemptSession {
            program,
            core: core.into_terminal(),
            attempts,
        },
    }
}

/// Commits a non-final rule-attempt miss and returns the next typed cursor.
fn commit_continuing_miss<'program, E, A, Pass>(
    program: &'program ExecutableProgram,
    parts: AttemptRunCoreParts<E, A>,
    pass: Pass,
) -> BorrowedRuleAttemptCursor<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass: ContinuingRuleAttemptPass<'program>,
{
    let runtime_rules = pass.commit_attempt_miss();
    BorrowedRuleAttemptCursor::from_after_miss_parts(program, parts, runtime_rules)
}

/// Commits a final rule-attempt miss and returns its terminal run state.
fn commit_final_miss<'program, E, A, Pass>(
    program: &'program ExecutableProgram,
    parts: AttemptRunCoreParts<E, A>,
    pass: Pass,
    attempts: RuleAttemptCount,
) -> TerminalAttemptSession<'program>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass: FinalRuleAttemptPass<'program>,
{
    let core = parts.with_pass(pass);
    TerminalAttemptSession {
        program,
        core: core.into_terminal(),
        attempts,
    }
}

/// Projects a continuing-pass committed rule application into the next rule-attempt state.
fn committed_continuing_rule_attempt_application<'program, E, A, Pass>(
    program: &'program ExecutableProgram,
    core: AttemptRunCore<E, A, Pass>,
    attempt: RuleAttemptCount,
    applied: AppliedRule<'program>,
) -> CoreContinuingRuleAttemptStep<'program, E, A>
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
            let continuation = BorrowedRuleAttemptCursor::from_first_parts(
                program,
                parts,
                runtime_rules.reset_attempt_after_rewrite(),
            );
            CoreContinuingRuleAttemptStep::AlwaysRewritten {
                attempt,
                step,
                rule,
                continuation,
            }
        }
        AppliedRule::OnceRewritten(committed) => {
            let step = committed.step();
            let rule = committed.rule();
            let (parts, runtime_rules) = core.into_parts();
            let continuation = BorrowedRuleAttemptCursor::from_first_parts(
                program,
                parts,
                runtime_rules.reset_attempt_after_rewrite(),
            );
            CoreContinuingRuleAttemptStep::OnceRewritten {
                attempt,
                step,
                rule,
                continuation,
            }
        }
        AppliedRule::AlwaysReturned(committed) => {
            let step = committed.step();
            let rule = committed.rule();
            let output = committed.into_output();
            let attempts = core.completed_attempts();
            CoreContinuingRuleAttemptStep::AlwaysReturned {
                attempt,
                step,
                rule,
                output,
                terminal: TerminalAttemptSession {
                    program,
                    core: core.into_terminal(),
                    attempts,
                },
            }
        }
        AppliedRule::OnceReturned(committed) => {
            let step = committed.step();
            let rule = committed.rule();
            let output = committed.into_output();
            let attempts = core.completed_attempts();
            CoreContinuingRuleAttemptStep::OnceReturned {
                attempt,
                step,
                rule,
                output,
                terminal: TerminalAttemptSession {
                    program,
                    core: core.into_terminal(),
                    attempts,
                },
            }
        }
    }
}

/// Projects a final-pass committed rule application into the next rule-attempt state.
fn committed_final_rule_attempt_application<'program, E, A, Pass>(
    program: &'program ExecutableProgram,
    core: AttemptRunCore<E, A, Pass>,
    attempt: RuleAttemptCount,
    applied: AppliedRule<'program>,
) -> CoreFinalRuleAttemptStep<'program, E, A>
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
            let continuation = BorrowedRuleAttemptCursor::from_first_parts(
                program,
                parts,
                runtime_rules.reset_attempt_after_rewrite(),
            );
            CoreFinalRuleAttemptStep::AlwaysRewritten {
                attempt,
                step,
                rule,
                continuation,
            }
        }
        AppliedRule::OnceRewritten(committed) => {
            let step = committed.step();
            let rule = committed.rule();
            let (parts, runtime_rules) = core.into_parts();
            let continuation = BorrowedRuleAttemptCursor::from_first_parts(
                program,
                parts,
                runtime_rules.reset_attempt_after_rewrite(),
            );
            CoreFinalRuleAttemptStep::OnceRewritten {
                attempt,
                step,
                rule,
                continuation,
            }
        }
        AppliedRule::AlwaysReturned(committed) => {
            let step = committed.step();
            let rule = committed.rule();
            let output = committed.into_output();
            let attempts = core.completed_attempts();
            CoreFinalRuleAttemptStep::AlwaysReturned {
                attempt,
                step,
                rule,
                output,
                terminal: TerminalAttemptSession {
                    program,
                    core: core.into_terminal(),
                    attempts,
                },
            }
        }
        AppliedRule::OnceReturned(committed) => {
            let step = committed.step();
            let rule = committed.rule();
            let output = committed.into_output();
            let attempts = core.completed_attempts();
            CoreFinalRuleAttemptStep::OnceReturned {
                attempt,
                step,
                rule,
                output,
                terminal: TerminalAttemptSession {
                    program,
                    core: core.into_terminal(),
                    attempts,
                },
            }
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
