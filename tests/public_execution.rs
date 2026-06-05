//! Public stepwise execution contract tests.

#[path = "support/runtime.rs"]
mod runtime_support;
mod support;

use rsaeb::error::{RuleAttemptStepError, RunStepError};
use rsaeb::execution::{
    BorrowedAppliedStep, BorrowedContinuingRuleAttemptTransition, BorrowedFailedRun,
    BorrowedFinalRuleAttemptTransition, BorrowedReturnedRun, BorrowedRuleAttemptCursor,
    BorrowedRuleAttemptFailedRun, BorrowedRunSession, BorrowedStableRun, BorrowedStepTransition,
    OnceConsumedRuleMiss, RuleMiss,
};
use rsaeb::input::AdmittedRun;
use rsaeb::inspect::{ReturnRuleView, RewriteActionView, RewriteRuleView, RuleAnchor, RuleView};
use rsaeb::limits::{ReturnByteLimit, RuleAttemptLimit, RuntimeStateByteLimit};
use rsaeb::policy::{
    DefaultParsePolicy, ExecutionPolicy, RuleAttemptPolicy, StaticRuleAttemptPolicy,
};
use rsaeb::program::{EmptyProgram, ExecutableProgram, RunOutcome, RunResult};
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
enum BorrowedRuleAttemptSignature {
    Missed {
        attempt: usize,
        miss: RuleMissSignature,
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
        final_miss: RuleMissSignature,
        state: Vec<u8>,
    },
    Return {
        attempt: usize,
        step: usize,
        rule_position: usize,
        output: Vec<u8>,
    },
}

#[derive(Debug, PartialEq, Eq)]
enum RuleMissSignature {
    StateMismatch { rule_position: usize },
    OnceRewriteConsumed { rule_position: usize },
    OnceReturnConsumed { rule_position: usize },
}

macro_rules! borrowed_state_mismatch {
    ($attempt:expr, $rule_position:expr, $state:expr) => {
        BorrowedRuleAttemptSignature::Missed {
            attempt: $attempt,
            miss: RuleMissSignature::StateMismatch {
                rule_position: $rule_position,
            },
            state: $state.to_vec(),
        }
    };
}

