use crate::bytes::RuntimeStateByteCount;
use crate::error::{RuleAttemptStepError, RunStepError};
use crate::inspect::RuleView;
use crate::limits::{RuleAttemptCount, StepCount};
use crate::policy::{ExecutionPolicy, RuleAttemptPolicy};
use crate::program::{ExecutableProgram, ReturnOutput, ReturnOutputView};
use crate::runtime::action::{AppliedRule, PreparedRuleStep, prepare_matched_rule};
use crate::runtime::budget::{RuleAttemptBudgetState, RuleAttemptReservation, RuntimeBudgetState};
use crate::runtime::matcher::{MatchedRuleApplication, RuleAttempt, RuleAttemptMiss};
use crate::runtime::once::{
    AfterMissContinuingRulePass, AfterMissFinalRulePass, AfterMissRuntimeRulePass,
    FirstContinuingRulePass, FirstFinalRulePass, RuntimeRulePassState, StartedRuntimeRulePass,
};
use crate::runtime::rewrite::RewriteScratch;
use crate::runtime::state::State;

use super::attempt::RuleMiss;
use super::engine::{
    AttemptRunCore, AttemptSession, AttemptSessionCursor, ContinuingAttemptSession,
    FinalAttemptSession, TerminalAttemptSession,
};

/// Compile-time rule witness policy for ordinary execution steps.
pub(super) trait RunRuleWitness<'program> {
    /// Rule witness emitted by this policy.
    type Witness;

    /// Builds a witness from the matched parsed rule.
    fn from_rule(rule: RuleView<'program>) -> Self::Witness;
}

/// Continuing rule-attempt pass behavior shared by first and after-miss states.
trait ContinuingRuleAttemptPass<'program>: RuntimeRulePassState<'program> + Sized {
    /// Attempts this pass's current target.
    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttempt<'program, 'state, 'once>;

    /// Commits a miss and advances to the next typed pass.
    fn commit_attempt_miss(self) -> AfterMissRuntimeRulePass<'program>;

    /// Resets this pass after a committed rewrite.
    fn reset_attempt_after_rewrite(self) -> StartedRuntimeRulePass<'program>;
}

/// Final rule-attempt pass behavior shared by first and after-miss states.
trait FinalRuleAttemptPass<'program>: RuntimeRulePassState<'program> + Sized {
    /// Attempts this pass's current target.
    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttempt<'program, 'state, 'once>;

    /// Resets this pass after a committed rewrite.
    fn reset_attempt_after_rewrite(self) -> StartedRuntimeRulePass<'program>;
}

/// Ordinary-run witness policy that discards rule metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DiscardedRunWitness {}

/// Ordinary-run witness policy that borrows parsed rule metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BorrowedRunWitness {}

/// Internal committed application paired with its public rule witness.
pub(super) enum CoreAppliedRule<'program, RuleWitness> {
    /// One rewrite rule committed and execution may continue.
    Continued {
        /// Committed step count.
        step: StepCount,
        /// Rule witness selected before runtime side effects committed.
        rule: RuleWitness,
    },
    /// One return rule committed and execution is terminal.
    Terminal {
        /// Committed step count.
        step: StepCount,
        /// Rule witness selected before runtime side effects committed.
        rule: RuleWitness,
        /// Borrowed return-output view for trace callbacks.
        output_view: ReturnOutputView<'program>,
        /// Materialized return output.
        output: ReturnOutput,
    },
}

/// Program-bound result of consuming one continuing rule-attempt session step.
pub(super) enum CoreContinuingRuleAttemptStep<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// A non-applying rule line was consumed and the run can continue.
    Missed {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Non-applying rule information.
        miss: RuleMiss<'program>,
        /// Continuation session with the returned next cursor.
        continuation: AttemptSessionCursor<'program, E, A>,
    },
    /// A rewrite committed and the rule-attempt run can continue.
    Applied {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Committed rewrite step count.
        step: StepCount,
        /// Rule witness paired with the committed rewrite.
        rule: RuleView<'program>,
        /// Continuation session with a fresh cursor.
        continuation: AttemptSessionCursor<'program, E, A>,
    },
    /// A return rule committed and the run is terminal.
    Returned {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Committed return step count.
        step: StepCount,
        /// Rule witness paired with the committed return.
        rule: RuleView<'program>,
        /// Materialized return output.
        output: ReturnOutput,
        /// Terminal session with no resumable cursor.
        terminal: TerminalAttemptSession<'program>,
    },
    /// A candidate attempt failed before committing runtime state.
    Failed {
        /// Error that prevented commit.
        error: RuleAttemptStepError,
        /// Terminal session preserving the uncommitted state.
        terminal: TerminalAttemptSession<'program>,
    },
}

