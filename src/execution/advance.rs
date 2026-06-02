use crate::bytes::RuntimeStateByteCount;
use crate::error::{RuleAttemptStepError, RunStepError};
use crate::inspect::RuleView;
use crate::limits::{RuleAttemptCount, StepCount};
use crate::policy::{ExecutionPolicy, ParsePolicy, RuleAttemptPolicy};
use crate::program::{ReturnOutput, ReturnOutputView};
use crate::runtime::action::{AppliedRule, PreparedRuleApplication, prepare_matched_rule};
use crate::runtime::budget::{RuleAttemptBudgetState, RuleAttemptReservation, RuntimeBudgetState};
use crate::runtime::matcher::{MatchedRuleApplication, RuleAttempt, attempt_rule};
use crate::runtime::once::{ContinuingRuntimeRulePass, FinalRuntimeRulePass, RuntimeRulePass};
use crate::runtime::rewrite::RewriteScratch;
use crate::runtime::state::State;

use super::attempt::RuleMiss;
use super::engine::{AttemptRunCore, AttemptSession, BorrowedProgram, TerminalAttemptSession};

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

/// Program-bound result of consuming one rule-attempt session step.
pub(super) enum CoreRuleAttemptStep<
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
        continuation: AttemptSession<'program, P, E, A>,
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
        continuation: AttemptSession<'program, P, E, A>,
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

/// Program-independent result of one rule-attempt advance.
enum RuleAttemptAdvance<'program, E: ExecutionPolicy, A: RuleAttemptPolicy, RuleWitness, StepError>
{
    /// A non-applying rule line was consumed and the run can continue.
    Missed {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Non-applying rule information.
        miss: RuleMiss<RuleWitness>,
        /// Mutable runtime state after the miss.
        core: AttemptRunCore<'program, E>,
        /// Rule-attempt budget after the miss.
        attempt_budget: RuleAttemptBudgetState<A>,
    },
    /// A rewrite committed and the rule-attempt run can continue.
    Applied {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Committed rewrite step count.
        step: StepCount,
        /// Rule witness paired with the committed rewrite.
        rule: RuleWitness,
        /// Mutable runtime state after the rewrite.
        core: AttemptRunCore<'program, E>,
        /// Rule-attempt budget after the rewrite.
        attempt_budget: RuleAttemptBudgetState<A>,
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
        /// Mutable runtime state retained for terminal observation.
        core: AttemptRunCore<'program, E>,
        /// Rule-attempt budget after the return.
        attempt_budget: RuleAttemptBudgetState<A>,
    },
    /// No rule in the current pass matched the current runtime state.
    Stable {
        /// Rule attempts consumed before stability.
        attempts: RuleAttemptCount,
        /// Final non-applying rule that exhausted the current pass.
        final_miss: RuleMiss<RuleWitness>,
        /// Mutable runtime state retained for terminal observation.
        core: AttemptRunCore<'program, E>,
    },
    /// A candidate attempt failed before committing runtime state.
    Failed {
        /// Error that prevented commit.
        error: StepError,
        /// Mutable runtime state retained for diagnostic observation.
        core: AttemptRunCore<'program, E>,
        /// Rule-attempt budget at failure.
        attempt_budget: RuleAttemptBudgetState<A>,
    },
}

/// Result of advancing a rule-attempt pass whose current target has successors.
enum ContinuingRuleAttemptAdvance<
    'program,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    RuleWitness,
    StepError,
