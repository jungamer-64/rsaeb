//! Public stepwise execution contract tests.

#[path = "support/runtime.rs"]
mod runtime_support;
mod support;

use rsaeb::error::{OwnedRunStepError, RuleAttemptStepError, RunStepError};
use rsaeb::execution::{
    BorrowedAppliedStep, BorrowedFailedRun, BorrowedReturnedRun, BorrowedRuleAttemptSession,
    BorrowedRuleAttemptStart, BorrowedRuleAttemptTransition, BorrowedRuleAttempts,
    BorrowedRunSession, BorrowedStableRun, BorrowedStepTransition, BorrowedSteps, CompleteRun,
    OwnedRuleAction, OwnedRuleAttemptSession, OwnedRuleAttemptStart, OwnedRuleAttemptTransition,
    OwnedRuleAttempts, OwnedRuleWitness, OwnedStepTransition, OwnedSteps, RuleMissReason,
};
use rsaeb::input::AdmittedRun;
use rsaeb::inspect::{RuleAnchor, RuleRepeat};
use rsaeb::limits::{ReturnByteLimit, RuleAttemptLimit, RuntimeStateByteLimit};
use rsaeb::policy::{
    DefaultParsePolicy, ExecutionPolicy, ParsePolicy, RuleAttemptPolicy, StaticRuleAttemptPolicy,
};
use rsaeb::program::{Program, RunOutcome, RunResult};
use runtime_support::{DEFAULT_BYTE_BUDGET, DefaultInputRunPolicy, TestRunPolicy};
use support::{TestFailure, TestResult, ensure_eq, ensure_matches, parse_program};

/// Returns stable output bytes when they match `expected`.
///
/// # Errors
///
/// Returns `TestFailure` if the run result is not stable or stable bytes differ.
fn expect_stable_bytes(result: &RunResult, expected: &[u8]) -> TestResult {
    match result.outcome() {
        RunOutcome::Stable(output) if output.as_slice() == expected => Ok(()),
        RunOutcome::Stable(_) => Err(TestFailure::message("stable output bytes differed")),
        RunOutcome::Return(_) => Err(TestFailure::message("expected stable outcome")),
    }
}

/// Materializes a runtime state view into comparable bytes.
///
/// # Errors
///
/// Returns `TestFailure` if runtime-state view materialization fails.
fn runtime_view_bytes(state: rsaeb::trace::RuntimeStateView<'_>) -> Result<Vec<u8>, TestFailure> {
    Ok(state.materialize()?.into_raw_bytes())
}

#[derive(Debug, PartialEq, Eq)]
enum StepSignature {
    Applied {
        step: usize,
        rule_position: usize,
        state: Vec<u8>,
    },
    Stable {
        steps: usize,
        state: Vec<u8>,
    },
    Return {
        step: usize,
        rule_position: usize,
        output: Vec<u8>,
    },
}

#[derive(Debug, PartialEq, Eq)]
enum OwnedRuleAttemptSignature {
    Missed {
        attempt: usize,
        rule_position: usize,
        reason: RuleMissReason,
    },
    Applied {
        attempt: usize,
        step: usize,
        rule_position: usize,
    },
    Stable {
        attempts: usize,
        steps: usize,
        final_miss: FinalMissSignature,
    },
    Return {
        attempt: usize,
        step: usize,
        rule_position: usize,
        output: Vec<u8>,
    },
}

#[derive(Debug, PartialEq, Eq)]
enum BorrowedRuleAttemptSignature {
    Missed {
        attempt: usize,
        rule_position: usize,
        reason: RuleMissReason,
        state: Vec<u8>,
    },
    Applied {
        attempt: usize,
        step: usize,
        rule_position: usize,
        state: Vec<u8>,
    },
    Stable {
        attempts: usize,
        steps: usize,
        final_miss: FinalMissSignature,
        state: Vec<u8>,
    },
    Return {
        attempt: usize,
        step: usize,
        rule_position: usize,
        output: Vec<u8>,
    },
}

macro_rules! borrowed_miss {
    ($attempt:expr, $rule_position:expr, $reason:expr, $state:expr) => {
        BorrowedRuleAttemptSignature::Missed {
            attempt: $attempt,
            rule_position: $rule_position,
            reason: $reason,
            state: $state.to_vec(),
        }
    };
}

macro_rules! borrowed_apply {
    ($attempt:expr, $step:expr, $rule_position:expr, $state:expr) => {
        BorrowedRuleAttemptSignature::Applied {
            attempt: $attempt,
            step: $step,
            rule_position: $rule_position,
            state: $state.to_vec(),
        }
    };
}

macro_rules! borrowed_stable {
    ($attempts:expr, $steps:expr, $final_miss:expr, $state:expr $(,)?) => {
        BorrowedRuleAttemptSignature::Stable {
            attempts: $attempts,
            steps: $steps,
            final_miss: $final_miss,
            state: $state.to_vec(),
        }
    };
}

macro_rules! expect_non_failed_transition {
    ($result:expr, $failed:path) => {
        match $result {
            $failed(failed) => Err(TestFailure::from(failed.into_error())),
            transition => Ok(transition),
        }
    };
}

macro_rules! collect_owned_rule_attempt_signatures {
    ($execution:expr) => {{
        let mut execution = $execution;
        let mut signatures = Vec::new();
        loop {
            match execution.step() {
                OwnedRuleAttemptTransition::Missed(missed) => {
                    let (attempt, miss, next_execution) = missed.into_parts();
                    signatures.push(OwnedRuleAttemptSignature::Missed {
                        attempt: attempt.get(),
                        rule_position: miss.rule().position().number().get(),
                        reason: miss.reason(),
                    });
                    execution = next_execution;
                }
                OwnedRuleAttemptTransition::Applied(applied) => {
                    let (attempt, step, rule, next_execution) = applied.into_parts();
                    signatures.push(OwnedRuleAttemptSignature::Applied {
                        attempt: attempt.get(),
                        step: step.get(),
                        rule_position: rule.position().number().get(),
                    });
                    execution = next_execution;
                }
                OwnedRuleAttemptTransition::Stable(stable) => {
                    signatures.push(OwnedRuleAttemptSignature::Stable {
                        attempts: stable.attempts().get(),
                        steps: stable.steps().get(),
                        final_miss: final_miss_signature(stable.final_miss(), |rule| {
                            rule.position().number().get()
                        }),
                    });
                    return Ok(signatures);
                }
                OwnedRuleAttemptTransition::Returned(returned) => {
                    signatures.push(OwnedRuleAttemptSignature::Return {
                        attempt: returned.attempt().get(),
                        step: returned.step().get(),
                        rule_position: returned.rule().position().number().get(),
                        output: returned.output().as_slice().to_vec(),
                    });
                    return Ok(signatures);
                }
                OwnedRuleAttemptTransition::Failed(failed) => {
                    return Err(TestFailure::from(failed.into_error()));
                }
            }
        }
    }};
}