/// Program-bound result of consuming one final rule-attempt session step.
pub(super) enum CoreFinalRuleAttemptStep<'program, E: ExecutionPolicy, A: RuleAttemptPolicy> {
    /// A rewrite committed and the rule-attempt run can continue.
    Applied {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Committed rewrite step count.
        step: StepCount,
        /// Rule witness paired with the committed rewrite.
        rule: RuleView<'program>,
        /// Continuation session with a fresh cursor.
        continuation: AttemptSessionCursor<'program, E, A>,
    },
    /// A return rule committed and the run is terminal.
    Returned {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Committed return step count.
        step: StepCount,
        /// Rule witness paired with the committed return.
        rule: RuleView<'program>,
        /// Materialized return output.
        output: ReturnOutput,
        /// Terminal session with no resumable cursor.
        terminal: TerminalAttemptSession<'program>,
    },
    /// No rule in the current pass matched the current runtime state.
    Stable {
        /// Rule attempts consumed before stability.
        attempts: RuleAttemptCount,
        /// Final non-applying rule that exhausted the current pass.
        final_miss: RuleMiss<'program>,
        /// Terminal session with no resumable cursor.
        terminal: TerminalAttemptSession<'program>,
    },
    /// A candidate attempt failed before committing runtime state.
    Failed {
        /// Error that prevented commit.
        error: RuleAttemptStepError,
        /// Terminal session preserving the uncommitted state.
        terminal: TerminalAttemptSession<'program>,
    },
}

/// Rule application after the public witness has been created but before runtime side effects commit.
pub(super) struct WitnessedApplication<'program, 'once, 'budget, E: ExecutionPolicy, RuleWitness> {
    /// Failure-prone runtime preparation that must still be committed linearly.
    prepared: PreparedRuleStep<'program, 'once, 'budget, E>,
    /// Public rule witness created before mutation commits.
    witness: RuleWitness,
}

/// Matched rule-attempt preparation paired with its consumed-attempt count.
type WitnessedRuleAttempt<'program, 'once, 'budget, E> = (
    RuleAttemptCount,
    WitnessedApplication<'program, 'once, 'budget, E, RuleView<'program>>,
);

impl<'program> RunRuleWitness<'program> for DiscardedRunWitness {
    type Witness = ();

    fn from_rule(_rule: RuleView<'program>) -> Self::Witness {}
}

impl<'program> RunRuleWitness<'program> for BorrowedRunWitness {
    type Witness = RuleView<'program>;

    fn from_rule(rule: RuleView<'program>) -> Self::Witness {
        rule
    }
}

impl<'program> ContinuingRuleAttemptPass<'program> for FirstContinuingRulePass<'program> {
    fn attempt_current_rule<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttempt<'program, 'state, 'once> {
        self.attempt_current(state)
    }

    fn commit_attempt_miss(self) -> AfterMissRuntimeRulePass<'program> {
        self.commit_miss()
    }

    fn reset_attempt_after_rewrite(self) -> StartedRuntimeRulePass<'program> {
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

    fn commit_attempt_miss(self) -> AfterMissRuntimeRulePass<'program> {
        self.commit_miss()
    }

    fn reset_attempt_after_rewrite(self) -> StartedRuntimeRulePass<'program> {
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

    fn reset_attempt_after_rewrite(self) -> StartedRuntimeRulePass<'program> {
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

    fn reset_attempt_after_rewrite(self) -> StartedRuntimeRulePass<'program> {
        self.reset_after_rewrite()
    }
}

impl<'program, RuleWitness> CoreAppliedRule<'program, RuleWitness> {
    /// Combines a committed runtime application with its pre-commit rule witness.
    fn from_applied_rule(applied: AppliedRule<'program>, rule: RuleWitness) -> Self {
        match applied {
            AppliedRule::Continued(committed) => Self::Continued {
                step: committed.step(),
                rule,
            },
            AppliedRule::Terminal(committed) => Self::Terminal {
                step: committed.step(),
                rule,
                output_view: committed.output_view(),
                output: committed.into_output(),
            },
        }
    }
}

