use crate::bytes::RuntimeStateByteCount;
use crate::error::{OwnedRunStepError, RuleAttemptStepError, RunStepError};
use crate::inspect::RuleView;
use crate::limits::{RuleAttemptCount, StepCount};
use crate::policy::{ExecutionPolicy, ParsePolicy, RuleAttemptPolicy};
use crate::program::{ReturnOutput, ReturnOutputView};
use crate::runtime::action::{AppliedRule, PreparedRuleApplication, prepare_matched_rule};
use crate::runtime::budget::{RuleAttemptBudgetState, RuleAttemptReservation, RuntimeBudgetState};
use crate::runtime::matcher::{
    MatchedRuleApplication, RuleAttempt, attempt_rule,
};
use crate::runtime::rewrite::RewriteScratch;
use crate::runtime::state::State;

use super::attempt::RuleMiss;
use super::engine::{
    ActiveRunCore, AttemptRunCore, AttemptSession, BorrowedProgram, TerminalAttemptSession,
};
use super::witness::OwnedRuleWitness;

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

/// Ordinary-run witness policy that owns parsed rule metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OwnedRunWitness {}

/// Rule-attempt witness policy that borrows parsed rule metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BorrowedAttemptWitness {}

/// Internal non-error result of one core step attempt.
pub(super) enum CoreStep<'program, RuleWitness> {
    /// A rule committed and may have terminal side effects.
    Applied(CoreAppliedRule<'program, RuleWitness>),
    /// No rule matched the current runtime state.
    Stable(StepCount),
}

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
        /// Rewrite steps committed before stability.
        steps: StepCount,
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
struct WitnessedApplication<'program, 'once, 'budget, E: ExecutionPolicy, RuleWitness> {
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

impl<'program> RunRuleWitness<'program> for OwnedRunWitness {
    type Witness = OwnedRuleWitness;
    type Error = OwnedRunStepError;

    fn from_rule(rule: RuleView<'program>) -> Result<Self::Witness, Self::Error> {
        OwnedRuleWitness::from_rule_view(rule).map_err(OwnedRunStepError::RuleWitnessAllocation)
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
    fn commit(
        self,
        state: &mut State,
        scratch: &mut RewriteScratch,
    ) -> CoreAppliedRule<'program, RuleWitness> {
        let applied = self.prepared.commit(state, scratch);
        CoreAppliedRule::from_applied_rule(applied, self.witness)
    }
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
                steps,
                final_miss,
                core,
            } => CoreRuleAttemptStep::Stable {
                attempts,
                final_miss,
                terminal: TerminalAttemptSession {
                    program,
                    core: core.into_terminal_at(steps),
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
    mut core: AttemptRunCore<'program, E>,
    mut attempt_budget: RuleAttemptBudgetState<A>,
) -> RuleAttemptAdvance<'program, E, A, W::Witness, W::Error>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
    W: AttemptRuleWitness<'program>,
{
    let target = core.runtime_rules.attempt_target();
    let (_after_miss, runtime_rule) = target.into_parts();

    let reservation = match attempt_budget.reserve_next_attempt(core.state.byte_count()) {
        Ok(reservation) => reservation,
        Err(error) => {
            return failed_rule_attempt(
                core,
                attempt_budget,
                <W::Error as From<RuleAttemptStepError>>::from(error),
            );
        }
    };
    let attempted = attempt_rule(runtime_rule, &core.state);

    match attempted {
        RuleAttempt::Missed(missed) => {
            let witness = match W::from_rule(RuleView::new(missed.rule())) {
                Ok(witness) => witness,
                Err(error) => return failed_rule_attempt(core, attempt_budget, error),
            };
            let miss = RuleMiss::new(witness, missed.reason());
            let attempt = reservation.commit();
            committed_rule_miss(core, attempt_budget, attempt, miss)
        }
        RuleAttempt::Matched(matched) => {
            let state_len = core.state.byte_count();
            let (attempt, witnessed) = match prepare_attempt_application::<_, _, W>(
                &mut core.scratch,
                &mut core.budget,
                state_len,
                reservation,
                matched,
            ) {
                Ok(committed) => committed,
                Err(error) => return failed_rule_attempt(core, attempt_budget, error),
            };
            let applied = witnessed.commit(&mut core.state, &mut core.scratch);
            committed_rule_attempt_application(core, attempt_budget, attempt, applied)
        }
    }
}

/// Reports a rule-attempt failure with the uncommitted runtime state.
fn failed_rule_attempt<'program, E, A, RuleWitness, StepError>(
    core: AttemptRunCore<'program, E>,
    attempt_budget: RuleAttemptBudgetState<A>,
    error: StepError,
) -> RuleAttemptAdvance<'program, E, A, RuleWitness, StepError>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    RuleAttemptAdvance::Failed {
        error,
        core,
        attempt_budget,
    }
}

/// Commits a non-applying rule attempt and returns the next typed state.
fn committed_rule_miss<'program, E, A, RuleWitness, StepError>(
    mut core: AttemptRunCore<'program, E>,
    attempt_budget: RuleAttemptBudgetState<A>,
    attempt: RuleAttemptCount,
    miss: RuleMiss<RuleWitness>,
) -> RuleAttemptAdvance<'program, E, A, RuleWitness, StepError>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    core.runtime_rules.commit_miss();
    RuleAttemptAdvance::Missed {
        attempt,
        miss,
        core,
        attempt_budget,
    }
}

/// Projects a committed rule application into the next rule-attempt state.
fn committed_rule_attempt_application<'program, E, A, RuleWitness, StepError>(
    mut core: AttemptRunCore<'program, E>,
    attempt_budget: RuleAttemptBudgetState<A>,
    attempt: RuleAttemptCount,
    applied: CoreAppliedRule<'program, RuleWitness>,
) -> RuleAttemptAdvance<'program, E, A, RuleWitness, StepError>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    match applied {
        CoreAppliedRule::Rewrite { step, rule } => {
            core.runtime_rules.reset_after_rewrite();
            RuleAttemptAdvance::Applied {
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
        } => RuleAttemptAdvance::Returned {
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