> {
    /// A non-applying rule line was consumed and the cursor advanced.
    Missed {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Non-applying rule information.
        miss: RuleMiss<RuleWitness>,
        /// Mutable runtime state after the miss.
        core: AttemptRunCore<'program, E>,
        /// Rule-attempt budget after the miss.
        attempt_budget: RuleAttemptBudgetState<A>,
    },
    /// A rewrite committed and the rule-attempt run can continue.
    Applied {
        /// Rule-attempt count committed by this transition.
        attempt: RuleAttemptCount,
        /// Committed rewrite step count.
        step: StepCount,
        /// Rule witness paired with the committed rewrite.
        rule: RuleWitness,
        /// Mutable runtime state after the rewrite.
        core: AttemptRunCore<'program, E>,
        /// Rule-attempt budget after the rewrite.
        attempt_budget: RuleAttemptBudgetState<A>,
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
        /// Mutable runtime state retained for terminal observation.
        core: AttemptRunCore<'program, E>,
        /// Rule-attempt budget after the return.
        attempt_budget: RuleAttemptBudgetState<A>,
    },
    /// A candidate attempt failed before committing runtime state.
    Failed {
        /// Error that prevented commit.
        error: StepError,
        /// Mutable runtime state retained for diagnostic observation.
        core: AttemptRunCore<'program, E>,
        /// Rule-attempt budget at failure.
        attempt_budget: RuleAttemptBudgetState<A>,
    },
}

/// Result of advancing a rule-attempt pass whose current target exhausts the pass.
enum FinalRuleAttemptAdvance<
    'program,
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
        /// Mutable runtime state after the rewrite.
        core: AttemptRunCore<'program, E>,
        /// Rule-attempt budget after the rewrite.
        attempt_budget: RuleAttemptBudgetState<A>,
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
        /// Mutable runtime state retained for terminal observation.
        core: AttemptRunCore<'program, E>,
        /// Rule-attempt budget after the return.
        attempt_budget: RuleAttemptBudgetState<A>,
    },
    /// The final rule in the pass missed, so the whole run is stable.
    Stable {
        /// Rule attempts consumed before stability.
        attempts: RuleAttemptCount,
        /// Final non-applying rule that exhausted the current pass.
        final_miss: RuleMiss<RuleWitness>,
        /// Mutable runtime state retained for terminal observation.
        core: AttemptRunCore<'program, E>,
    },
    /// A candidate attempt failed before committing runtime state.
    Failed {
        /// Error that prevented commit.
        error: StepError,
        /// Mutable runtime state retained for diagnostic observation.
        core: AttemptRunCore<'program, E>,
        /// Rule-attempt budget at failure.
        attempt_budget: RuleAttemptBudgetState<A>,
    },
}

/// Rule application after the public witness has been created but before runtime side effects commit.
pub(super) struct WitnessedApplication<'program, 'once, 'budget, E: ExecutionPolicy, RuleWitness> {
    /// Failure-prone runtime preparation that must still be committed linearly.
    prepared: PreparedRuleApplication<'program, 'once, 'budget, E>,
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
        prepared: PreparedRuleApplication<'program, 'once, 'budget, E>,
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

impl<'program, E, A, RuleWitness, StepError>
    RuleAttemptAdvance<'program, E, A, RuleWitness, StepError>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    /// Attaches the owning program shape to this program-independent rule-attempt result.
    fn with_program<P: ParsePolicy>(
        self,
        program: BorrowedProgram<'program, P>,
    ) -> CoreRuleAttemptStep<'program, P, E, A, RuleWitness, StepError> {
        match self {
            Self::Missed {
                attempt,
                miss,
                core,
                attempt_budget,
            } => CoreRuleAttemptStep::Missed {
                attempt,
                miss,
                continuation: AttemptSession {
                    program,
                    core,
                    attempt_budget,
                },
            },
            Self::Applied {
                attempt,
                step,
                rule,
                core,
                attempt_budget,
            } => CoreRuleAttemptStep::Applied {
                attempt,
                step,
                rule,
                continuation: AttemptSession {
                    program,
                    core,
                    attempt_budget,
                },
            },
            Self::Returned {
                attempt,
                step,
                rule,
                output,
                core,
                attempt_budget,
            } => CoreRuleAttemptStep::Returned {
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
            Self::Stable {
                attempts,
                final_miss,
                core,
            } => CoreRuleAttemptStep::Stable {
                attempts,
                final_miss,
                terminal: TerminalAttemptSession {
                    program,
                    core: core.into_terminal(),
                    attempts,
                },
            },
            Self::Failed {
                error,
                core,
                attempt_budget,
            } => CoreRuleAttemptStep::Failed {
                error,
                terminal: TerminalAttemptSession {
                    program,
                    core: core.into_terminal(),
                    attempts: attempt_budget.completed_attempts(),
                },
            },
        }
    }
}