macro_rules! borrowed_once_rewrite_consumed {
    ($attempt:expr, $rule_position:expr, $state:expr) => {
        BorrowedRuleAttemptSignature::Missed {
            attempt: $attempt,
            miss: RuleMissSignature::OnceRewriteConsumed {
                rule_position: $rule_position,
            },
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

macro_rules! collect_borrowed_rule_attempt_signatures {
    ($execution:expr) => {{
        let mut cursor = $execution;
        let mut signatures = Vec::new();
        loop {
            match cursor {
                BorrowedRuleAttemptCursor::Continuing(execution) => {
                    match expect_continuing_rule_attempt_transition(execution.step())? {
                        BorrowedContinuingRuleAttemptTransition::Missed(missed) => {
                            signatures.push(BorrowedRuleAttemptSignature::Missed {
                                attempt: missed.attempt().get(),
                                miss: rule_miss_signature(missed.miss()),
                                state: runtime_view_bytes(missed.state())?,
                            });
                            cursor = missed.into_cursor();
                        }
                        BorrowedContinuingRuleAttemptTransition::Applied(applied) => {
                            signatures.push(BorrowedRuleAttemptSignature::Applied {
                                attempt: applied.attempt().get(),
                                step: applied.step().get(),
                                rule_position: applied.rule().position().get(),
                                state: runtime_view_bytes(applied.state())?,
                            });
                            cursor = applied.into_cursor();
                        }
                        BorrowedContinuingRuleAttemptTransition::Returned(returned) => {
                            signatures.push(BorrowedRuleAttemptSignature::Return {
                                attempt: returned.attempt().get(),
                                step: returned.step().get(),
                                rule_position: returned.rule().position().get(),
                                output: returned.output().as_slice().to_vec(),
                            });
                            return Ok(signatures);
                        }
                        BorrowedContinuingRuleAttemptTransition::Failed(failed) => {
                            return Err(TestFailure::from(failed.into_error()));
                        }
                    }
                }
                BorrowedRuleAttemptCursor::Final(execution) => {
                    match expect_final_rule_attempt_transition(execution.step())? {
                        BorrowedFinalRuleAttemptTransition::Stable(stable) => {
                            signatures.push(BorrowedRuleAttemptSignature::Stable {
                                attempts: stable.attempts().get(),
                                steps: stable.steps().get(),
                                final_miss: rule_miss_signature(stable.final_miss()),
                                state: runtime_view_bytes(stable.state())?,
                            });
                            return Ok(signatures);
                        }
                        BorrowedFinalRuleAttemptTransition::Applied(applied) => {
                            signatures.push(BorrowedRuleAttemptSignature::Applied {
                                attempt: applied.attempt().get(),
                                step: applied.step().get(),
                                rule_position: applied.rule().position().get(),
                                state: runtime_view_bytes(applied.state())?,
                            });
                            cursor = applied.into_cursor();
                        }
                        BorrowedFinalRuleAttemptTransition::Returned(returned) => {
                            signatures.push(BorrowedRuleAttemptSignature::Return {
                                attempt: returned.attempt().get(),
                                step: returned.step().get(),
                                rule_position: returned.rule().position().get(),
                                output: returned.output().as_slice().to_vec(),
                            });
                            return Ok(signatures);
                        }
                        BorrowedFinalRuleAttemptTransition::Failed(failed) => {
                            return Err(TestFailure::from(failed.into_error()));
                        }
                    }
                }
            }
        }
    }};
}

struct ExpectedBorrowedRuleView<'expected> {
    position: usize,
    line_number: usize,
    lhs: &'expected [u8],
    canonical_source: &'expected [u8],
}

/// Ensures a committed rewrite witness retained the expected parsed-rule metadata.
///
/// # Errors
///
/// Returns `TestFailure` if the rule metadata or materialized payloads differ.
fn ensure_borrowed_rewrite_rule_view(
    rule: RewriteRuleView<'_>,
    expected: ExpectedBorrowedRuleView<'_>,
    expected_replacement: &[u8],
) -> TestResult {
    ensure_eq!(rule.position().get(), expected.position)?;
    ensure_eq!(rule.line_number().get(), expected.line_number)?;
    ensure_eq!(rule.anchor(), RuleAnchor::Anywhere)?;
    ensure_eq!(rule.lhs().materialize()?.as_slice(), expected.lhs)?;
    ensure_eq!(
        rule.canonical_source()?.as_slice(),
        expected.canonical_source,
    )?;
    match rule.rewrite_action() {
        RewriteActionView::Replace(payload) => {
            ensure_eq!(payload.materialize()?.as_slice(), expected_replacement)
        }
        RewriteActionView::MoveStart(_) | RewriteActionView::MoveEnd(_) => {
            Err(TestFailure::message("unexpected borrowed rewrite action"))
        }
    }
}

/// Ensures a committed return witness retained the expected parsed-rule metadata.
///
/// # Errors
///
/// Returns `TestFailure` if the rule metadata or materialized payloads differ.
fn ensure_borrowed_return_rule_view(
    rule: ReturnRuleView<'_>,
    expected: ExpectedBorrowedRuleView<'_>,
    expected_output: &[u8],
) -> TestResult {
    ensure_eq!(rule.position().get(), expected.position)?;
    ensure_eq!(rule.line_number().get(), expected.line_number)?;
    ensure_eq!(rule.anchor(), RuleAnchor::Anywhere)?;
    ensure_eq!(rule.lhs().materialize()?.as_slice(), expected.lhs)?;
    ensure_eq!(
        rule.canonical_source()?.as_slice(),
        expected.canonical_source,
    )?;
    ensure_eq!(rule.output().materialize()?.as_slice(), expected_output,)
}

/// Verifies a committed rewrite witness has the expected repeat shape.
///
/// # Errors
///
/// Returns `TestFailure` if the repeat shape differs.
fn ensure_rewrite_rule_shape(rule: RewriteRuleView<'_>, expected_once: bool) -> TestResult {
    ensure_matches(
        matches!(
            (rule, expected_once),
            (RewriteRuleView::Always(_), false) | (RewriteRuleView::Once(_), true)
        ),
        "unexpected committed rewrite rule shape",
    )
}

/// Verifies a committed return witness has the expected repeat shape.
///
/// # Errors
///
/// Returns `TestFailure` if the repeat shape differs.
fn ensure_return_rule_shape(rule: ReturnRuleView<'_>, expected_once: bool) -> TestResult {
    ensure_matches(
        matches!(
            (rule, expected_once),
            (ReturnRuleView::Always(_), false) | (ReturnRuleView::Once(_), true)
        ),
        "unexpected committed return rule shape",
    )
}

/// Verifies one stepwise rewrite outcome carries its exact repeat/action witness.
///
/// # Errors
///
/// Returns `TestFailure` if the program does not produce the expected rewrite.
fn ensure_stepwise_rewrite_rule_shape(source: &str, expected_once: bool) -> TestResult {
    let program = parse_program(source)?;
    match program
        .steps(runtime_input(b"a", default_test_run_policy())?)?
        .step()
    {
        BorrowedStepTransition::Applied(applied) => {
            ensure_rewrite_rule_shape(applied.rule(), expected_once)
        }
        BorrowedStepTransition::Stable(_)
        | BorrowedStepTransition::Returned(_)
        | BorrowedStepTransition::Failed(_) => {
            Err(TestFailure::message("expected stepwise rewrite outcome"))
        }
    }
}

/// Verifies one stepwise return outcome carries its exact repeat/action witness.
///
/// # Errors
///
/// Returns `TestFailure` if the program does not produce the expected return.
fn ensure_stepwise_return_rule_shape(source: &str, expected_once: bool) -> TestResult {
    let program = parse_program(source)?;
    match program
        .steps(runtime_input(b"a", default_test_run_policy())?)?
        .step()
    {
        BorrowedStepTransition::Returned(returned) => {
            ensure_return_rule_shape(returned.rule(), expected_once)
        }
        BorrowedStepTransition::Applied(_)
        | BorrowedStepTransition::Stable(_)
        | BorrowedStepTransition::Failed(_) => {
            Err(TestFailure::message("expected stepwise return outcome"))
        }
    }
}

/// Verifies one rule-attempt rewrite outcome carries its exact repeat/action witness.
///
/// # Errors
///
/// Returns `TestFailure` if the program does not produce the expected rewrite.
fn ensure_rule_attempt_rewrite_rule_shape(source: &str, expected_once: bool) -> TestResult {
    let program = parse_program(source)?;
    let cursor = program.rule_attempts::<StaticRuleAttemptPolicy<1>, _>(runtime_input(
        b"a",
        default_test_run_policy(),
    )?)?;
    let BorrowedRuleAttemptCursor::Final(execution) = cursor else {
        return Err(TestFailure::message("expected final rule-attempt cursor"));
    };
    match expect_final_rule_attempt_transition(execution.step())? {
        BorrowedFinalRuleAttemptTransition::Applied(applied) => {
            ensure_rewrite_rule_shape(applied.rule(), expected_once)
        }
        BorrowedFinalRuleAttemptTransition::Stable(_)
        | BorrowedFinalRuleAttemptTransition::Returned(_)
        | BorrowedFinalRuleAttemptTransition::Failed(_) => Err(TestFailure::message(
            "expected rule-attempt rewrite outcome",
        )),
    }
}

/// Verifies one rule-attempt return outcome carries its exact repeat/action witness.
///
/// # Errors
///
/// Returns `TestFailure` if the program does not produce the expected return.
fn ensure_rule_attempt_return_rule_shape(source: &str, expected_once: bool) -> TestResult {
    let program = parse_program(source)?;
    let cursor = program.rule_attempts::<StaticRuleAttemptPolicy<1>, _>(runtime_input(
        b"a",
        default_test_run_policy(),
    )?)?;
    let BorrowedRuleAttemptCursor::Final(execution) = cursor else {
        return Err(TestFailure::message("expected final rule-attempt cursor"));
    };
    match expect_final_rule_attempt_transition(execution.step())? {
        BorrowedFinalRuleAttemptTransition::Returned(returned) => {
            ensure_return_rule_shape(returned.rule(), expected_once)
        }
        BorrowedFinalRuleAttemptTransition::Applied(_)
        | BorrowedFinalRuleAttemptTransition::Stable(_)
        | BorrowedFinalRuleAttemptTransition::Failed(_) => {
            Err(TestFailure::message("expected rule-attempt return outcome"))
        }
    }
}

fn rule_miss_signature(miss: &RuleMiss<'_>) -> RuleMissSignature {
    match *miss {
        RuleMiss::StateMismatch(miss) => RuleMissSignature::StateMismatch {
            rule_position: miss.rule().position().get(),
        },
        RuleMiss::OnceConsumed(OnceConsumedRuleMiss::Rewrite(rule)) => {
            RuleMissSignature::OnceRewriteConsumed {
                rule_position: rule.position().get(),
            }
        }
        RuleMiss::OnceConsumed(OnceConsumedRuleMiss::Return(rule)) => {
            RuleMissSignature::OnceReturnConsumed {
                rule_position: rule.position().get(),
            }
        }
    }
}

/// Builds a comparable signature for an applied step.
///
/// # Errors
///
/// Returns `TestFailure` if state materialization fails.
fn applied_signature<E: ExecutionPolicy>(
    applied: &BorrowedAppliedStep<'_, E>,
) -> Result<StepSignature, TestFailure> {
    Ok(StepSignature::Applied {
        step: applied.step().get(),
        rule_position: applied.rule().position().get(),
        state: runtime_view_bytes(applied.state())?,
    })
}

/// Builds a comparable signature for a stable terminal state.
///
/// # Errors
///
/// Returns `TestFailure` if stable-state materialization fails.
fn stable_signature(stable: &BorrowedStableRun<'_>) -> Result<StepSignature, TestFailure> {
    Ok(StepSignature::Stable {
        steps: stable.steps().get(),
        state: runtime_view_bytes(stable.state())?,
    })
}

/// Builds a comparable signature for a returned step.
fn returned_signature(returned: &BorrowedReturnedRun<'_>) -> StepSignature {
    StepSignature::Return {
        step: returned.step().get(),
        rule_position: returned.rule().position().get(),
        output: returned.output().as_slice().to_vec(),
    }
}

fn default_test_run_policy() -> DefaultInputRunPolicy<10, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>
{
    DefaultInputRunPolicy::<10, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new()
}

/// Runs borrowed rule-attempt execution and collects comparable transition signatures.
///
/// # Errors
///
/// Returns `TestFailure` if the program cannot be parsed, input is rejected, or
/// rule-attempt execution fails.
fn borrowed_rule_attempt_signatures<const ATTEMPTS: usize>(
    program: &ExecutableProgram,
    input: &'static [u8],
) -> Result<Vec<BorrowedRuleAttemptSignature>, TestFailure> {
    let execution = program.rule_attempts::<StaticRuleAttemptPolicy<ATTEMPTS>, _>(
        runtime_input(input, default_test_run_policy())?,
    )?;
    finish_borrowed_rule_attempt_signatures(execution)
}

/// Runs stepwise execution and collects comparable transition signatures.
///
/// # Errors
///
/// Returns `TestFailure` if a step fails or transition materialization fails.
fn finish_step_signatures<E: ExecutionPolicy>(
    mut execution: BorrowedRunSession<'_, E>,
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

/// Runs borrowed rule-attempt execution and collects comparable transition signatures.
///
/// # Errors
///
/// Returns `TestFailure` if a rule attempt fails or state materialization fails.
fn finish_borrowed_rule_attempt_signatures<E, A>(
    execution: BorrowedRuleAttemptCursor<'_, E, A>,
) -> Result<Vec<BorrowedRuleAttemptSignature>, TestFailure>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    collect_borrowed_rule_attempt_signatures!(execution)
}

/// Returns the expected successful step transition.
///
/// # Errors
///
/// Returns `TestFailure` if stepping fails.
fn expect_step_transition<'program, E: ExecutionPolicy>(
    result: BorrowedStepTransition<'program, E>,
) -> Result<BorrowedStepTransition<'program, E>, TestFailure> {
    expect_non_failed_transition!(result, BorrowedStepTransition::Failed)
}