macro_rules! collect_borrowed_rule_attempt_signatures {
    ($execution:expr) => {{
        let mut execution = $execution;
        let mut signatures = Vec::new();
        loop {
            match expect_rule_attempt_transition(execution.step())? {
                BorrowedRuleAttemptTransition::Missed(missed) => {
                    signatures.push(BorrowedRuleAttemptSignature::Missed {
                        attempt: missed.attempt().get(),
                        rule_position: missed.miss().rule().position().number().get(),
                        reason: missed.miss().reason(),
                        state: runtime_view_bytes(missed.state())?,
                    });
                    execution = missed.into_session();
                }
                BorrowedRuleAttemptTransition::Applied(applied) => {
                    signatures.push(BorrowedRuleAttemptSignature::Applied {
                        attempt: applied.attempt().get(),
                        step: applied.step().get(),
                        rule_position: applied.rule().position().number().get(),
                        state: runtime_view_bytes(applied.state())?,
                    });
                    execution = applied.into_session();
                }
                BorrowedRuleAttemptTransition::Stable(stable) => {
                    signatures.push(BorrowedRuleAttemptSignature::Stable {
                        attempts: stable.attempts().get(),
                        steps: stable.steps().get(),
                        final_miss: final_miss_signature(stable.final_miss(), |rule| {
                            rule.position().number().get()
                        }),
                        state: runtime_view_bytes(stable.state())?,
                    });
                    return Ok(signatures);
                }
                BorrowedRuleAttemptTransition::Returned(returned) => {
                    signatures.push(BorrowedRuleAttemptSignature::Return {
                        attempt: returned.attempt().get(),
                        step: returned.step().get(),
                        rule_position: returned.rule().position().number().get(),
                        output: returned.output().as_slice().to_vec(),
                    });
                    return Ok(signatures);
                }
                BorrowedRuleAttemptTransition::Failed(failed) => {
                    return Err(TestFailure::from(failed.into_error()));
                }
            }
        }
    }};
}

#[derive(Debug, PartialEq, Eq)]
struct FinalMissSignature {
    rule_position: usize,
    reason: RuleMissReason,
}

enum ExpectedRuleAction<'expected> {
    Replace(&'expected [u8]),
    Return(&'expected [u8]),
}

struct ExpectedOwnedRuleWitness<'expected> {
    position: usize,
    line_number: usize,
    lhs: &'expected [u8],
    action: ExpectedRuleAction<'expected>,
}

enum ExpectedOwnedAttemptWitness<'expected> {
    Missed {
        attempt: usize,
        rule: ExpectedOwnedRuleWitness<'expected>,
        reason: RuleMissReason,
    },
    Applied {
        attempt: usize,
        step: usize,
        rule: ExpectedOwnedRuleWitness<'expected>,
    },
}

/// Ensures an owned public rule witness retained the expected parsed-rule metadata.
///
/// # Errors
///
/// Returns `TestFailure` if the witness metadata or materialized payloads differ.
fn ensure_owned_rule_witness(
    rule: &OwnedRuleWitness,
    expected: ExpectedOwnedRuleWitness<'_>,
) -> TestResult {
    ensure_eq!(rule.position().number().get(), expected.position)?;
    ensure_eq!(rule.line_number().get(), expected.line_number)?;
    ensure_eq!(rule.repeat(), RuleRepeat::Always)?;
    ensure_eq!(rule.anchor(), RuleAnchor::Anywhere)?;
    ensure_eq!(rule.lhs().as_slice(), expected.lhs)?;
    match (rule.action(), expected.action) {
        (OwnedRuleAction::Replace(payload), ExpectedRuleAction::Replace(expected))
        | (OwnedRuleAction::Return(payload), ExpectedRuleAction::Return(expected)) => {
            ensure_eq!(payload.as_slice(), expected)
        }
        (
            OwnedRuleAction::MoveStart(_)
            | OwnedRuleAction::MoveEnd(_)
            | OwnedRuleAction::Replace(_)
            | OwnedRuleAction::Return(_),
            _,
        ) => Err(TestFailure::message("unexpected owned rule witness action")),
    }
}

fn final_miss_signature<Rule>(
    miss: &rsaeb::execution::RuleMiss<Rule>,
    rule_position: impl FnOnce(&Rule) -> usize,
) -> FinalMissSignature {
    FinalMissSignature {
        rule_position: rule_position(miss.rule()),
        reason: miss.reason(),
    }
}

/// Builds a comparable signature for an applied step.
///
/// # Errors
///
/// Returns `TestFailure` if state materialization fails.
fn applied_signature<P: ParsePolicy, E: ExecutionPolicy>(
    applied: &BorrowedAppliedStep<'_, P, E>,
) -> Result<StepSignature, TestFailure> {
    Ok(StepSignature::Applied {
        step: applied.step().get(),
        rule_position: applied.rule().position().number().get(),
        state: runtime_view_bytes(applied.state())?,
    })
}

/// Builds a comparable signature for a stable terminal state.
///
/// # Errors
///
/// Returns `TestFailure` if stable-state materialization fails.
fn stable_signature<P: ParsePolicy, E: ExecutionPolicy>(
    stable: &BorrowedStableRun<'_, P, E>,
) -> Result<StepSignature, TestFailure> {
    Ok(StepSignature::Stable {
        steps: stable.steps().get(),
        state: runtime_view_bytes(stable.state())?,
    })
}

/// Builds a comparable signature for a returned step.
fn returned_signature<P: ParsePolicy>(returned: &BorrowedReturnedRun<'_, P>) -> StepSignature {
    StepSignature::Return {
        step: returned.step().get(),
        rule_position: returned.rule().position().number().get(),
        output: returned.output().as_slice().to_vec(),
    }
}

fn default_test_run_policy() -> DefaultInputRunPolicy<10, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>
{
    DefaultInputRunPolicy::<10, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new()
}

