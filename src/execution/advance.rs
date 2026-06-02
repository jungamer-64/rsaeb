use crate::bytes::RuntimeStateByteCount;
use crate::error::{RuleAttemptStepError, RunStepError};
use crate::inspect::RuleView;
use crate::limits::{RuleAttemptCount, StepCount};
use crate::policy::{ExecutionPolicy, ParsePolicy, RuleAttemptPolicy};
use crate::program::{ReturnOutput, ReturnOutputView};
use crate::runtime::action::{AppliedRule, PreparedRuleStep, prepare_matched_rule};
use crate::runtime::budget::{RuleAttemptBudgetState, RuleAttemptReservation, RuntimeBudgetState};
use crate::runtime::matcher::{MatchedRuleApplication, RuleAttempt, attempt_rule};
use crate::runtime::once::{
    ContinuingRuntimeRulePass, FinalRuntimeRulePass, RuntimeRulePassCursor,
};
use crate::runtime::rewrite::RewriteScratch;
use crate::runtime::state::State;

use super::attempt::RuleMiss;
use super::engine::{
    AttemptRunCore, AttemptSession, AttemptSessionCursor, BorrowedProgram, TerminalAttemptSession,
};

/// Compile-time rule witness policy for ordinary execution steps.
pub(super) trait RunRuleWitness<'program> {
    /// Rule witness emitted by this policy.
    type Witness;
    /// Step error domain emitted by this policy.
    type Error: From<RunStepError>;

    /// Builds a witness from the matched parsed rule.
    ///
    /// # Errors
    ///
    /// Returns this policy's error when retaining the witness fails.
    fn from_rule(rule: RuleView<'program>) -> Result<Self::Witness, Self::Error>;
}

/// Compile-time rule witness policy for rule-attempt execution steps.
pub(super) trait AttemptRuleWitness<'program> {
    /// Rule witness emitted by this policy.
    type Witness;
    /// Rule-attempt error domain emitted by this policy.
    type Error: From<RuleAttemptStepError> + From<RunStepError>;

    /// Builds a witness from the selected parsed rule.
    ///
    /// # Errors
    ///
    /// Returns this policy's error when retaining the witness fails.
    fn from_rule(rule: RuleView<'program>) -> Result<Self::Witness, Self::Error>;
}

/// Ordinary-run witness policy that discards rule metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DiscardedRunWitness {}

/// Ordinary-run witness policy that borrows parsed rule metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BorrowedRunWitness {}

/// Rule-attempt witness policy that borrows parsed rule metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BorrowedAttemptWitness {}