impl<'program, E, A, RuleWitness, StepError>
    ContinuingRuleAttemptAdvance<'program, E, A, RuleWitness, StepError>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    /// Erases the pass-specific outcome into the public rule-attempt transition domain.
    fn into_advance(self) -> RuleAttemptAdvance<'program, E, A, RuleWitness, StepError> {
        match self {
            Self::Missed {
                attempt,
                miss,
                core,
                attempt_budget,
            } => RuleAttemptAdvance::Missed {
                attempt,
                miss,
                core,
                attempt_budget,
            },
            Self::Applied {
                attempt,
                step,
                rule,
                core,
                attempt_budget,
            } => RuleAttemptAdvance::Applied {
                attempt,
                step,
                rule,
                core,
                attempt_budget,
            },
            Self::Returned {
                attempt,
                step,
                rule,
                output,
                core,
                attempt_budget,
            } => RuleAttemptAdvance::Returned {
                attempt,
                step,
                rule,
                output,
                core,
                attempt_budget,
            },
            Self::Failed {
                error,
                core,
                attempt_budget,
            } => RuleAttemptAdvance::Failed {
                error,
                core,
                attempt_budget,
            },
        }
    }
}

impl<'program, E, A, RuleWitness, StepError>
    FinalRuleAttemptAdvance<'program, E, A, RuleWitness, StepError>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    /// Erases the pass-specific outcome into the public rule-attempt transition domain.
    fn into_advance(self) -> RuleAttemptAdvance<'program, E, A, RuleWitness, StepError> {
        match self {
            Self::Applied {
                attempt,
                step,
                rule,
                core,
                attempt_budget,
            } => RuleAttemptAdvance::Applied {
                attempt,
                step,
                rule,
                core,
                attempt_budget,
            },
            Self::Returned {
                attempt,
                step,
                rule,
                output,
                core,
                attempt_budget,
            } => RuleAttemptAdvance::Returned {
                attempt,
                step,
                rule,
                output,
                core,
                attempt_budget,
            },
            Self::Stable {
                attempts,
                final_miss,
                core,
            } => RuleAttemptAdvance::Stable {
                attempts,
                final_miss,
                core,
            },
            Self::Failed {
                error,
                core,
                attempt_budget,
            } => RuleAttemptAdvance::Failed {
                error,
                core,
                attempt_budget,
            },
        }
    }
}

/// Advances a borrowed rule-attempt session through the shared rule-attempt kernel.
pub(super) fn advance_borrowed_rule_attempt<'program, P, E, A>(
    session: AttemptSession<'program, P, E, A>,
) -> CoreRuleAttemptStep<'program, P, E, A, RuleView<'program>, RuleAttemptStepError>
where
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    let AttemptSession {
        program,
        core,
        attempt_budget,
    } = session;
    let advanced = advance_rule_attempt::<_, _, BorrowedAttemptWitness>(core, attempt_budget);
    advanced.with_program(program)
}

/// Advances one rule-attempt step under a compile-time witness policy.
fn advance_rule_attempt<'program, E, A, W>(
    core: AttemptRunCore<'program, E>,
    attempt_budget: RuleAttemptBudgetState<A>,
) -> RuleAttemptAdvance<'program, E, A, W::Witness, W::Error>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    W: AttemptRuleWitness<'program>,
{
    let AttemptRunCore {
        state,
        scratch,
        budget,
        runtime_rules,
    } = core;
    match runtime_rules {
        RuntimeRulePass::Continuing(pass) => {
            advance_continuing_rule_attempt::<_, _, W>(state, scratch, budget, pass, attempt_budget)
                .into_advance()
        }
        RuntimeRulePass::Final(pass) => {
            advance_final_rule_attempt::<_, _, W>(state, scratch, budget, pass, attempt_budget)
                .into_advance()
        }
    }
}