/// Extracts an active borrowed rule-attempt session from its start state.
///
/// # Errors
///
/// Returns `TestFailure` if the parsed program was empty.
fn expect_borrowed_rule_attempt_active<P, E, A>(
    start: BorrowedRuleAttemptStart<'_, P, E, A>,
) -> Result<BorrowedRuleAttemptSession<'_, P, E, A>, TestFailure>
where
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    match start {
        BorrowedRuleAttemptStart::Active(session) => Ok(session),
        BorrowedRuleAttemptStart::Empty(_) => Err(TestFailure::message(
            "expected active borrowed rule-attempt start",
        )),
    }
}

/// Extracts an active owned rule-attempt session from its start state.
///
/// # Errors
///
/// Returns `TestFailure` if the parsed program was empty.
fn expect_owned_rule_attempt_active<P, E, A>(
    start: OwnedRuleAttemptStart<P, E, A>,
) -> Result<OwnedRuleAttemptSession<P, E, A>, TestFailure>
where
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    match start {
        OwnedRuleAttemptStart::Active(session) => Ok(session),
        OwnedRuleAttemptStart::Empty(_) => Err(TestFailure::message(
            "expected active owned rule-attempt start",
        )),
    }
}

/// Runs borrowed rule-attempt execution and collects comparable transition signatures.
///
/// # Errors
///
/// Returns `TestFailure` if the program cannot be parsed, input is rejected, or
/// rule-attempt execution fails.
fn borrowed_rule_attempt_signatures<const ATTEMPTS: usize>(
    program: &Program<DefaultParsePolicy>,
    input: &'static [u8],
) -> Result<Vec<BorrowedRuleAttemptSignature>, TestFailure> {
    let execution = program.execute::<BorrowedRuleAttempts<StaticRuleAttemptPolicy<ATTEMPTS>>, _>(
        runtime_input(input, default_test_run_policy())?,
    )?;
    finish_borrowed_rule_attempt_signatures(execution)
}

/// Runs stepwise execution and collects comparable transition signatures.
///
/// # Errors
///
/// Returns `TestFailure` if a step fails or transition materialization fails.
fn finish_step_signatures<P: ParsePolicy, E: ExecutionPolicy>(
    mut execution: BorrowedRunSession<'_, P, E>,
) -> Result<Vec<StepSignature>, TestFailure> {
    let mut signatures = Vec::new();
    loop {
        match expect_step_transition(execution.step())? {
            BorrowedStepTransition::Applied(applied) => {
                signatures.push(applied_signature(&applied)?);
                execution = applied.into_session();
            }
            BorrowedStepTransition::Stable(stable) => {
                signatures.push(stable_signature(&stable)?);
                return Ok(signatures);
            }
            BorrowedStepTransition::Returned(returned) => {
                signatures.push(returned_signature(&returned));
                return Ok(signatures);
            }
            BorrowedStepTransition::Failed(failed) => {
                return Err(TestFailure::from(failed.into_error()));
            }
        }
    }
}

/// Runs owned rule-attempt execution and collects comparable transition signatures.
///
/// # Errors
///
/// Returns `TestFailure` if a rule attempt fails.
fn finish_owned_rule_attempt_signatures<P, E, A>(
    start: OwnedRuleAttemptStart<P, E, A>,
) -> Result<Vec<OwnedRuleAttemptSignature>, TestFailure>
where
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    let execution = expect_owned_rule_attempt_active(start)?;
    collect_owned_rule_attempt_signatures!(execution)
}

/// Runs borrowed rule-attempt execution and collects comparable transition signatures.
///
/// # Errors
///
/// Returns `TestFailure` if a rule attempt fails or state materialization fails.
fn finish_borrowed_rule_attempt_signatures<P, E, A>(
    start: BorrowedRuleAttemptStart<'_, P, E, A>,
) -> Result<Vec<BorrowedRuleAttemptSignature>, TestFailure>
where
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    let execution = expect_borrowed_rule_attempt_active(start)?;
    collect_borrowed_rule_attempt_signatures!(execution)
}

/// Returns the expected successful step transition.
///
/// # Errors
///
/// Returns `TestFailure` if stepping fails.
fn expect_step_transition<'program, P: ParsePolicy, E: ExecutionPolicy>(
    result: BorrowedStepTransition<'program, P, E>,
) -> Result<BorrowedStepTransition<'program, P, E>, TestFailure> {
    expect_non_failed_transition!(result, BorrowedStepTransition::Failed)
}

/// Returns the expected failed step transition.
///
/// # Errors
///
/// Returns `TestFailure` if stepping does not fail.
fn expect_failed_transition<'program, P: ParsePolicy, E: ExecutionPolicy>(
    result: BorrowedStepTransition<'program, P, E>,
) -> Result<BorrowedFailedRun<'program, P, E>, TestFailure> {
    match result {
        BorrowedStepTransition::Failed(failed) => Ok(failed),
        BorrowedStepTransition::Applied(_)
        | BorrowedStepTransition::Stable(_)
        | BorrowedStepTransition::Returned(_) => Err(TestFailure::message("expected failed step")),
    }
}

/// Validates test bytes as runtime input.
///
/// # Errors
///
/// Returns `RuntimeInputError` if the bytes are not valid runtime input.
fn runtime_input<I: rsaeb::policy::RuntimeInputPolicy, E: ExecutionPolicy>(
    bytes: &[u8],
    limits: TestRunPolicy<I, E>,
) -> Result<AdmittedRun<E>, TestFailure> {
    runtime_support::admitted_run(bytes, limits)
}

/// Returns the expected successful rule-attempt transition.
///
/// # Errors
///
/// Returns `TestFailure` if stepping fails.
fn expect_rule_attempt_transition<'program, P, E, A>(
    result: BorrowedRuleAttemptTransition<'program, P, E, A>,
) -> Result<BorrowedRuleAttemptTransition<'program, P, E, A>, TestFailure>
where
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    expect_non_failed_transition!(result, BorrowedRuleAttemptTransition::Failed)
}

/// Returns the expected failed rule-attempt transition.
///
/// # Errors
///
/// Returns `TestFailure` if stepping does not fail.
fn expect_failed_rule_attempt<'program, P, E, A>(
    result: BorrowedRuleAttemptTransition<'program, P, E, A>,
) -> Result<rsaeb::execution::BorrowedRuleAttemptFailedRun<'program, P, E>, TestFailure>
where
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    match result {
        BorrowedRuleAttemptTransition::Failed(failed) => Ok(failed),
        BorrowedRuleAttemptTransition::Missed(_)
        | BorrowedRuleAttemptTransition::Applied(_)
        | BorrowedRuleAttemptTransition::Stable(_)
        | BorrowedRuleAttemptTransition::Returned(_) => {
            Err(TestFailure::message("expected failed rule attempt"))
        }
    }
}