impl<'program, 'once, 'budget, E: ExecutionPolicy, RuleWitness>
    WitnessedApplication<'program, 'once, 'budget, E, RuleWitness>
{
    /// Pairs a prepared application with its public rule witness before commit.
    ///
    fn new(
        prepared: PreparedRuleStep<'program, 'once, 'budget, E>,
        make_witness: impl FnOnce(RuleView<'program>) -> RuleWitness,
    ) -> Self {
        let witness = make_witness(prepared.rule());
        Self { prepared, witness }
    }

    /// Commits prepared runtime side effects and publishes the paired witness.
    pub(super) fn commit(
        self,
        state: &mut State,
        scratch: &mut RewriteScratch,
    ) -> CoreAppliedRule<'program, RuleWitness> {
        let applied = self.prepared.commit(state, scratch);
        CoreAppliedRule::from_applied_rule(applied, self.witness)
    }
}

/// Prepares one matched ordinary execution step under a compile-time witness policy.
///
/// # Errors
///
/// Returns `RunStepError` if runtime preparation fails.
pub(super) fn prepare_witnessed_run_application<'program, 'once, 'budget, E, W>(
    scratch: &mut RewriteScratch,
    budget: &'budget mut RuntimeBudgetState<E>,
    state_len: RuntimeStateByteCount,
    matched: MatchedRuleApplication<'program, '_, 'once>,
) -> Result<WitnessedApplication<'program, 'once, 'budget, E, W::Witness>, RunStepError>
where
    E: ExecutionPolicy,
    W: RunRuleWitness<'program>,
{
    let prepared = prepare_matched_rule(scratch, budget, state_len, matched)?;
    Ok(WitnessedApplication::new(prepared, W::from_rule))
}

/// Advances a borrowed rule-attempt session whose current rule has successors.
pub(super) fn advance_continuing_borrowed_rule_attempt<'program, E, A>(
    session: ContinuingAttemptSession<'program, E, A>,
) -> CoreContinuingRuleAttemptStep<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    match session {
        ContinuingAttemptSession::First(session) => advance_continuing_rule_attempt(session),
        ContinuingAttemptSession::AfterMiss(session) => advance_continuing_rule_attempt(session),
    }
}