/// Advances a rule-attempt step whose selected rule is not final in the pass.
fn advance_continuing_rule_attempt<'program, E, A, W>(
    mut state: State,
    mut scratch: RewriteScratch,
    mut budget: RuntimeBudgetState<E>,
    mut pass: ContinuingRuntimeRulePass<'program>,
    mut attempt_budget: RuleAttemptBudgetState<A>,
) -> ContinuingRuleAttemptAdvance<'program, E, A, W::Witness, W::Error>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    W: AttemptRuleWitness<'program>,
{
    let reservation = match attempt_budget.reserve_next_attempt(state.byte_count()) {
        Ok(reservation) => reservation,
        Err(error) => {
            let core =
                active_attempt_core(state, scratch, budget, RuntimeRulePass::Continuing(pass));
            return failed_continuing_rule_attempt(
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
                    let core = active_attempt_core(
                        state,
                        scratch,
                        budget,
                        RuntimeRulePass::Continuing(pass),
                    );
                    return failed_continuing_rule_attempt(core, attempt_budget, error);
                }
            };
            let miss = RuleMiss::new(witness, missed.reason());
            let attempt = reservation.commit();
            let runtime_rules = pass.commit_miss();
            let core = active_attempt_core(state, scratch, budget, runtime_rules);
            committed_continuing_rule_miss(core, attempt_budget, attempt, miss)
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
                    let core = active_attempt_core(
                        state,
                        scratch,
                        budget,
                        RuntimeRulePass::Continuing(pass),
                    );
                    return failed_continuing_rule_attempt(core, attempt_budget, error);
                }
            };
            let applied = witnessed.commit(&mut state, &mut scratch);
            let core =
                active_attempt_core(state, scratch, budget, RuntimeRulePass::Continuing(pass));
            committed_continuing_rule_attempt_application(core, attempt_budget, attempt, applied)
        }
    }
}

/// Advances a rule-attempt step whose selected rule exhausts the pass.
fn advance_final_rule_attempt<'program, E, A, W>(
    mut state: State,
    mut scratch: RewriteScratch,
    mut budget: RuntimeBudgetState<E>,
    mut pass: FinalRuntimeRulePass<'program>,
    mut attempt_budget: RuleAttemptBudgetState<A>,
) -> FinalRuleAttemptAdvance<'program, E, A, W::Witness, W::Error>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    W: AttemptRuleWitness<'program>,
{
    let reservation = match attempt_budget.reserve_next_attempt(state.byte_count()) {
        Ok(reservation) => reservation,
        Err(error) => {
            let core = active_attempt_core(state, scratch, budget, RuntimeRulePass::Final(pass));
            return failed_final_rule_attempt(
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
                    let core =
                        active_attempt_core(state, scratch, budget, RuntimeRulePass::Final(pass));
                    return failed_final_rule_attempt(core, attempt_budget, error);
                }
            };
            let miss = RuleMiss::new(witness, missed.reason());
            let attempt = reservation.commit();
            let core = active_attempt_core(state, scratch, budget, RuntimeRulePass::Final(pass));
            committed_final_rule_miss(core, attempt_budget, attempt, miss)
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
                    let core =
                        active_attempt_core(state, scratch, budget, RuntimeRulePass::Final(pass));
                    return failed_final_rule_attempt(core, attempt_budget, error);
                }
            };
            let applied = witnessed.commit(&mut state, &mut scratch);
            let core = active_attempt_core(state, scratch, budget, RuntimeRulePass::Final(pass));
            committed_final_rule_attempt_application(core, attempt_budget, attempt, applied)
        }
    }
}

/// Rebuilds an active attempt core after its typed pass state has changed.
fn active_attempt_core<'program, E>(
    state: State,
    scratch: RewriteScratch,
    budget: RuntimeBudgetState<E>,
    runtime_rules: RuntimeRulePass<'program>,
) -> AttemptRunCore<'program, E>
where
    E: ExecutionPolicy,
{
    AttemptRunCore {
        state,
        scratch,
        budget,
        runtime_rules,
    }
}