/// # Errors
///
/// Returns `TestFailure` if rewrite order, anchors, once rules, or runtime-only
/// byte preservation drift from the public contract.
#[test]
fn execution_rewrite_semantics_follow_public_contract() -> TestResult {
    let limits = DefaultInputRunPolicy::<10_000, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new();

    let program = parse_program("aa=x\na=y")?;
    let result = program.execute::<CompleteRun, _>(runtime_input(b"aaaa", limits)?)?;
    expect_stable_bytes(&result, b"xx")?;

    let program = parse_program("(start)a=x")?;
    let result = program.execute::<CompleteRun, _>(runtime_input(b"aba", limits)?)?;
    expect_stable_bytes(&result, b"xba")?;

    let program = parse_program("(end)a=x")?;
    let result = program.execute::<CompleteRun, _>(runtime_input(b"aba", limits)?)?;
    expect_stable_bytes(&result, b"abx")?;

    let program = parse_program("(once)a=b\na=c")?;
    let result = program.execute::<CompleteRun, _>(runtime_input(b"aa", limits)?)?;
    expect_stable_bytes(&result, b"bc")?;

    let program = parse_program("ab=x")?;
    let result = program.execute::<CompleteRun, _>(runtime_input(b"a=b", limits)?)?;
    expect_stable_bytes(&result, b"a=b")?;

    let program = parse_program("a= b")?;
    let result = program.execute::<CompleteRun, _>(runtime_input(b"a bc", limits)?)?;
    expect_stable_bytes(&result, b"b bc")
}