/// Returns the expected failed step transition.
///
/// # Errors
///
/// Returns `TestFailure` if stepping does not fail.
fn expect_failed_transition<'program, E: ExecutionPolicy>(
    result: BorrowedStepTransition<'program, E>,
) -> Result<BorrowedFailedRun<'program>, TestFailure> {
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

/// Executes a parsed program that is expected to contain executable rules.
///
/// # Errors
///
/// Returns `TestFailure` if execution fails.
fn execute_program<E>(
    program: &ExecutableProgram,
    admitted: AdmittedRun<E>,
) -> Result<RunResult, TestFailure>
where
    E: ExecutionPolicy,
{
    Ok(program.execute(admitted)?)
}

/// Returns the expected successful continuing rule-attempt transition.
///
/// # Errors
///
/// Returns `TestFailure` if stepping fails.
fn expect_continuing_rule_attempt_transition<'program, E, A>(
    result: BorrowedContinuingRuleAttemptTransition<'program, E, A>,
) -> Result<BorrowedContinuingRuleAttemptTransition<'program, E, A>, TestFailure>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    expect_non_failed_transition!(result, BorrowedContinuingRuleAttemptTransition::Failed)
}

/// Returns the expected successful final rule-attempt transition.
///
/// # Errors
///
/// Returns `TestFailure` if stepping fails.
fn expect_final_rule_attempt_transition<'program, E, A>(
    result: BorrowedFinalRuleAttemptTransition<'program, E, A>,
) -> Result<BorrowedFinalRuleAttemptTransition<'program, E, A>, TestFailure>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    expect_non_failed_transition!(result, BorrowedFinalRuleAttemptTransition::Failed)
}