/// Reports a continuing-pass failure with the uncommitted runtime state.
fn failed_continuing_rule_attempt<'program, E, A, RuleWitness, StepError>(
    core: AttemptRunCore<'program, E>,
    attempt_budget: RuleAttemptBudgetState<A>,
    error: StepError,
) -> ContinuingRuleAttemptAdvance<'program, E, A, RuleWitness, StepError>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    ContinuingRuleAttemptAdvance::Failed {
        error,
        core,
        attempt_budget,
    }
}

/// Reports a final-pass failure with the uncommitted runtime state.
fn failed_final_rule_attempt<'program, E, A, RuleWitness, StepError>(
    core: AttemptRunCore<'program, E>,
    attempt_budget: RuleAttemptBudgetState<A>,
    error: StepError,
) -> FinalRuleAttemptAdvance<'program, E, A, RuleWitness, StepError>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    FinalRuleAttemptAdvance::Failed {
        error,
        core,
        attempt_budget,
    }
}

/// Commits a non-applying continuing-pass attempt and returns the next cursor state.
fn committed_continuing_rule_miss<'program, E, A, RuleWitness, StepError>(
    core: AttemptRunCore<'program, E>,
    attempt_budget: RuleAttemptBudgetState<A>,
    attempt: RuleAttemptCount,
    miss: RuleMiss<RuleWitness>,
) -> ContinuingRuleAttemptAdvance<'program, E, A, RuleWitness, StepError>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    ContinuingRuleAttemptAdvance::Missed {
        attempt,
        miss,
        core,
        attempt_budget,
    }
}

/// Commits the final non-applying rule attempt and returns terminal stability.
fn committed_final_rule_miss<'program, E, A, RuleWitness, StepError>(
    core: AttemptRunCore<'program, E>,
    attempt_budget: RuleAttemptBudgetState<A>,
    _attempt: RuleAttemptCount,
    miss: RuleMiss<RuleWitness>,
) -> FinalRuleAttemptAdvance<'program, E, A, RuleWitness, StepError>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    let attempts = attempt_budget.completed_attempts();
    FinalRuleAttemptAdvance::Stable {
        attempts,
        final_miss: miss,
        core,
    }
}

/// Projects a continuing-pass rule application into the next rule-attempt state.
fn committed_continuing_rule_attempt_application<'program, E, A, RuleWitness, StepError>(
    core: AttemptRunCore<'program, E>,
    attempt_budget: RuleAttemptBudgetState<A>,
    attempt: RuleAttemptCount,
    applied: CoreAppliedRule<'program, RuleWitness>,
) -> ContinuingRuleAttemptAdvance<'program, E, A, RuleWitness, StepError>
where
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
            } = core;
            let core =
                active_attempt_core(state, scratch, budget, runtime_rules.reset_after_rewrite());
            ContinuingRuleAttemptAdvance::Applied {
                attempt,
                step,
                rule,
                core,
                attempt_budget,
            }
        }
        CoreAppliedRule::Return {
            step,
            rule,
            output_view: _,
            output,
        } => ContinuingRuleAttemptAdvance::Returned {
            attempt,
            step,
            rule,
            output,
            core,
            attempt_budget,
        },
    }
}

/// Projects a final-pass rule application into the next rule-attempt state.
fn committed_final_rule_attempt_application<'program, E, A, RuleWitness, StepError>(
    core: AttemptRunCore<'program, E>,
    attempt_budget: RuleAttemptBudgetState<A>,
    attempt: RuleAttemptCount,
    applied: CoreAppliedRule<'program, RuleWitness>,
) -> FinalRuleAttemptAdvance<'program, E, A, RuleWitness, StepError>
where
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
            } = core;
            let core =
                active_attempt_core(state, scratch, budget, runtime_rules.reset_after_rewrite());
            FinalRuleAttemptAdvance::Applied {
                attempt,
                step,
                rule,
                core,
                attempt_budget,
            }
        }
        CoreAppliedRule::Return {
            step,
            rule,
            output_view: _,
            output,
        } => FinalRuleAttemptAdvance::Returned {
            attempt,
            step,
            rule,
            output,
            core,
            attempt_budget,
        },
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