/// # Errors
///
/// Returns `TestFailure` if stepwise execution diverges from full-run behavior
/// or fails to pause after each applied rule.
#[test]
fn execution_stepwise_transition_surface_is_rule_by_rule() -> TestResult {
    let limits = default_test_run_policy();
    let program = parse_program("a=b\nb=c")?;
    let input = runtime_input(b"a", limits)?;
    let execution = program.execute::<BorrowedSteps, _>(input)?;
    ensure_eq!(execution.completed_steps().get(), 0)?;

    let execution = match expect_step_transition(execution.step())? {
        BorrowedStepTransition::Applied(applied) => {
            ensure_eq!(applied.step().get(), 1)?;
            ensure_eq!(applied.rule().position().number().get(), 1)?;
            ensure_eq!(
                runtime_view_bytes(applied.state())?.as_slice(),
                b"b".as_slice()
            )?;
            ensure_eq!(applied.state().byte_count().get(), 1)?;
            applied.into_session()
        }
        BorrowedStepTransition::Stable(_)
        | BorrowedStepTransition::Returned(_)
        | BorrowedStepTransition::Failed(_) => {
            return Err(TestFailure::message("expected first applied step"));
        }
    };

    let execution = match expect_step_transition(execution.step())? {
        BorrowedStepTransition::Applied(applied) => {
            ensure_eq!(applied.step().get(), 2)?;
            ensure_eq!(applied.rule().position().number().get(), 2)?;
            ensure_eq!(
                runtime_view_bytes(applied.state())?.as_slice(),
                b"c".as_slice()
            )?;
            applied.into_session()
        }
        BorrowedStepTransition::Stable(_)
        | BorrowedStepTransition::Returned(_)
        | BorrowedStepTransition::Failed(_) => {
            return Err(TestFailure::message("expected second applied step"));
        }
    };

    match expect_step_transition(execution.step())? {
        BorrowedStepTransition::Stable(stable) => {
            ensure_eq!(stable.steps().get(), 2)?;
            ensure_eq!(
                runtime_view_bytes(stable.state())?.as_slice(),
                b"c".as_slice()
            )?;
        }
        BorrowedStepTransition::Applied(_)
        | BorrowedStepTransition::Returned(_)
        | BorrowedStepTransition::Failed(_) => {
            return Err(TestFailure::message("expected stable completion"));
        }
    }
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if rule-attempt execution does not pause on
/// non-matching executable rule lines or reset the rule cursor after matches.
#[test]
fn execution_rule_attempt_surface_reports_misses_and_resets_after_apply() -> TestResult {
    let program = parse_program("z=x\na=b\nb=c")?;
    ensure_eq!(
        borrowed_rule_attempt_signatures::<20>(&program, b"a")?,
        vec![
            borrowed_miss!(1, 1, RuleMissReason::StateMismatch, b"a"),
            borrowed_apply!(2, 1, 2, b"b"),
            borrowed_miss!(3, 1, RuleMissReason::StateMismatch, b"b"),
            borrowed_miss!(4, 2, RuleMissReason::StateMismatch, b"b"),
            borrowed_apply!(5, 2, 3, b"c"),
            borrowed_miss!(6, 1, RuleMissReason::StateMismatch, b"c"),
            borrowed_miss!(7, 2, RuleMissReason::StateMismatch, b"c"),
            borrowed_stable!(
                8,
                2,
                FinalMissSignature {
                    rule_position: 3,
                    reason: RuleMissReason::StateMismatch,
                },
                b"c",
            ),
        ],
    )
}

/// # Errors
///
/// Returns `TestFailure` if owned rule-attempt execution loses the same miss,
/// reset, or return semantics as the borrowed surface.
#[test]
fn execution_owned_rule_attempt_surface_reports_misses_resets_and_returns() -> TestResult {
    let limits = default_test_run_policy();
    let execution = parse_program("z=x\na=b\nb=(return)ok")?.into_execute::<OwnedRuleAttempts<
        StaticRuleAttemptPolicy<20>,
    >, _>(runtime_input(
        b"a", limits,
    )?)?;
    ensure_eq!(
        finish_owned_rule_attempt_signatures(execution)?,
        [
            OwnedRuleAttemptSignature::Missed {
                attempt: 1,
                rule_position: 1,
                reason: RuleMissReason::StateMismatch,
            },
            OwnedRuleAttemptSignature::Applied {
                attempt: 2,
                step: 1,
                rule_position: 2,
            },
            OwnedRuleAttemptSignature::Missed {
                attempt: 3,
                rule_position: 1,
                reason: RuleMissReason::StateMismatch,
            },
            OwnedRuleAttemptSignature::Missed {
                attempt: 4,
                rule_position: 2,
                reason: RuleMissReason::StateMismatch,
            },
            OwnedRuleAttemptSignature::Return {
                attempt: 5,
                step: 2,
                rule_position: 3,
                output: b"ok".to_vec(),
            },
        ],
    )
}

/// # Errors
///
/// Returns `TestFailure` if rule-attempt start and final-miss terminals are not
/// exposed as typed public values.
#[test]
fn execution_rule_attempt_start_and_final_miss_are_typed() -> TestResult {
    let limits = default_test_run_policy();
    let program = parse_program("a=b")?;
    let input = runtime_input(b"z", limits)?;
    let execution = expect_borrowed_rule_attempt_active(
        program.execute::<BorrowedRuleAttempts<StaticRuleAttemptPolicy<10>>, _>(input)?,
    )?;

    match expect_rule_attempt_transition(execution.step())? {
        BorrowedRuleAttemptTransition::Stable(stable) => {
            ensure_eq!(stable.attempts().get(), 1)?;
            ensure_eq!(stable.steps().get(), 0)?;
            let final_miss = stable.final_miss();
            ensure_eq!(final_miss.rule().position().number().get(), 1)?;
            ensure_eq!(final_miss.reason(), RuleMissReason::StateMismatch)?;
            ensure_eq!(
                runtime_view_bytes(stable.state())?.as_slice(),
                b"z".as_slice(),
            )?;
        }
        BorrowedRuleAttemptTransition::Missed(_)
        | BorrowedRuleAttemptTransition::Applied(_)
        | BorrowedRuleAttemptTransition::Returned(_)
        | BorrowedRuleAttemptTransition::Failed(_) => {
            return Err(TestFailure::message("expected immediate stable terminal"));
        }
    }
    let program = parse_program("# no executable rules")?;
    let start = program.execute::<BorrowedRuleAttempts<StaticRuleAttemptPolicy<10>>, _>(
        runtime_input(b"z", limits)?,
    )?;

    match start {
        BorrowedRuleAttemptStart::Empty(empty) => {
            ensure_eq!(empty.attempts().get(), 0)?;
            ensure_eq!(empty.steps().get(), 0)?;
            ensure_eq!(
                runtime_view_bytes(empty.state())?.as_slice(),
                b"z".as_slice(),
            )
        }
        BorrowedRuleAttemptStart::Active(_) => Err(TestFailure::message(
            "expected empty-program stable terminal",
        )),
    }?;

    let owned_start = parse_program("# no executable rules")?.into_execute::<OwnedRuleAttempts<
        StaticRuleAttemptPolicy<10>,
    >, _>(runtime_input(
        b"z", limits,
    )?)?;

    match owned_start {
        OwnedRuleAttemptStart::Empty(empty) => {
            ensure_eq!(empty.attempts().get(), 0)?;
            ensure_eq!(empty.steps().get(), 0)?;
            ensure_eq!(
                runtime_view_bytes(empty.state())?.as_slice(),
                b"z".as_slice(),
            )?;
            ensure_eq!(empty.into_program().rule_count().get(), 0)
        }
        OwnedRuleAttemptStart::Active(_) => Err(TestFailure::message(
            "expected owned empty-program terminal",
        )),
    }
}

/// # Errors
///
/// Returns `TestFailure` if interleaved always rules consume `(once)` state or
/// consumed `(once)` rules stop being reported as typed rule-attempt misses.
#[test]
fn execution_rule_attempt_preserves_interleaved_once_state() -> TestResult {
    let program = parse_program("(once)a=b\nz=z\n(once)b=c")?;
    ensure_eq!(program.once_rule_count().get(), 2)?;
    ensure_eq!(
        borrowed_rule_attempt_signatures::<10>(&program, b"a")?,
        vec![
            borrowed_apply!(1, 1, 1, b"b"),
            borrowed_miss!(2, 1, RuleMissReason::OnceConsumed, b"b"),
            borrowed_miss!(3, 2, RuleMissReason::StateMismatch, b"b"),
            borrowed_apply!(4, 2, 3, b"c"),
            borrowed_miss!(5, 1, RuleMissReason::OnceConsumed, b"c"),
            borrowed_miss!(6, 2, RuleMissReason::StateMismatch, b"c"),
            borrowed_stable!(
                7,
                2,
                FinalMissSignature {
                    rule_position: 3,
                    reason: RuleMissReason::OnceConsumed,
                },
                b"c",
            ),
        ],
    )
}

/// # Errors
///
/// Returns `TestFailure` if rule-attempt execution leaks `(once)` consumption
/// between separate runs of the same parsed program.
#[test]
fn execution_rule_attempt_once_state_is_run_local_for_reused_program() -> TestResult {
    let program = parse_program("(once)a=b\nb=c")?;
    let expected = vec![
        borrowed_apply!(1, 1, 1, b"b"),
        borrowed_miss!(2, 1, RuleMissReason::OnceConsumed, b"b"),
        borrowed_apply!(3, 2, 2, b"c"),
        borrowed_miss!(4, 1, RuleMissReason::OnceConsumed, b"c"),
        borrowed_stable!(
            5,
            2,
            FinalMissSignature {
                rule_position: 2,
                reason: RuleMissReason::StateMismatch,
            },
            b"c",
        ),
    ];

    ensure_eq!(
        borrowed_rule_attempt_signatures::<10>(&program, b"a")?,
        expected
    )?;
    ensure_eq!(
        borrowed_rule_attempt_signatures::<10>(&program, b"a")?,
        expected
    )
}

/// # Errors
///
/// Returns `TestFailure` if rule-attempt budget is folded into execution-step
/// budget or fails to report typed details.
#[test]
fn execution_rule_attempt_limit_is_independent_from_step_limit() -> TestResult {
    let limits = DefaultInputRunPolicy::<0, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new();
    let program = parse_program("x=y\na=b")?;
    let input = runtime_input(b"a", limits)?;
    let execution = expect_borrowed_rule_attempt_active(
        program.execute::<BorrowedRuleAttempts<StaticRuleAttemptPolicy<1>>, _>(input)?,
    )?;

    let execution = match expect_rule_attempt_transition(execution.step())? {
        BorrowedRuleAttemptTransition::Missed(missed) => {
            ensure_eq!(missed.attempt().get(), 1)?;
            ensure_eq!((*missed.miss().rule()).position().number().get(), 1)?;
            ensure_eq!(missed.miss().reason(), RuleMissReason::StateMismatch)?;
            missed.into_session()
        }
        BorrowedRuleAttemptTransition::Applied(_)
        | BorrowedRuleAttemptTransition::Stable(_)
        | BorrowedRuleAttemptTransition::Returned(_)
        | BorrowedRuleAttemptTransition::Failed(_) => {
            return Err(TestFailure::message(
                "expected miss despite zero execution-step limit",
            ));
        }
    };

    let failed = expect_failed_rule_attempt(execution.step())?;
    ensure_eq!(failed.completed_attempts().get(), 1)?;
    ensure_eq!(failed.completed_steps().get(), 0)?;
    ensure_matches(
        matches!(
            failed.into_error(),
            RuleAttemptStepError::RuleAttemptLimit(error)
                if error.max_attempts() == RuleAttemptLimit::new(1)
                    && error.completed_attempts().get() == 1
                    && error.state_len().get() == 1
        ),
        "expected rule-attempt limit details",
    )
}

/// # Errors
///
/// Returns `TestFailure` if failed rule preparation publishes the reserved
/// rule-attempt count.
#[test]
fn execution_rule_attempt_preparation_failure_drops_attempt_reservation() -> TestResult {
    let limits = DefaultInputRunPolicy::<10, 1, DEFAULT_BYTE_BUDGET>::new();
    let program = parse_program("a=aa")?;
    let input = runtime_input(b"a", limits)?;
    let execution = expect_borrowed_rule_attempt_active(
        program.execute::<BorrowedRuleAttempts<StaticRuleAttemptPolicy<10>>, _>(input)?,
    )?;

    let failed = expect_failed_rule_attempt(execution.step())?;
    ensure_eq!(failed.completed_attempts().get(), 0)?;
    ensure_eq!(failed.completed_steps().get(), 0)?;
    ensure_eq!(
        runtime_view_bytes(failed.state())?.as_slice(),
        b"a".as_slice(),
    )?;
    ensure_matches(
        matches!(
            failed.into_error(),
            RuleAttemptStepError::Step(RunStepError::RuntimeStateLimit(error))
                if error.limit() == RuntimeStateByteLimit::new(1)
                    && error.attempted_len().get() == 2
        ),
        "expected state limit before attempt reservation commits",
    )
}

/// # Errors
///
/// Returns `TestFailure` if execution state views do not expose initial and
/// current state bytes correctly.
#[test]
fn execution_state_view_exposes_initial_and_current_state() -> TestResult {
    let limits = default_test_run_policy();
    let program = parse_program("a=b")?;
    let input = runtime_input(b"a", limits)?;
    let execution = program.execute::<BorrowedSteps, _>(input)?;

    ensure_eq!(
        runtime_view_bytes(execution.state())?.as_slice(),
        b"a".as_slice(),
    )?;

    let execution = match expect_step_transition(execution.step())? {
        BorrowedStepTransition::Applied(applied) => {
            ensure_eq!(
                runtime_view_bytes(applied.state())?.as_slice(),
                b"b".as_slice()
            )?;
            applied.into_session()
        }
        BorrowedStepTransition::Stable(_)
        | BorrowedStepTransition::Returned(_)
        | BorrowedStepTransition::Failed(_) => {
            return Err(TestFailure::message("expected applied step"));
        }
    };

    ensure_eq!(
        runtime_view_bytes(execution.state())?.as_slice(),
        b"b".as_slice(),
    )
}

/// # Errors
///
/// Returns `TestFailure` if repeated stepwise executions from one parsed
/// program diverge.
#[test]
fn execution_consumes_runtime_input_without_session_leakage() -> TestResult {
    let limits = default_test_run_policy();
    let source = "(once)a=b\na=c";
    let program = parse_program(source)?;
    let first = program.execute::<BorrowedSteps, _>(runtime_input(b"aa", limits)?)?;
    let second = program.execute::<BorrowedSteps, _>(runtime_input(b"aa", limits)?)?;
    let third = program.execute::<BorrowedSteps, _>(runtime_input(b"aa", limits)?)?;

    ensure_eq!(
        finish_step_signatures(first)?,
        [
            StepSignature::Applied {
                step: 1,
                rule_position: 1,
                state: b"ba".to_vec(),
            },
            StepSignature::Applied {
                step: 2,
                rule_position: 2,
                state: b"bc".to_vec(),
            },
            StepSignature::Stable {
                steps: 2,
                state: b"bc".to_vec(),
            },
        ],
    )?;
    ensure_eq!(
        finish_step_signatures(second)?,
        finish_step_signatures(third)?,
    )
}

/// # Errors
///
/// Returns `TestFailure` if borrowed and owned run sessions diverge for the
/// same source, input, and limits.
#[test]
fn execution_borrowed_run_and_owned_session_share_contract() -> TestResult {
    let source = "a=b\nb=(return)ok";
    let limits = default_test_run_policy();

    let borrowed =
        parse_program(source)?.execute::<CompleteRun, _>(runtime_input(b"a", limits)?)?;
    let owned = parse_program(source)?
        .into_execute::<OwnedSteps, _>(runtime_input(b"a", limits)?)?
        .finish()?;

    ensure_eq!(borrowed, owned)
}

/// # Errors
///
/// Returns `TestFailure` if owned stepwise terminal states cannot return the
/// parsed program to the caller.
#[test]
fn execution_owned_terminals_can_return_program() -> TestResult {
    let limits = default_test_run_policy();

    let stable_session =
        parse_program("a=b")?.into_execute::<OwnedSteps, _>(runtime_input(b"a", limits)?)?;
    let stable_session = match stable_session.step() {
        OwnedStepTransition::Applied(applied) => applied.into_session(),
        OwnedStepTransition::Stable(_)
        | OwnedStepTransition::Returned(_)
        | OwnedStepTransition::Failed(_) => {
            return Err(TestFailure::message("expected applied owned step"));
        }
    };
    let stable_program = match stable_session.step() {
        OwnedStepTransition::Stable(stable) => stable.into_program(),
        OwnedStepTransition::Applied(_)
        | OwnedStepTransition::Returned(_)
        | OwnedStepTransition::Failed(_) => {
            return Err(TestFailure::message("expected owned stable terminal"));
        }
    };
    ensure_eq!(stable_program.rule_count().get(), 1)?;

    let returned_program = match parse_program("a=(return)ok")?
        .into_execute::<OwnedSteps, _>(runtime_input(b"a", limits)?)?
        .step()
    {
        OwnedStepTransition::Returned(returned) => returned.into_program(),
        OwnedStepTransition::Applied(_)
        | OwnedStepTransition::Stable(_)
        | OwnedStepTransition::Failed(_) => {
            return Err(TestFailure::message("expected owned return terminal"));
        }
    };
    ensure_eq!(returned_program.rule_count().get(), 1)?;

    let failed_limits = DefaultInputRunPolicy::<1, DEFAULT_BYTE_BUDGET, 1>::new();
    let (error, failed_program) = match parse_program("a=(return)ok")?
        .into_execute::<OwnedSteps, _>(runtime_input(b"a", failed_limits)?)?
        .step()
    {
        OwnedStepTransition::Failed(failed) => failed.into_parts(),
        OwnedStepTransition::Applied(_)
        | OwnedStepTransition::Stable(_)
        | OwnedStepTransition::Returned(_) => {
            return Err(TestFailure::message("expected owned failed terminal"));
        }
    };
    ensure_matches(
        matches!(
            error,
            OwnedRunStepError::Step(RunStepError::ReturnOutputLimit(_))
        ),
        "expected owned return limit failure",
    )?;
    ensure_eq!(failed_program.rule_count().get(), 1)
}

/// # Errors
///
/// Returns `TestFailure` if owned execution transitions do not retain owned
/// rule witnesses at every public rule-witness boundary.
#[test]
fn execution_owned_transitions_retain_rule_witnesses() -> TestResult {
    let limits = default_test_run_policy();

    ensure_owned_run_witnesses(limits)?;
    ensure_owned_rule_attempt_witnesses(limits)
}

/// Ensures owned stepwise run transitions retain owned rule witnesses.
///
/// # Errors
///
/// Returns `TestFailure` if owned stepwise run witness metadata differs.
fn ensure_owned_run_witnesses<I: rsaeb::policy::RuntimeInputPolicy, E: ExecutionPolicy>(
    limits: TestRunPolicy<I, E>,
) -> TestResult {
    let execution = parse_program("a=b\nb=(return)ok")?
        .into_execute::<OwnedSteps, _>(runtime_input(b"a", limits)?)?;
    let execution = match execution.step() {
        OwnedStepTransition::Applied(applied) => {
            let (step, rule, next_execution) = applied.into_parts();
            ensure_eq!(step.get(), 1)?;
            ensure_owned_rule_witness(
                &rule,
                ExpectedOwnedRuleWitness {
                    position: 1,
                    line_number: 1,
                    lhs: b"a",
                    action: ExpectedRuleAction::Replace(b"b"),
                },
            )?;
            next_execution
        }
        OwnedStepTransition::Stable(_)
        | OwnedStepTransition::Returned(_)
        | OwnedStepTransition::Failed(_) => {
            return Err(TestFailure::message("expected owned applied witness"));
        }
    };

    match execution.step() {
        OwnedStepTransition::Returned(returned) => ensure_owned_rule_witness(
            returned.rule(),
            ExpectedOwnedRuleWitness {
                position: 2,
                line_number: 2,
                lhs: b"b",
                action: ExpectedRuleAction::Return(b"ok"),
            },
        ),
        OwnedStepTransition::Applied(_)
        | OwnedStepTransition::Stable(_)
        | OwnedStepTransition::Failed(_) => {
            Err(TestFailure::message("expected owned returned rule witness"))
        }
    }
}

/// Ensures owned rule-attempt transitions retain owned rule witnesses.
///
/// # Errors
///
/// Returns `TestFailure` if owned rule-attempt witness metadata differs.
fn ensure_owned_rule_attempt_witnesses<I: rsaeb::policy::RuntimeInputPolicy, E: ExecutionPolicy>(
    limits: TestRunPolicy<I, E>,
) -> TestResult {
    let attempt = expect_owned_rule_attempt_active(
        parse_program("z=x\na=b")?
            .into_execute::<OwnedRuleAttempts<StaticRuleAttemptPolicy<10>>, _>(runtime_input(
                b"a", limits,
            )?)?,
    )?;
    let attempt = ensure_owned_attempt_witness(
        attempt,
        ExpectedOwnedAttemptWitness::Missed {
            attempt: 1,
            rule: ExpectedOwnedRuleWitness {
                position: 1,
                line_number: 1,
                lhs: b"z",
                action: ExpectedRuleAction::Replace(b"x"),
            },
            reason: RuleMissReason::StateMismatch,
        },
    )?;
    let _attempt = ensure_owned_attempt_witness(
        attempt,
        ExpectedOwnedAttemptWitness::Applied {
            attempt: 2,
            step: 1,
            rule: ExpectedOwnedRuleWitness {
                position: 2,
                line_number: 2,
                lhs: b"a",
                action: ExpectedRuleAction::Replace(b"b"),
            },
        },
    )?;

    let final_attempt =
        expect_owned_rule_attempt_active(parse_program("z=x")?.into_execute::<OwnedRuleAttempts<
            StaticRuleAttemptPolicy<10>,
        >, _>(runtime_input(
            b"a", limits,
        )?)?)?;
    ensure_owned_final_miss_witness(final_attempt)
}

/// Ensures an owned rule-attempt transition retains its expected rule witness.
///
/// # Errors
///
/// Returns `TestFailure` if the transition or owned witness differs.
fn ensure_owned_attempt_witness<P, E, A>(
    attempt: OwnedRuleAttemptSession<P, E, A>,
    expected: ExpectedOwnedAttemptWitness<'_>,
) -> Result<OwnedRuleAttemptSession<P, E, A>, TestFailure>
where
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    match (attempt.step(), expected) {
        (
            OwnedRuleAttemptTransition::Missed(missed),
            ExpectedOwnedAttemptWitness::Missed {
                attempt: expected_attempt,
                rule,
                reason,
            },
        ) => {
            let (attempt, miss, next_attempt) = missed.into_parts();
            ensure_eq!(attempt.get(), expected_attempt)?;
            ensure_owned_rule_witness(miss.rule(), rule)?;
            ensure_eq!(miss.reason(), reason)?;
            Ok(next_attempt)
        }
        (
            OwnedRuleAttemptTransition::Applied(applied),
            ExpectedOwnedAttemptWitness::Applied {
                attempt: expected_attempt,
                step: expected_step,
                rule: expected_rule,
            },
        ) => {
            let (attempt, step, rule, next_attempt) = applied.into_parts();
            ensure_eq!(attempt.get(), expected_attempt)?;
            ensure_eq!(step.get(), expected_step)?;
            ensure_owned_rule_witness(&rule, expected_rule)?;
            Ok(next_attempt)
        }
        (
            OwnedRuleAttemptTransition::Missed(_)
            | OwnedRuleAttemptTransition::Applied(_)
            | OwnedRuleAttemptTransition::Stable(_)
            | OwnedRuleAttemptTransition::Returned(_)
            | OwnedRuleAttemptTransition::Failed(_),
            ExpectedOwnedAttemptWitness::Missed { .. }
            | ExpectedOwnedAttemptWitness::Applied { .. },
        ) => Err(TestFailure::message(
            "expected owned rule-attempt witness transition",
        )),
    }
}