/// Returns the expected failed continuing rule-attempt transition.
///
/// # Errors
///
/// Returns `TestFailure` if stepping does not fail.
fn expect_failed_continuing_rule_attempt<'program, E, A>(
    result: BorrowedContinuingRuleAttemptTransition<'program, E, A>,
) -> Result<BorrowedRuleAttemptFailedRun<'program>, TestFailure>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    match result {
        BorrowedContinuingRuleAttemptTransition::Failed(failed) => Ok(failed),
        BorrowedContinuingRuleAttemptTransition::Missed(_)
        | BorrowedContinuingRuleAttemptTransition::Applied(_)
        | BorrowedContinuingRuleAttemptTransition::Returned(_) => {
            Err(TestFailure::message("expected failed rule attempt"))
        }
    }
}

/// Returns the expected failed final rule-attempt transition.
///
/// # Errors
///
/// Returns `TestFailure` if stepping does not fail.
fn expect_failed_final_rule_attempt<'program, E, A>(
    result: BorrowedFinalRuleAttemptTransition<'program, E, A>,
) -> Result<BorrowedRuleAttemptFailedRun<'program>, TestFailure>
where
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    match result {
        BorrowedFinalRuleAttemptTransition::Failed(failed) => Ok(failed),
        BorrowedFinalRuleAttemptTransition::Stable(_)
        | BorrowedFinalRuleAttemptTransition::Applied(_)
        | BorrowedFinalRuleAttemptTransition::Returned(_) => {
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
    let result = execute_program(&program, runtime_input(b"aaaa", limits)?)?;
    expect_stable_bytes(&result, b"xx")?;

    let program = parse_program("(start)a=x")?;
    let result = execute_program(&program, runtime_input(b"aba", limits)?)?;
    expect_stable_bytes(&result, b"xba")?;

    let program = parse_program("(end)a=x")?;
    let result = execute_program(&program, runtime_input(b"aba", limits)?)?;
    expect_stable_bytes(&result, b"abx")?;

    let program = parse_program("(once)a=b\na=c")?;
    let result = execute_program(&program, runtime_input(b"aa", limits)?)?;
    expect_stable_bytes(&result, b"bc")?;

    let program = parse_program("ab=x")?;
    let result = execute_program(&program, runtime_input(b"a=b", limits)?)?;
    expect_stable_bytes(&result, b"a=b")?;

    let program = parse_program("a= b")?;
    let result = execute_program(&program, runtime_input(b"a bc", limits)?)?;
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
    let execution = program.steps(input)?;
    ensure_eq!(execution.completed_steps().get(), 0)?;

    let execution = match expect_step_transition(execution.step())? {
        BorrowedStepTransition::Applied(applied) => {
            ensure_eq!(applied.step().get(), 1)?;
            ensure_eq!(applied.rule().position().get(), 1)?;
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
            ensure_eq!(applied.rule().position().get(), 2)?;
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
            borrowed_state_mismatch!(1, 1, b"a"),
            borrowed_apply!(2, 1, 2, b"b"),
            borrowed_state_mismatch!(3, 1, b"b"),
            borrowed_state_mismatch!(4, 2, b"b"),
            borrowed_apply!(5, 2, 3, b"c"),
            borrowed_state_mismatch!(6, 1, b"c"),
            borrowed_state_mismatch!(7, 2, b"c"),
            borrowed_stable!(
                8,
                2,
                RuleMissSignature::StateMismatch { rule_position: 3 },
                b"c",
            ),
        ],
    )
}

/// # Errors
///
/// Returns `TestFailure` if borrowed rule-attempt execution loses return
/// semantics after miss and reset transitions.
#[test]
fn execution_rule_attempt_surface_reports_misses_resets_and_returns() -> TestResult {
    let program = parse_program("z=x\na=b\nb=(return)ok")?;
    ensure_eq!(
        borrowed_rule_attempt_signatures::<20>(&program, b"a")?,
        [
            BorrowedRuleAttemptSignature::Missed {
                attempt: 1,
                miss: RuleMissSignature::StateMismatch { rule_position: 1 },
                state: b"a".to_vec(),
            },
            BorrowedRuleAttemptSignature::Applied {
                attempt: 2,
                step: 1,
                rule_position: 2,
                state: b"b".to_vec(),
            },
            BorrowedRuleAttemptSignature::Missed {
                attempt: 3,
                miss: RuleMissSignature::StateMismatch { rule_position: 1 },
                state: b"b".to_vec(),
            },
            BorrowedRuleAttemptSignature::Missed {
                attempt: 4,
                miss: RuleMissSignature::StateMismatch { rule_position: 2 },
                state: b"b".to_vec(),
            },
            BorrowedRuleAttemptSignature::Return {
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
    let cursor = program.rule_attempts::<StaticRuleAttemptPolicy<10>, _>(input)?;

    let BorrowedRuleAttemptCursor::Final(execution) = cursor else {
        return Err(TestFailure::message(
            "expected single-rule start as final cursor",
        ));
    };
    match expect_final_rule_attempt_transition(execution.step())? {
        BorrowedFinalRuleAttemptTransition::Stable(stable) => {
            ensure_eq!(stable.attempts().get(), 1)?;
            ensure_eq!(stable.steps().get(), 0)?;
            let final_miss = stable.final_miss();
            ensure_eq!(
                rule_miss_signature(final_miss),
                RuleMissSignature::StateMismatch { rule_position: 1 },
            )?;
            ensure_eq!(
                runtime_view_bytes(stable.state())?.as_slice(),
                b"z".as_slice(),
            )?;
        }
        BorrowedFinalRuleAttemptTransition::Applied(_)
        | BorrowedFinalRuleAttemptTransition::Returned(_)
        | BorrowedFinalRuleAttemptTransition::Failed(_) => {
            return Err(TestFailure::message("expected immediate stable terminal"));
        }
    }
    let empty_program = EmptyProgram::parse_text::<DefaultParsePolicy>("# no executable rules")?;
    let borrowed_empty_result = empty_program.stabilize(runtime_input(b"empty", limits)?)?;
    expect_stable_bytes(&borrowed_empty_result, b"empty")?;
    ensure_eq!(borrowed_empty_result.steps().get(), 0)?;

    let owned_empty = EmptyProgram::parse_text::<DefaultParsePolicy>("# no executable rules")?;
    let owned_empty_result = owned_empty.stabilize(runtime_input(b"owned", limits)?)?;
    expect_stable_bytes(&owned_empty_result, b"owned")?;
    ensure_eq!(owned_empty_result.steps().get(), 0)?;

    EmptyProgram::parse_text::<DefaultParsePolicy>("# no executable rules")?;
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if public rule-attempt misses collapse state mismatch
/// and consumed `(once)` rules back into one reason-bearing shape.
#[test]
fn execution_rule_attempt_miss_shapes_are_typed() -> TestResult {
    let limits = default_test_run_policy();
    let program = parse_program("(once)a=b\nz=z")?;
    let input = runtime_input(b"a", limits)?;
    let cursor = program.rule_attempts::<StaticRuleAttemptPolicy<10>, _>(input)?;

    let BorrowedRuleAttemptCursor::Continuing(execution) = cursor else {
        return Err(TestFailure::message(
            "expected two-rule start as continuing cursor",
        ));
    };
    let cursor = match expect_continuing_rule_attempt_transition(execution.step())? {
        BorrowedContinuingRuleAttemptTransition::Applied(applied) => applied.into_cursor(),
        BorrowedContinuingRuleAttemptTransition::Missed(_)
        | BorrowedContinuingRuleAttemptTransition::Returned(_)
        | BorrowedContinuingRuleAttemptTransition::Failed(_) => {
            return Err(TestFailure::message("expected first once rewrite to apply"));
        }
    };

    let BorrowedRuleAttemptCursor::Continuing(execution) = cursor else {
        return Err(TestFailure::message(
            "expected reset cursor to revisit first rule",
        ));
    };
    let cursor = match expect_continuing_rule_attempt_transition(execution.step())? {
        BorrowedContinuingRuleAttemptTransition::Missed(missed) => {
            ensure_eq!(
                rule_miss_signature(missed.miss()),
                RuleMissSignature::OnceRewriteConsumed { rule_position: 1 },
            )?;
            missed.into_cursor()
        }
        BorrowedContinuingRuleAttemptTransition::Applied(_)
        | BorrowedContinuingRuleAttemptTransition::Returned(_)
        | BorrowedContinuingRuleAttemptTransition::Failed(_) => {
            return Err(TestFailure::message("expected consumed once rewrite miss"));
        }
    };

    let BorrowedRuleAttemptCursor::Final(execution) = cursor else {
        return Err(TestFailure::message(
            "expected always rule to become final cursor",
        ));
    };
    match expect_final_rule_attempt_transition(execution.step())? {
        BorrowedFinalRuleAttemptTransition::Stable(stable) => ensure_eq!(
            rule_miss_signature(stable.final_miss()),
            RuleMissSignature::StateMismatch { rule_position: 2 },
        ),
        BorrowedFinalRuleAttemptTransition::Applied(_)
        | BorrowedFinalRuleAttemptTransition::Returned(_)
        | BorrowedFinalRuleAttemptTransition::Failed(_) => {
            Err(TestFailure::message("expected always rule state mismatch"))
        }
    }
}

/// # Errors
///
/// Returns `TestFailure` if a committed rewrite resets the rule-attempt cursor
/// to a shape-erased state instead of the typed first-rule cursor.
#[test]
fn execution_rule_attempt_rewrite_reset_returns_typed_cursor() -> TestResult {
    let limits = default_test_run_policy();

    let single_rule = parse_program("a=b")?;
    let input = runtime_input(b"a", limits)?;
    let cursor = single_rule.rule_attempts::<StaticRuleAttemptPolicy<10>, _>(input)?;
    let BorrowedRuleAttemptCursor::Final(execution) = cursor else {
        return Err(TestFailure::message(
            "expected single-rule start as final cursor",
        ));
    };
    let cursor = match expect_final_rule_attempt_transition(execution.step())? {
        BorrowedFinalRuleAttemptTransition::Applied(applied) => applied.into_cursor(),
        BorrowedFinalRuleAttemptTransition::Stable(_)
        | BorrowedFinalRuleAttemptTransition::Returned(_)
        | BorrowedFinalRuleAttemptTransition::Failed(_) => {
            return Err(TestFailure::message("expected single-rule rewrite apply"));
        }
    };
    let BorrowedRuleAttemptCursor::Final(_) = cursor else {
        return Err(TestFailure::message(
            "expected single-rule rewrite reset to final cursor",
        ));
    };

    let multi_rule = parse_program("a=b\nb=c")?;
    let input = runtime_input(b"a", limits)?;
    let cursor = multi_rule.rule_attempts::<StaticRuleAttemptPolicy<10>, _>(input)?;
    let BorrowedRuleAttemptCursor::Continuing(execution) = cursor else {
        return Err(TestFailure::message(
            "expected multi-rule start as continuing cursor",
        ));
    };
    let cursor = match expect_continuing_rule_attempt_transition(execution.step())? {
        BorrowedContinuingRuleAttemptTransition::Applied(applied) => applied.into_cursor(),
        BorrowedContinuingRuleAttemptTransition::Missed(_)
        | BorrowedContinuingRuleAttemptTransition::Returned(_)
        | BorrowedContinuingRuleAttemptTransition::Failed(_) => {
            return Err(TestFailure::message("expected multi-rule rewrite apply"));
        }
    };
    let BorrowedRuleAttemptCursor::Continuing(_) = cursor else {
        return Err(TestFailure::message(
            "expected multi-rule rewrite reset to continuing cursor",
        ));
    };
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if interleaved always rules consume `(once)` state or
/// consumed `(once)` rules stop being reported as typed rule-attempt misses.
#[test]
fn execution_rule_attempt_preserves_interleaved_once_state() -> TestResult {
    let program = parse_program("(once)a=b\nz=z\n(once)b=c")?;
    let once_rules = program
        .rules()
        .filter(|rule| matches!(rule, RuleView::OnceRewrite(_) | RuleView::OnceReturn(_)))
        .count();
    ensure_eq!(once_rules, 2)?;
    ensure_eq!(
        borrowed_rule_attempt_signatures::<10>(&program, b"a")?,
        vec![
            borrowed_apply!(1, 1, 1, b"b"),
            borrowed_once_rewrite_consumed!(2, 1, b"b"),
            borrowed_state_mismatch!(3, 2, b"b"),
            borrowed_apply!(4, 2, 3, b"c"),
            borrowed_once_rewrite_consumed!(5, 1, b"c"),
            borrowed_state_mismatch!(6, 2, b"c"),
            borrowed_stable!(
                7,
                2,
                RuleMissSignature::OnceRewriteConsumed { rule_position: 3 },
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
        borrowed_once_rewrite_consumed!(2, 1, b"b"),
        borrowed_apply!(3, 2, 2, b"c"),
        borrowed_once_rewrite_consumed!(4, 1, b"c"),
        borrowed_stable!(
            5,
            2,
            RuleMissSignature::StateMismatch { rule_position: 2 },
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
    let cursor = program.rule_attempts::<StaticRuleAttemptPolicy<0>, _>(input)?;
    let BorrowedRuleAttemptCursor::Continuing(execution) = cursor else {
        return Err(TestFailure::message(
            "expected multi-rule start as continuing cursor",
        ));
    };
    let failed = expect_failed_continuing_rule_attempt(execution.step())?;
    ensure_eq!(failed.completed_attempts().get(), 0)?;
    ensure_eq!(failed.completed_steps().get(), 0)?;
    ensure_matches(
        matches!(
            failed.into_error(),
            RuleAttemptStepError::RuleAttemptLimit(error)
                if error.max_attempts() == RuleAttemptLimit::new(0)
                    && error.completed_attempts().get() == 0
                    && error.state_len().get() == 1
        ),
        "expected continuing rule-attempt limit details",
    )?;

    let input = runtime_input(b"a", limits)?;
    let cursor = program.rule_attempts::<StaticRuleAttemptPolicy<1>, _>(input)?;
    let BorrowedRuleAttemptCursor::Continuing(execution) = cursor else {
        return Err(TestFailure::message(
            "expected multi-rule start as continuing cursor",
        ));
    };

    let cursor = match expect_continuing_rule_attempt_transition(execution.step())? {
        BorrowedContinuingRuleAttemptTransition::Missed(missed) => {
            ensure_eq!(missed.attempt().get(), 1)?;
            ensure_eq!(
                rule_miss_signature(missed.miss()),
                RuleMissSignature::StateMismatch { rule_position: 1 },
            )?;
            missed.into_cursor()
        }
        BorrowedContinuingRuleAttemptTransition::Applied(_)
        | BorrowedContinuingRuleAttemptTransition::Returned(_)
        | BorrowedContinuingRuleAttemptTransition::Failed(_) => {
            return Err(TestFailure::message(
                "expected miss despite zero execution-step limit",
            ));
        }
    };

    let BorrowedRuleAttemptCursor::Final(execution) = cursor else {
        return Err(TestFailure::message(
            "expected miss to advance into final cursor",
        ));
    };
    let failed = expect_failed_final_rule_attempt(execution.step())?;
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
    let cursor = program.rule_attempts::<StaticRuleAttemptPolicy<10>, _>(input)?;
    let BorrowedRuleAttemptCursor::Final(execution) = cursor else {
        return Err(TestFailure::message(
            "expected single-rule start as final cursor",
        ));
    };

    let failed = expect_failed_final_rule_attempt(execution.step())?;
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
    let execution = program.steps(input)?;

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
    let first = program.steps(runtime_input(b"aa", limits)?)?;
    let second = program.steps(runtime_input(b"aa", limits)?)?;
    let third = program.steps(runtime_input(b"aa", limits)?)?;

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
/// Returns `TestFailure` if run-to-completion and borrowed stepwise execution
/// diverge for the same source, input, and limits.
#[test]
fn execution_full_run_and_borrowed_session_share_contract() -> TestResult {
    let source = "a=b\nb=(return)ok";
    let limits = default_test_run_policy();
    let program = parse_program(source)?;

    let completed = execute_program(&program, runtime_input(b"a", limits)?)?;
    let stepped = program.steps(runtime_input(b"a", limits)?)?.finish()?;

    ensure_eq!(completed, stepped)
}

/// # Errors
///
/// Returns `TestFailure` if borrowed stepwise terminal states lose their
/// executable program witness.
#[test]
fn execution_borrowed_terminals_keep_program_witness() -> TestResult {
    let limits = default_test_run_policy();

    let stable_program = parse_program("a=b")?;
    let stable_session = stable_program.steps(runtime_input(b"a", limits)?)?;
    let stable_session = match stable_session.step() {
        BorrowedStepTransition::Applied(applied) => applied.into_session(),
        BorrowedStepTransition::Stable(_)
        | BorrowedStepTransition::Returned(_)
        | BorrowedStepTransition::Failed(_) => {
            return Err(TestFailure::message("expected applied borrowed step"));
        }
    };
    match stable_session.step() {
        BorrowedStepTransition::Stable(stable) => {
            ensure_eq!(stable.program().rule_count().get(), 1)?;
        }
        BorrowedStepTransition::Applied(_)
        | BorrowedStepTransition::Returned(_)
        | BorrowedStepTransition::Failed(_) => {
            return Err(TestFailure::message("expected borrowed stable terminal"));
        }
    };

    let returned_program = parse_program("a=(return)ok")?;
    match returned_program.steps(runtime_input(b"a", limits)?)?.step() {
        BorrowedStepTransition::Returned(returned) => {
            ensure_eq!(returned.program().rule_count().get(), 1)?;
        }
        BorrowedStepTransition::Applied(_)
        | BorrowedStepTransition::Stable(_)
        | BorrowedStepTransition::Failed(_) => {
            return Err(TestFailure::message("expected borrowed return terminal"));
        }
    };

    let failed_limits = DefaultInputRunPolicy::<1, DEFAULT_BYTE_BUDGET, 1>::new();
    let failed_program = parse_program("a=(return)ok")?;
    let failed = match failed_program
        .steps(runtime_input(b"a", failed_limits)?)?
        .step()
    {
        BorrowedStepTransition::Failed(failed) => failed,
        BorrowedStepTransition::Applied(_)
        | BorrowedStepTransition::Stable(_)
        | BorrowedStepTransition::Returned(_) => {
            return Err(TestFailure::message("expected borrowed failed terminal"));
        }
    };
    ensure_matches(
        matches!(failed.error(), RunStepError::ReturnOutputLimit(_)),
        "expected borrowed return limit failure",
    )?;
    ensure_eq!(failed.program().rule_count().get(), 1)
}

/// # Errors
///
/// Returns `TestFailure` if borrowed execution transitions do not retain
/// structured rule views at every public rule-witness boundary.
#[test]
fn execution_borrowed_transitions_retain_rule_views() -> TestResult {
    let limits = default_test_run_policy();

    let program = parse_program("a=b\nb=(return)ok")?;
    let execution = program.steps(runtime_input(b"a", limits)?)?;
    let execution = match execution.step() {
        BorrowedStepTransition::Applied(applied) => {
            ensure_eq!(applied.step().get(), 1)?;
            ensure_borrowed_rewrite_rule_view(
                applied.rule(),
                ExpectedBorrowedRuleView {
                    position: 1,
                    line_number: 1,
                    lhs: b"a",
                    canonical_source: b"a=b",
                },
                b"b",
            )?;
            applied.into_session()
        }
        BorrowedStepTransition::Stable(_)
        | BorrowedStepTransition::Returned(_)
        | BorrowedStepTransition::Failed(_) => {
            return Err(TestFailure::message("expected borrowed applied rule view"));
        }
    };

    match execution.step() {
        BorrowedStepTransition::Returned(returned) => ensure_borrowed_return_rule_view(
            returned.rule(),
            ExpectedBorrowedRuleView {
                position: 2,
                line_number: 2,
                lhs: b"b",
                canonical_source: b"b=(return)ok",
            },
            b"ok",
        ),
        BorrowedStepTransition::Applied(_)
        | BorrowedStepTransition::Stable(_)
        | BorrowedStepTransition::Failed(_) => {
            Err(TestFailure::message("expected borrowed returned rule view"))
        }
    }
}

/// # Errors
///
/// Returns `TestFailure` if successful stepwise or rule-attempt outcomes erase
/// their action or repeat provenance.
#[test]
fn execution_success_outcomes_preserve_exact_rule_shapes() -> TestResult {
    ensure_stepwise_rewrite_rule_shape("a=b", false)?;
    ensure_stepwise_rewrite_rule_shape("(once)a=b", true)?;
    ensure_stepwise_return_rule_shape("a=(return)ok", false)?;
    ensure_stepwise_return_rule_shape("(once)a=(return)ok", true)?;

    ensure_rule_attempt_rewrite_rule_shape("a=b", false)?;
    ensure_rule_attempt_rewrite_rule_shape("(once)a=b", true)?;
    ensure_rule_attempt_return_rule_shape("a=(return)ok", false)?;
    ensure_rule_attempt_return_rule_shape("(once)a=(return)ok", true)
}

/// # Errors
///
/// Returns `TestFailure` if a failed step does not preserve the uncommitted
/// state as a terminal transition.
#[test]
fn execution_step_failure_is_terminal_transition() -> TestResult {
    let program = parse_program("a=(return)ok")?;
    let limits = DefaultInputRunPolicy::<1, DEFAULT_BYTE_BUDGET, 1>::new();
    let execution = program.steps(runtime_input(b"a", limits)?)?;

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
    let execution = program.steps(runtime_input(b"a", limits)?)?;

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