/// Advances a borrowed rule-attempt session whose current rule exhausts the pass.
pub(super) fn advance_final_borrowed_rule_attempt<'program, E, A>(
    session: FinalAttemptSession<'program, E, A>,
) -> CoreFinalRuleAttemptStep<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    match session {
        FinalAttemptSession::First(session) => advance_final_rule_attempt(session),
        FinalAttemptSession::AfterMiss(session) => advance_final_rule_attempt(session),
    }
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
    let AttemptSession {
        program,
        core,
        mut attempt_budget,
    } = session;
    let AttemptRunCore {
        mut state,
        mut scratch,
        mut budget,
        runtime_rules: mut pass,
    } = core;

    let reservation = match attempt_budget.reserve_next_attempt(state.byte_count()) {
        Ok(reservation) => reservation,
        Err(error) => {
            let core = AttemptRunCore::from_parts(state, scratch, budget, pass);
            return failed_continuing_rule_attempt(program, core, &attempt_budget, error);
        }
    };

    match pass.attempt_current_rule(&state) {
        RuleAttempt::Missed(missed) => {
            let miss = public_rule_miss(missed);
            let attempt = reservation.commit();
            let runtime_rules = pass.commit_attempt_miss();
            let continuation = session_start_from_after_miss_pass(
                program,
                state,
                scratch,
                budget,
                runtime_rules,
                attempt_budget,
            );
            CoreContinuingRuleAttemptStep::Missed {
                attempt,
                miss,
                continuation,
            }
        }
        RuleAttempt::Matched(matched) => {
            let state_len = state.byte_count();
            let (attempt, witnessed) = match prepare_attempt_application(
                &mut scratch,
                &mut budget,
                state_len,
                reservation,
                matched,
            ) {
                Ok(committed) => committed,
                Err(error) => {
                    let core = AttemptRunCore::from_parts(state, scratch, budget, pass);
                    return failed_continuing_rule_attempt(program, core, &attempt_budget, error);
                }
            };
            let applied = witnessed.commit(&mut state, &mut scratch);
            let core = AttemptRunCore::from_parts(state, scratch, budget, pass);
            committed_continuing_rule_attempt_application(
                program,
                core,
                attempt_budget,
                attempt,
                applied,
            )
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
    let AttemptSession {
        program,
        core,
        mut attempt_budget,
    } = session;
    let AttemptRunCore {
        mut state,
        mut scratch,
        mut budget,
        runtime_rules: mut pass,
    } = core;

    let reservation = match attempt_budget.reserve_next_attempt(state.byte_count()) {
        Ok(reservation) => reservation,
        Err(error) => {
            let core = AttemptRunCore::from_parts(state, scratch, budget, pass);
            return failed_final_rule_attempt(program, core, &attempt_budget, error);
        }
    };

    match pass.attempt_current_rule(&state) {
        RuleAttempt::Missed(missed) => {
            let miss = public_rule_miss(missed);
            let attempt = reservation.commit();
            let core = AttemptRunCore::from_parts(state, scratch, budget, pass);
            let attempts = attempt;
            let terminal = TerminalAttemptSession {
                program,
                core: core.into_terminal(),
                attempts,
            };
            CoreFinalRuleAttemptStep::Stable {
                attempts,
                final_miss: miss,
                terminal,
            }
        }
        RuleAttempt::Matched(matched) => {
            let state_len = state.byte_count();
            let (attempt, witnessed) = match prepare_attempt_application(
                &mut scratch,
                &mut budget,
                state_len,
                reservation,
                matched,
            ) {
                Ok(committed) => committed,
                Err(error) => {
                    let core = AttemptRunCore::from_parts(state, scratch, budget, pass);
                    return failed_final_rule_attempt(program, core, &attempt_budget, error);
                }
            };
            let applied = witnessed.commit(&mut state, &mut scratch);
            let core = AttemptRunCore::from_parts(state, scratch, budget, pass);
            committed_final_rule_attempt_application(
                program,
                core,
                attempt_budget,
                attempt,
                applied,
            )
        }
    }
}

/// Reports a continuing-pass rule-attempt failure with the uncommitted runtime state.
fn failed_continuing_rule_attempt<'program, E, A, Pass>(
    program: &'program ExecutableProgram,
    core: AttemptRunCore<E, Pass>,
    attempt_budget: &RuleAttemptBudgetState<A>,
    error: RuleAttemptStepError,
) -> CoreContinuingRuleAttemptStep<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    CoreContinuingRuleAttemptStep::Failed {
        error,
        terminal: TerminalAttemptSession {
            program,
            core: core.into_terminal(),
            attempts: attempt_budget.completed_attempts(),
        },
    }
}

/// Reports a final-pass rule-attempt failure with the uncommitted runtime state.
fn failed_final_rule_attempt<'program, E, A, Pass>(
    program: &'program ExecutableProgram,
    core: AttemptRunCore<E, Pass>,
    attempt_budget: &RuleAttemptBudgetState<A>,
    error: RuleAttemptStepError,
) -> CoreFinalRuleAttemptStep<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    CoreFinalRuleAttemptStep::Failed {
        error,
        terminal: TerminalAttemptSession {
            program,
            core: core.into_terminal(),
            attempts: attempt_budget.completed_attempts(),
        },
    }
}

/// Projects a continuing-pass committed rule application into the next rule-attempt state.
fn committed_continuing_rule_attempt_application<'program, E, A, Pass>(
    program: &'program ExecutableProgram,
    core: AttemptRunCore<E, Pass>,
    attempt_budget: RuleAttemptBudgetState<A>,
    attempt: RuleAttemptCount,
    applied: CoreAppliedRule<'program, RuleView<'program>>,
) -> CoreContinuingRuleAttemptStep<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass: ContinuingRuleAttemptPass<'program>,
{
    match applied {
        CoreAppliedRule::Continued { step, rule } => {
            let AttemptRunCore {
                state,
                scratch,
                budget,
                runtime_rules,
            } = core;
            let continuation = session_start_from_started_pass(
                program,
                state,
                scratch,
                budget,
                runtime_rules.reset_attempt_after_rewrite(),
                attempt_budget,
            );
            CoreContinuingRuleAttemptStep::Applied {
                attempt,
                step,
                rule,
                continuation,
            }
        }
        CoreAppliedRule::Terminal {
            step,
            rule,
            output_view: _,
            output,
        } => CoreContinuingRuleAttemptStep::Returned {
            attempt,
            step,
            rule,
            output,
            terminal: TerminalAttemptSession {
                program,
                core: core.into_terminal(),
                attempts: attempt_budget.completed_attempts(),
            },
        },
    }
}