/// Ensures an owned stable final miss retains its rule witness.
///
/// # Errors
///
/// Returns `TestFailure` if the terminal transition or final miss witness differs.
fn ensure_owned_final_miss_witness<P, E, A>(attempt: OwnedRuleAttemptSession<P, E, A>) -> TestResult
where
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    match attempt.step() {
        OwnedRuleAttemptTransition::Stable(stable) => {
            let final_miss = stable.final_miss();
            ensure_owned_rule_witness(
                final_miss.rule(),
                ExpectedOwnedRuleWitness {
                    position: 1,
                    line_number: 1,
                    lhs: b"z",
                    action: ExpectedRuleAction::Replace(b"x"),
                },
            )?;
            ensure_eq!(final_miss.reason(), RuleMissReason::StateMismatch)
        }
        OwnedRuleAttemptTransition::Missed(_)
        | OwnedRuleAttemptTransition::Applied(_)
        | OwnedRuleAttemptTransition::Returned(_)
        | OwnedRuleAttemptTransition::Failed(_) => Err(TestFailure::message(
            "expected owned stable final-miss witness",
        )),
    }
}

/// # Errors
///
/// Returns `TestFailure` if owned rule-attempt terminal states cannot return
/// the parsed program to the caller.
#[test]
fn execution_owned_rule_attempt_terminals_can_return_program() -> TestResult {
    let limits = default_test_run_policy();

    let stable_start = parse_program("a=b")?
        .into_execute::<OwnedRuleAttempts<StaticRuleAttemptPolicy<10>>, _>(runtime_input(
            b"z", limits,
        )?)?;
    let stable_program = match expect_owned_rule_attempt_active(stable_start)?.step() {
        OwnedRuleAttemptTransition::Stable(stable) => stable.into_program(),
        OwnedRuleAttemptTransition::Missed(_)
        | OwnedRuleAttemptTransition::Applied(_)
        | OwnedRuleAttemptTransition::Returned(_)
        | OwnedRuleAttemptTransition::Failed(_) => {
            return Err(TestFailure::message("expected owned rule-attempt stable"));
        }
    };
    ensure_eq!(stable_program.rule_count().get(), 1)?;

    let returned_start = parse_program("a=(return)ok")?.into_execute::<OwnedRuleAttempts<
        StaticRuleAttemptPolicy<10>,
    >, _>(runtime_input(b"a", limits)?)?;
    let returned_program = match expect_owned_rule_attempt_active(returned_start)?.step() {
        OwnedRuleAttemptTransition::Returned(returned) => returned.into_program(),
        OwnedRuleAttemptTransition::Missed(_)
        | OwnedRuleAttemptTransition::Applied(_)
        | OwnedRuleAttemptTransition::Stable(_)
        | OwnedRuleAttemptTransition::Failed(_) => {
            return Err(TestFailure::message("expected owned rule-attempt return"));
        }
    };
    ensure_eq!(returned_program.rule_count().get(), 1)?;

    let failed_limits = DefaultInputRunPolicy::<10, 1, DEFAULT_BYTE_BUDGET>::new();
    let failed_start =
        parse_program("a=aa")?.into_execute::<OwnedRuleAttempts<StaticRuleAttemptPolicy<10>>, _>(
            runtime_input(b"a", failed_limits)?,
        )?;
    let failed_program = match expect_owned_rule_attempt_active(failed_start)?.step() {
        OwnedRuleAttemptTransition::Failed(failed) => failed.into_program(),
        OwnedRuleAttemptTransition::Missed(_)
        | OwnedRuleAttemptTransition::Applied(_)
        | OwnedRuleAttemptTransition::Stable(_)
        | OwnedRuleAttemptTransition::Returned(_) => {
            return Err(TestFailure::message("expected owned rule-attempt failure"));
        }
    };
    ensure_eq!(failed_program.rule_count().get(), 1)
}