/// Internal committed application paired with its public rule witness.
pub(super) enum CoreAppliedRule<'program, RuleWitness> {
    /// One rewrite rule committed and execution may continue.
    Rewrite {
        /// Committed step count.
        step: StepCount,
        /// Rule witness selected before runtime side effects committed.
        rule: RuleWitness,
    },
    /// One return rule committed and execution is terminal.
    Return {
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
pub(super) enum CoreContinuingRuleAttemptStep<
    'program,
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    RuleWitness,
    StepError,
> {
    /// A non-applying rule line was consumed and the run can continue.
    Missed {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Non-applying rule information.
        miss: RuleMiss<RuleWitness>,
        /// Continuation session with the returned next cursor.
        continuation: AttemptSessionCursor<'program, P, E, A>,
    },
    /// A rewrite committed and the rule-attempt run can continue.
    Applied {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Committed rewrite step count.
        step: StepCount,
        /// Rule witness paired with the committed rewrite.
        rule: RuleWitness,
        /// Continuation session with a fresh cursor.
        continuation: AttemptSessionCursor<'program, P, E, A>,
    },
    /// A return rule committed and the run is terminal.
    Returned {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Committed return step count.
        step: StepCount,
        /// Rule witness paired with the committed return.
        rule: RuleWitness,
        /// Materialized return output.
        output: ReturnOutput,
        /// Terminal session with no resumable cursor.
        terminal: TerminalAttemptSession<'program, P>,
    },
    /// A candidate attempt failed before committing runtime state.
    Failed {
        /// Error that prevented commit.
        error: StepError,
        /// Terminal session preserving the uncommitted state.
        terminal: TerminalAttemptSession<'program, P>,
    },
}

/// Program-bound result of consuming one final rule-attempt session step.
pub(super) enum CoreFinalRuleAttemptStep<
    'program,
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    RuleWitness,
    StepError,
> {
    /// A rewrite committed and the rule-attempt run can continue.
    Applied {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Committed rewrite step count.
        step: StepCount,
        /// Rule witness paired with the committed rewrite.
        rule: RuleWitness,
        /// Continuation session with a fresh cursor.
        continuation: AttemptSessionCursor<'program, P, E, A>,
    },
    /// A return rule committed and the run is terminal.
    Returned {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Committed return step count.
        step: StepCount,
        /// Rule witness paired with the committed return.
        rule: RuleWitness,
        /// Materialized return output.
        output: ReturnOutput,
        /// Terminal session with no resumable cursor.
        terminal: TerminalAttemptSession<'program, P>,
    },
    /// No rule in the current pass matched the current runtime state.
    Stable {
        /// Rule attempts consumed before stability.
        attempts: RuleAttemptCount,
        /// Final non-applying rule that exhausted the current pass.
        final_miss: RuleMiss<RuleWitness>,
        /// Terminal session with no resumable cursor.
        terminal: TerminalAttemptSession<'program, P>,
    },
    /// A candidate attempt failed before committing runtime state.
    Failed {
        /// Error that prevented commit.
        error: StepError,
        /// Terminal session preserving the uncommitted state.
        terminal: TerminalAttemptSession<'program, P>,
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
type WitnessedRuleAttempt<'program, 'once, 'budget, E, W> = (
    RuleAttemptCount,
    WitnessedApplication<'program, 'once, 'budget, E, <W as AttemptRuleWitness<'program>>::Witness>,
);

impl<'program> RunRuleWitness<'program> for DiscardedRunWitness {
    type Witness = ();
    type Error = RunStepError;

    fn from_rule(_rule: RuleView<'program>) -> Result<Self::Witness, Self::Error> {
        Ok(())
    }
}

impl<'program> RunRuleWitness<'program> for BorrowedRunWitness {
    type Witness = RuleView<'program>;
    type Error = RunStepError;

    fn from_rule(rule: RuleView<'program>) -> Result<Self::Witness, Self::Error> {
        Ok(rule)
    }
}

impl<'program> AttemptRuleWitness<'program> for BorrowedAttemptWitness {
    type Witness = RuleView<'program>;
    type Error = RuleAttemptStepError;

    fn from_rule(rule: RuleView<'program>) -> Result<Self::Witness, Self::Error> {
        Ok(rule)
    }
}

impl<'program, RuleWitness> CoreAppliedRule<'program, RuleWitness> {
    /// Combines a committed runtime application with its pre-commit rule witness.
    fn from_applied_rule(applied: AppliedRule<'program>, rule: RuleWitness) -> Self {
        match applied {
            AppliedRule::Rewrite(committed) => Self::Rewrite {
                step: committed.step(),
                rule,
            },
            AppliedRule::Return(committed) => Self::Return {
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
    /// # Errors
    ///
    /// Returns `Error` if witness creation cannot retain the selected rule.
    fn new<Error>(
        prepared: PreparedRuleStep<'program, 'once, 'budget, E>,
        make_witness: impl FnOnce(RuleView<'program>) -> Result<RuleWitness, Error>,
    ) -> Result<Self, Error> {
        let witness = make_witness(RuleView::new(prepared.rule()))?;
        Ok(Self { prepared, witness })
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
/// Returns the selected witness policy's error if runtime preparation or witness
/// construction fails.
pub(super) fn prepare_witnessed_run_application<'program, 'once, 'budget, E, W>(
    scratch: &mut RewriteScratch,
    budget: &'budget mut RuntimeBudgetState<E>,
    state_len: RuntimeStateByteCount,
    matched: MatchedRuleApplication<'program, '_, 'once>,
) -> Result<WitnessedApplication<'program, 'once, 'budget, E, W::Witness>, W::Error>
where
    E: ExecutionPolicy,
    W: RunRuleWitness<'program>,
{
    let prepared =
        prepare_matched_rule(scratch, budget, state_len, matched).map_err(W::Error::from)?;
    WitnessedApplication::new(prepared, W::from_rule)
}

/// Advances a borrowed rule-attempt session whose current rule has successors.
pub(super) fn advance_continuing_borrowed_rule_attempt<'program, P, E, A>(
    session: AttemptSession<'program, P, E, A, ContinuingRuntimeRulePass<'program>>,
) -> CoreContinuingRuleAttemptStep<'program, P, E, A, RuleView<'program>, RuleAttemptStepError>
where
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    advance_continuing_rule_attempt::<_, _, _, BorrowedAttemptWitness>(session)
}

/// Advances a borrowed rule-attempt session whose current rule exhausts the pass.
pub(super) fn advance_final_borrowed_rule_attempt<'program, P, E, A>(
    session: AttemptSession<'program, P, E, A, FinalRuntimeRulePass<'program>>,
) -> CoreFinalRuleAttemptStep<'program, P, E, A, RuleView<'program>, RuleAttemptStepError>
where
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    advance_final_rule_attempt::<_, _, _, BorrowedAttemptWitness>(session)
}

/// Advances a rule-attempt step whose selected rule is not final in the pass.
fn advance_continuing_rule_attempt<'program, P, E, A, W>(
    session: AttemptSession<'program, P, E, A, ContinuingRuntimeRulePass<'program>>,
) -> CoreContinuingRuleAttemptStep<'program, P, E, A, W::Witness, W::Error>
where
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    W: AttemptRuleWitness<'program>,
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
        program: _,
    } = core;

    let reservation = match attempt_budget.reserve_next_attempt(state.byte_count()) {
        Ok(reservation) => reservation,
        Err(error) => {
            let core = AttemptRunCore::from_parts(state, scratch, budget, pass);
            return failed_continuing_rule_attempt(
                program,
                core,
                attempt_budget,
                <W::Error as From<RuleAttemptStepError>>::from(error),
            );
        }
    };

    match attempt_rule(pass.current_rule(), &state) {
        RuleAttempt::Missed(missed) => {
            let witness = match W::from_rule(RuleView::new(missed.rule())) {
                Ok(witness) => witness,
                Err(error) => {
                    let core = AttemptRunCore::from_parts(state, scratch, budget, pass);
                    return failed_continuing_rule_attempt(program, core, attempt_budget, error);
                }
            };
            let miss = RuleMiss::new(witness, missed.reason());
            let attempt = reservation.commit();
            let runtime_rules = pass.commit_miss();
            let continuation = session_start_from_pass(
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
            let (attempt, witnessed) = match prepare_attempt_application::<_, _, W>(
                &mut scratch,
                &mut budget,
                state_len,
                reservation,
                matched,
            ) {
                Ok(committed) => committed,
                Err(error) => {
                    let core = AttemptRunCore::from_parts(state, scratch, budget, pass);
                    return failed_continuing_rule_attempt(program, core, attempt_budget, error);
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
fn advance_final_rule_attempt<'program, P, E, A, W>(
    session: AttemptSession<'program, P, E, A, FinalRuntimeRulePass<'program>>,
) -> CoreFinalRuleAttemptStep<'program, P, E, A, W::Witness, W::Error>
where
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    W: AttemptRuleWitness<'program>,
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
        program: _,
    } = core;

    let reservation = match attempt_budget.reserve_next_attempt(state.byte_count()) {
        Ok(reservation) => reservation,
        Err(error) => {
            let core = AttemptRunCore::from_parts(state, scratch, budget, pass);
            return failed_final_rule_attempt(
                program,
                core,
                attempt_budget,
                <W::Error as From<RuleAttemptStepError>>::from(error),
            );
        }
    };

    match attempt_rule(pass.current_rule(), &state) {
        RuleAttempt::Missed(missed) => {
            let witness = match W::from_rule(RuleView::new(missed.rule())) {
                Ok(witness) => witness,
                Err(error) => {
                    let core = AttemptRunCore::from_parts(state, scratch, budget, pass);
                    return failed_final_rule_attempt(program, core, attempt_budget, error);
                }
            };
            let miss = RuleMiss::new(witness, missed.reason());
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
            let (attempt, witnessed) = match prepare_attempt_application::<_, _, W>(
                &mut scratch,
                &mut budget,
                state_len,
                reservation,
                matched,
            ) {
                Ok(committed) => committed,
                Err(error) => {
                    let core = AttemptRunCore::from_parts(state, scratch, budget, pass);
                    return failed_final_rule_attempt(program, core, attempt_budget, error);
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
fn failed_continuing_rule_attempt<'program, P, E, A, Pass, RuleWitness, StepError>(
    program: BorrowedProgram<'program, P>,
    core: AttemptRunCore<'program, E, Pass>,
    attempt_budget: RuleAttemptBudgetState<A>,
    error: StepError,
) -> CoreContinuingRuleAttemptStep<'program, P, E, A, RuleWitness, StepError>
where
    P: ParsePolicy,
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
fn failed_final_rule_attempt<'program, P, E, A, Pass, RuleWitness, StepError>(
    program: BorrowedProgram<'program, P>,
    core: AttemptRunCore<'program, E, Pass>,
    attempt_budget: RuleAttemptBudgetState<A>,
    error: StepError,
) -> CoreFinalRuleAttemptStep<'program, P, E, A, RuleWitness, StepError>
where
    P: ParsePolicy,
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
fn committed_continuing_rule_attempt_application<'program, P, E, A, RuleWitness, StepError>(
    program: BorrowedProgram<'program, P>,
    core: AttemptRunCore<'program, E, ContinuingRuntimeRulePass<'program>>,
    attempt_budget: RuleAttemptBudgetState<A>,
    attempt: RuleAttemptCount,
    applied: CoreAppliedRule<'program, RuleWitness>,
) -> CoreContinuingRuleAttemptStep<'program, P, E, A, RuleWitness, StepError>
where
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    match applied {
        CoreAppliedRule::Rewrite { step, rule } => {
            let AttemptRunCore {
                state,
                scratch,
                budget,
                runtime_rules,
                program: _,
            } = core;
            let continuation = session_start_from_pass(
                program,
                state,
                scratch,
                budget,
                runtime_rules.reset_after_rewrite(),
                attempt_budget,
            );
            CoreContinuingRuleAttemptStep::Applied {
                attempt,
                step,
                rule,
                continuation,
            }
        }
        CoreAppliedRule::Return {
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
fn committed_final_rule_attempt_application<'program, P, E, A, RuleWitness, StepError>(
    program: BorrowedProgram<'program, P>,
    core: AttemptRunCore<'program, E, FinalRuntimeRulePass<'program>>,
    attempt_budget: RuleAttemptBudgetState<A>,
    attempt: RuleAttemptCount,
    applied: CoreAppliedRule<'program, RuleWitness>,
) -> CoreFinalRuleAttemptStep<'program, P, E, A, RuleWitness, StepError>
where
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    match applied {
        CoreAppliedRule::Rewrite { step, rule } => {
            let AttemptRunCore {
                state,
                scratch,
                budget,
                runtime_rules,
                program: _,
            } = core;
            let continuation = session_start_from_pass(
                program,
                state,
                scratch,
                budget,
                runtime_rules.reset_after_rewrite(),
                attempt_budget,
            );
            CoreFinalRuleAttemptStep::Applied {
                attempt,
                step,
                rule,
                continuation,
            }
        }
        CoreAppliedRule::Return {
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

/// Rebuilds a typed continuation session from a pass transition.
fn session_start_from_pass<'program, P, E, A>(
    program: BorrowedProgram<'program, P>,
    state: State,
    scratch: RewriteScratch,
    budget: RuntimeBudgetState<E>,
    runtime_rules: RuntimeRulePassCursor<'program>,
    attempt_budget: RuleAttemptBudgetState<A>,
) -> AttemptSessionCursor<'program, P, E, A>
where
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    match runtime_rules {
        RuntimeRulePassCursor::Continuing(pass) => {
            AttemptSessionCursor::Continuing(AttemptSession {
                program,
                core: AttemptRunCore::from_parts(state, scratch, budget, pass),
                attempt_budget,
            })
        }
        RuntimeRulePassCursor::Final(pass) => AttemptSessionCursor::Final(AttemptSession {
            program,
            core: AttemptRunCore::from_parts(state, scratch, budget, pass),
            attempt_budget,
        }),
    }
}

/// Prepares a matched rule-attempt application and commits its consumed-attempt count.
///
/// # Errors
///
/// Returns the selected witness policy's error if step preparation or
/// rule-witness materialization fails.
fn prepare_attempt_application<'program, 'once, 'budget, E, A, W>(
    scratch: &mut RewriteScratch,
    budget: &'budget mut RuntimeBudgetState<E>,
    state_len: RuntimeStateByteCount,
    attempt_reservation: RuleAttemptReservation<'_, A>,
    matched: MatchedRuleApplication<'program, '_, 'once>,
) -> Result<WitnessedRuleAttempt<'program, 'once, 'budget, E, W>, W::Error>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    W: AttemptRuleWitness<'program>,
{
    let prepared = prepare_matched_rule(scratch, budget, state_len, matched)?;
    let witnessed = WitnessedApplication::new(prepared, W::from_rule)?;
    let attempt = attempt_reservation.commit();
    Ok((attempt, witnessed))
}