/// Projects a final-pass committed rule application into the next rule-attempt state.
fn committed_final_rule_attempt_application<'program, E, A, Pass>(
    program: &'program ExecutableProgram,
    core: AttemptRunCore<E, Pass>,
    attempt_budget: RuleAttemptBudgetState<A>,
    attempt: RuleAttemptCount,
    applied: CoreAppliedRule<'program, RuleView<'program>>,
) -> CoreFinalRuleAttemptStep<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    Pass: FinalRuleAttemptPass<'program>,
{
    match applied {
        CoreAppliedRule::Continued { step, rule } => {
            let AttemptRunCore {
                state,
                scratch,
                budget,
                runtime_rules,
            } = core;
            let continuation = session_start_from_started_pass(
                program,
                state,
                scratch,
                budget,
                runtime_rules.reset_attempt_after_rewrite(),
                attempt_budget,
            );
            CoreFinalRuleAttemptStep::Applied {
                attempt,
                step,
                rule,
                continuation,
            }
        }
        CoreAppliedRule::Terminal {
            step,
            rule,
            output_view: _,
            output,
        } => CoreFinalRuleAttemptStep::Returned {
            attempt,
            step,
            rule,
            output,
            terminal: TerminalAttemptSession {
                program,
                core: core.into_terminal(),
                attempts: attempt_budget.completed_attempts(),
            },
        },
    }
}

/// Rebuilds a typed continuation session from a reset or newly started pass.
fn session_start_from_started_pass<'program, E, A>(
    program: &'program ExecutableProgram,
    state: State,
    scratch: RewriteScratch,
    budget: RuntimeBudgetState<E>,
    runtime_rules: StartedRuntimeRulePass<'program>,
    attempt_budget: RuleAttemptBudgetState<A>,
) -> AttemptSessionCursor<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    match runtime_rules {
        StartedRuntimeRulePass::Continuing(pass) => {
            AttemptSessionCursor::Continuing(ContinuingAttemptSession::First(AttemptSession {
                program,
                core: AttemptRunCore::from_parts(state, scratch, budget, pass),
                attempt_budget,
            }))
        }
        StartedRuntimeRulePass::Final(pass) => {
            AttemptSessionCursor::Final(FinalAttemptSession::First(AttemptSession {
                program,
                core: AttemptRunCore::from_parts(state, scratch, budget, pass),
                attempt_budget,
            }))
        }
    }
}

/// Rebuilds a typed continuation session after a committed miss.
fn session_start_from_after_miss_pass<'program, E, A>(
    program: &'program ExecutableProgram,
    state: State,
    scratch: RewriteScratch,
    budget: RuntimeBudgetState<E>,
    runtime_rules: AfterMissRuntimeRulePass<'program>,
    attempt_budget: RuleAttemptBudgetState<A>,
) -> AttemptSessionCursor<'program, E, A>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    match runtime_rules {
        AfterMissRuntimeRulePass::Continuing(pass) => {
            AttemptSessionCursor::Continuing(ContinuingAttemptSession::AfterMiss(AttemptSession {
                program,
                core: AttemptRunCore::from_parts(state, scratch, budget, pass),
                attempt_budget,
            }))
        }
        AfterMissRuntimeRulePass::Final(pass) => {
            AttemptSessionCursor::Final(FinalAttemptSession::AfterMiss(AttemptSession {
                program,
                core: AttemptRunCore::from_parts(state, scratch, budget, pass),
                attempt_budget,
            }))
        }
    }
}

/// Projects runtime miss shapes into the public typed miss API.
fn public_rule_miss<'program>(miss: RuleAttemptMiss<'program>) -> RuleMiss<'program> {
    match miss {
        RuleAttemptMiss::StateMismatch(rule) => RuleMiss::state_mismatch(rule),
        RuleAttemptMiss::OnceRewriteConsumed(rule) => RuleMiss::once_rewrite_consumed(rule),
        RuleAttemptMiss::OnceReturnConsumed(rule) => RuleMiss::once_return_consumed(rule),
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
) -> Result<WitnessedRuleAttempt<'program, 'once, 'budget, E>, RuleAttemptStepError>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    let prepared = prepare_matched_rule(scratch, budget, state_len, matched)?;
    let witnessed = WitnessedApplication::new(prepared, |rule| rule);
    let attempt = attempt_reservation.commit();
    Ok((attempt, witnessed))
}