/// # Errors
///
/// Returns `TestFailure` if a failed step does not preserve the uncommitted
/// state as a terminal transition.
#[test]
fn execution_step_failure_is_terminal_transition() -> TestResult {
    let program = parse_program("a=(return)ok")?;
    let limits = DefaultInputRunPolicy::<1, DEFAULT_BYTE_BUDGET, 1>::new();
    let execution = program.execute::<BorrowedSteps, _>(runtime_input(b"a", limits)?)?;

    let failed = expect_failed_transition(execution.step())?;
    ensure_eq!(failed.completed_steps().get(), 0)?;
    ensure_eq!(
        runtime_view_bytes(failed.state())?.as_slice(),
        b"a".as_slice(),
    )?;
    ensure_matches(
        matches!(
            failed.error(),
            RunStepError::ReturnOutputLimit(error)
                if error.limit() == ReturnByteLimit::new(1)
                    && error.attempted_len().get() == 2
        ),
        "expected return limit failure",
    )
}

/// # Errors
///
/// Returns `TestFailure` if a failed later step loses completed progress or
/// the current uncommitted state.
#[test]
fn execution_step_failure_preserves_current_progress() -> TestResult {
    let program = parse_program("a=b\nb=c")?;
    let limits = DefaultInputRunPolicy::<1, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new();
    let execution = program.execute::<BorrowedSteps, _>(runtime_input(b"a", limits)?)?;

    let running = match expect_step_transition(execution.step())? {
        BorrowedStepTransition::Applied(applied) => applied.into_session(),
        BorrowedStepTransition::Stable(_)
        | BorrowedStepTransition::Returned(_)
        | BorrowedStepTransition::Failed(_) => {
            return Err(TestFailure::message("expected applied execution"));
        }
    };
    let failed = expect_failed_transition(running.step())?;
    ensure_eq!(failed.completed_steps().get(), 1)?;
    ensure_eq!(
        runtime_view_bytes(failed.state())?.as_slice(),
        b"b".as_slice(),
    )?;
    ensure_matches(
        matches!(
            failed.into_error(),
            RunStepError::StepLimit(error) if error.completed_steps().get() == 1
        ),
        "expected completed-step limit failure",
    )
}
