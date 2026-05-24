//! Public stepwise execution contract tests.

#[path = "support/runtime.rs"]
mod runtime_support;
mod support;

use rsaeb::error::LimitError;
use rsaeb::execution::{
    AppliedStep, FailedRun, OwnedRuleAttemptTransition, OwnedStepTransition, ReturnedRun,
    RuleAttemptTransition, RuleMissReason, RunSession, StableRun, StepTransition,
};
use rsaeb::input::RunSeed;
use rsaeb::limits::{
    DEFAULT_MAX_INPUT_LEN, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, ReturnByteLimit,
    RuleAttemptLimit, StepLimit,
};
use rsaeb::program::{RunOutcome, RunResult};
use runtime_support::TestRunPolicy;
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

/// Builds a comparable signature for an applied step.
///
/// # Errors
///
/// Returns `TestFailure` if state materialization fails.
fn applied_signature(applied: &AppliedStep<'_>) -> Result<StepSignature, TestFailure> {
    Ok(StepSignature::Applied {
        step: applied.step().get(),
        rule_position: applied.rule_position().number().get(),
        state: runtime_view_bytes(applied.state())?,
    })
}

/// Builds a comparable signature for a stable terminal state.
///
/// # Errors
///
/// Returns `TestFailure` if stable-state materialization fails.
fn stable_signature(stable: &StableRun<'_>) -> Result<StepSignature, TestFailure> {
    Ok(StepSignature::Stable {
        steps: stable.steps().get(),
        state: runtime_view_bytes(stable.state())?,
    })
}

/// Builds a comparable signature for a returned step.
///
/// # Errors
///
/// Returns `TestFailure` if output bytes differ from the expected signature.
fn returned_signature(returned: &ReturnedRun<'_>) -> Result<StepSignature, TestFailure> {
    Ok(StepSignature::Return {
        step: returned.step().get(),
        rule_position: returned.rule_position().number().get(),
        output: returned.output().as_slice().to_vec(),
    })
}

/// Runs stepwise execution and collects comparable transition signatures.
///
/// # Errors
///
/// Returns `TestFailure` if a step fails or transition materialization fails.
fn finish_step_signatures(
    mut execution: RunSession<'_>,
) -> Result<Vec<StepSignature>, TestFailure> {
    let mut signatures = Vec::new();
    loop {
        match expect_step_transition(execution.step())? {
            StepTransition::Applied(applied) => {
                signatures.push(applied_signature(&applied)?);
                execution = applied.into_session();
            }
            StepTransition::Stable(stable) => {
                signatures.push(stable_signature(&stable)?);
                return Ok(signatures);
            }
            StepTransition::Returned(returned) => {
                signatures.push(returned_signature(&returned)?);
                return Ok(signatures);
            }
            StepTransition::Failed(failed) => return Err(TestFailure::from(failed.into_error())),
        }
    }
}

/// Returns the expected successful step transition.
///
/// # Errors
///
/// Returns `TestFailure` if stepping fails.
fn expect_step_transition<'program>(
    result: StepTransition<'program>,
) -> Result<StepTransition<'program>, TestFailure> {
    match result {
        StepTransition::Failed(failed) => Err(TestFailure::from(failed.into_error())),
        transition => Ok(transition),
    }
}

/// Returns the expected failed step transition.
///
/// # Errors
///
/// Returns `TestFailure` if stepping does not fail.
fn expect_failed_transition<'program>(
    result: StepTransition<'program>,
) -> Result<FailedRun<'program>, TestFailure> {
    match result {
        StepTransition::Failed(failed) => Ok(failed),
        StepTransition::Applied(_) | StepTransition::Stable(_) | StepTransition::Returned(_) => {
            Err(TestFailure::message("expected failed step"))
        }
    }
}

/// Validates test bytes as runtime input.
///
/// # Errors
///
/// Returns `RuntimeInputError` if the bytes are not valid runtime input.
fn runtime_input(bytes: &[u8], limits: TestRunPolicy) -> Result<RunSeed, TestFailure> {
    runtime_support::run_seed(bytes, limits)
}

/// Returns the expected successful rule-attempt transition.
///
/// # Errors
///
/// Returns `TestFailure` if stepping fails.
fn expect_rule_attempt_transition<'program>(
    result: RuleAttemptTransition<'program>,
) -> Result<RuleAttemptTransition<'program>, TestFailure> {
    match result {
        RuleAttemptTransition::Failed(failed) => Err(TestFailure::from(failed.into_error())),
        transition => Ok(transition),
    }
}

/// Returns the expected failed rule-attempt transition.
///
/// # Errors
///
/// Returns `TestFailure` if stepping does not fail.
fn expect_failed_rule_attempt<'program>(
    result: RuleAttemptTransition<'program>,
) -> Result<rsaeb::execution::RuleAttemptFailedRun<'program>, TestFailure> {
    match result {
        RuleAttemptTransition::Failed(failed) => Ok(failed),
        RuleAttemptTransition::Missed(_)
        | RuleAttemptTransition::Applied(_)
        | RuleAttemptTransition::Stable(_)
        | RuleAttemptTransition::Returned(_) => {
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
    let limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(10_000),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );

    let program = parse_program("aa=x\na=y")?;
    let result = program.run(runtime_input(b"aaaa", limits)?)?;
    expect_stable_bytes(&result, b"xx")?;

    let program = parse_program("(start)a=x")?;
    let result = program.run(runtime_input(b"aba", limits)?)?;
    expect_stable_bytes(&result, b"xba")?;

    let program = parse_program("(end)a=x")?;
    let result = program.run(runtime_input(b"aba", limits)?)?;
    expect_stable_bytes(&result, b"abx")?;

    let program = parse_program("(once)a=b\na=c")?;
    let result = program.run(runtime_input(b"aa", limits)?)?;
    expect_stable_bytes(&result, b"bc")?;

    let program = parse_program("ab=x")?;
    let result = program.run(runtime_input(b"a=b", limits)?)?;
    expect_stable_bytes(&result, b"a=b")?;

    let program = parse_program("a= b")?;
    let result = program.run(runtime_input(b"a bc", limits)?)?;
    expect_stable_bytes(&result, b"b bc")
}

/// # Errors
///
/// Returns `TestFailure` if stepwise execution diverges from full-run behavior
/// or fails to pause after each applied rule.
#[test]
fn execution_stepwise_transition_surface_is_rule_by_rule() -> TestResult {
    let limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(10),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let program = parse_program("a=b\nb=c")?;
    let input = runtime_input(b"a", limits)?;
    let execution = program.start_run(input)?;
    ensure_eq!(execution.completed_steps().get(), 0)?;

    let execution = match expect_step_transition(execution.step())? {
        StepTransition::Applied(applied) => {
            ensure_eq!(applied.step().get(), 1)?;
            ensure_eq!(applied.rule_position().number().get(), 1)?;
            ensure_eq!(
                runtime_view_bytes(applied.state())?.as_slice(),
                b"b".as_slice()
            )?;
            ensure_eq!(applied.state().byte_count().get(), 1)?;
            applied.into_session()
        }
        StepTransition::Stable(_) | StepTransition::Returned(_) | StepTransition::Failed(_) => {
            return Err(TestFailure::message("expected first applied step"));
        }
    };

    let execution = match expect_step_transition(execution.step())? {
        StepTransition::Applied(applied) => {
            ensure_eq!(applied.step().get(), 2)?;
            ensure_eq!(applied.rule_position().number().get(), 2)?;
            ensure_eq!(
                runtime_view_bytes(applied.state())?.as_slice(),
                b"c".as_slice()
            )?;
            applied.into_session()
        }
        StepTransition::Stable(_) | StepTransition::Returned(_) | StepTransition::Failed(_) => {
            return Err(TestFailure::message("expected second applied step"));
        }
    };

    match expect_step_transition(execution.step())? {
        StepTransition::Stable(stable) => {
            ensure_eq!(stable.steps().get(), 2)?;
            ensure_eq!(
                runtime_view_bytes(stable.state())?.as_slice(),
                b"c".as_slice()
            )?;
        }
        StepTransition::Applied(_) | StepTransition::Returned(_) | StepTransition::Failed(_) => {
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
    let limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(10),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let program = parse_program("z=x\na=b\nb=c")?;
    let input = runtime_input(b"a", limits)?;
    let execution = program.start_rule_attempt_run(input, RuleAttemptLimit::new(20))?;

    let execution = match expect_rule_attempt_transition(execution.step())? {
        RuleAttemptTransition::Missed(missed) => {
            ensure_eq!(missed.attempt().get(), 1)?;
            ensure_eq!(missed.rule_position().number().get(), 1)?;
            ensure_eq!(missed.reason(), RuleMissReason::StateMismatch)?;
            ensure_eq!(
                runtime_view_bytes(missed.state())?.as_slice(),
                b"a".as_slice(),
            )?;
            missed.into_session()
        }
        RuleAttemptTransition::Applied(_)
        | RuleAttemptTransition::Stable(_)
        | RuleAttemptTransition::Returned(_)
        | RuleAttemptTransition::Failed(_) => {
            return Err(TestFailure::message("expected first missed rule attempt"));
        }
    };

    let execution = match expect_rule_attempt_transition(execution.step())? {
        RuleAttemptTransition::Applied(applied) => {
            ensure_eq!(applied.attempt().get(), 2)?;
            ensure_eq!(applied.step().get(), 1)?;
            ensure_eq!(applied.rule_position().number().get(), 2)?;
            ensure_eq!(
                runtime_view_bytes(applied.state())?.as_slice(),
                b"b".as_slice(),
            )?;
            applied.into_session()
        }
        RuleAttemptTransition::Missed(_)
        | RuleAttemptTransition::Stable(_)
        | RuleAttemptTransition::Returned(_)
        | RuleAttemptTransition::Failed(_) => {
            return Err(TestFailure::message("expected applied rule attempt"));
        }
    };

    let execution = match expect_rule_attempt_transition(execution.step())? {
        RuleAttemptTransition::Missed(missed) => {
            ensure_eq!(missed.attempt().get(), 3)?;
            ensure_eq!(missed.rule_position().number().get(), 1)?;
            ensure_eq!(missed.reason(), RuleMissReason::StateMismatch)?;
            missed.into_session()
        }
        RuleAttemptTransition::Applied(_)
        | RuleAttemptTransition::Stable(_)
        | RuleAttemptTransition::Returned(_)
        | RuleAttemptTransition::Failed(_) => {
            return Err(TestFailure::message(
                "expected cursor reset to first rule after apply",
            ));
        }
    };

    let execution = match expect_rule_attempt_transition(execution.step())? {
        RuleAttemptTransition::Missed(missed) => {
            ensure_eq!(missed.attempt().get(), 4)?;
            ensure_eq!(missed.rule_position().number().get(), 2)?;
            ensure_eq!(missed.reason(), RuleMissReason::StateMismatch)?;
            missed.into_session()
        }
        RuleAttemptTransition::Applied(_)
        | RuleAttemptTransition::Stable(_)
        | RuleAttemptTransition::Returned(_)
        | RuleAttemptTransition::Failed(_) => {
            return Err(TestFailure::message("expected second miss after reset"));
        }
    };

    match expect_rule_attempt_transition(execution.step())? {
        RuleAttemptTransition::Applied(applied) => {
            ensure_eq!(applied.attempt().get(), 5)?;
            ensure_eq!(applied.step().get(), 2)?;
            ensure_eq!(applied.rule_position().number().get(), 3)?;
            ensure_eq!(
                runtime_view_bytes(applied.state())?.as_slice(),
                b"c".as_slice(),
            )?;
        }
        RuleAttemptTransition::Missed(_)
        | RuleAttemptTransition::Stable(_)
        | RuleAttemptTransition::Returned(_)
        | RuleAttemptTransition::Failed(_) => {
            return Err(TestFailure::message("expected later applied rule attempt"));
        }
    }
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if the final non-matching rule attempt requires an
/// extra call before reporting stable completion.
#[test]
fn execution_rule_attempt_final_miss_is_stable_immediately() -> TestResult {
    let limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(10),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let program = parse_program("a=b")?;
    let input = runtime_input(b"z", limits)?;
    let execution = program.start_rule_attempt_run(input, RuleAttemptLimit::new(10))?;

    match expect_rule_attempt_transition(execution.step())? {
        RuleAttemptTransition::Stable(stable) => {
            ensure_eq!(stable.attempts().get(), 1)?;
            ensure_eq!(stable.steps().get(), 0)?;
            let terminal_miss = stable
                .terminal_miss()
                .ok_or(TestFailure::message("expected terminal miss"))?;
            ensure_eq!(terminal_miss.rule_position().number().get(), 1)?;
            ensure_eq!(terminal_miss.reason(), RuleMissReason::StateMismatch)?;
            ensure_eq!(
                runtime_view_bytes(stable.state())?.as_slice(),
                b"z".as_slice(),
            )?;
        }
        RuleAttemptTransition::Missed(_)
        | RuleAttemptTransition::Applied(_)
        | RuleAttemptTransition::Returned(_)
        | RuleAttemptTransition::Failed(_) => {
            return Err(TestFailure::message("expected immediate stable terminal"));
        }
    }
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if consumed `(once)` rules are hidden instead of
/// reported as typed rule-attempt misses.
#[test]
fn execution_rule_attempt_reports_consumed_once_rule_before_later_match() -> TestResult {
    let limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(10),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let program = parse_program("(once)a=b\nb=c")?;
    let input = runtime_input(b"ab", limits)?;
    let execution = program.start_rule_attempt_run(input, RuleAttemptLimit::new(10))?;

    let execution = match expect_rule_attempt_transition(execution.step())? {
        RuleAttemptTransition::Applied(applied) => {
            ensure_eq!(applied.attempt().get(), 1)?;
            ensure_eq!(applied.step().get(), 1)?;
            ensure_eq!(applied.rule_position().number().get(), 1)?;
            applied.into_session()
        }
        RuleAttemptTransition::Missed(_)
        | RuleAttemptTransition::Stable(_)
        | RuleAttemptTransition::Returned(_)
        | RuleAttemptTransition::Failed(_) => {
            return Err(TestFailure::message("expected once rule to apply first"));
        }
    };

    let execution = match expect_rule_attempt_transition(execution.step())? {
        RuleAttemptTransition::Missed(missed) => {
            ensure_eq!(missed.attempt().get(), 2)?;
            ensure_eq!(missed.rule_position().number().get(), 1)?;
            ensure_eq!(missed.reason(), RuleMissReason::OnceConsumed)?;
            missed.into_session()
        }
        RuleAttemptTransition::Applied(_)
        | RuleAttemptTransition::Stable(_)
        | RuleAttemptTransition::Returned(_)
        | RuleAttemptTransition::Failed(_) => {
            return Err(TestFailure::message("expected consumed once miss"));
        }
    };

    match expect_rule_attempt_transition(execution.step())? {
        RuleAttemptTransition::Applied(applied) => {
            ensure_eq!(applied.attempt().get(), 3)?;
            ensure_eq!(applied.step().get(), 2)?;
            ensure_eq!(applied.rule_position().number().get(), 2)?;
        }
        RuleAttemptTransition::Missed(_)
        | RuleAttemptTransition::Stable(_)
        | RuleAttemptTransition::Returned(_)
        | RuleAttemptTransition::Failed(_) => {
            return Err(TestFailure::message(
                "expected later rule to match after once miss",
            ));
        }
    }
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if rule-attempt budget is folded into rewrite step
/// budget or fails to report typed details.
#[test]
fn execution_rule_attempt_limit_is_independent_from_step_limit() -> TestResult {
    let limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(0),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let program = parse_program("x=y\na=b")?;
    let input = runtime_input(b"a", limits)?;
    let execution = program.start_rule_attempt_run(input, RuleAttemptLimit::new(1))?;

    let execution = match expect_rule_attempt_transition(execution.step())? {
        RuleAttemptTransition::Missed(missed) => {
            ensure_eq!(missed.attempt().get(), 1)?;
            ensure_eq!(missed.rule_position().number().get(), 1)?;
            ensure_eq!(missed.reason(), RuleMissReason::StateMismatch)?;
            missed.into_session()
        }
        RuleAttemptTransition::Applied(_)
        | RuleAttemptTransition::Stable(_)
        | RuleAttemptTransition::Returned(_)
        | RuleAttemptTransition::Failed(_) => {
            return Err(TestFailure::message(
                "expected miss despite zero rewrite step limit",
            ));
        }
    };

    let failed = expect_failed_rule_attempt(execution.step())?;
    ensure_eq!(failed.completed_attempts().get(), 1)?;
    ensure_eq!(failed.completed_steps().get(), 0)?;
    ensure_matches(
        matches!(
            failed.into_error(),
            rsaeb::error::RunError::Limit(LimitError::RuleAttempt {
                max_attempts,
                completed_attempts,
                state_len,
            }) if max_attempts == RuleAttemptLimit::new(1)
                && completed_attempts.get() == 1
                && state_len.get() == 1
        ),
        "expected rule-attempt limit details",
    )
}

/// # Errors
///
/// Returns `TestFailure` if execution state views do not expose initial and
/// current state bytes correctly.
#[test]
fn execution_state_view_exposes_initial_and_current_state() -> TestResult {
    let limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(10),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let program = parse_program("a=b")?;
    let input = runtime_input(b"a", limits)?;
    let execution = program.start_run(input)?;

    ensure_eq!(
        runtime_view_bytes(execution.state())?.as_slice(),
        b"a".as_slice(),
    )?;

    let execution = match expect_step_transition(execution.step())? {
        StepTransition::Applied(applied) => {
            ensure_eq!(
                runtime_view_bytes(applied.state())?.as_slice(),
                b"b".as_slice()
            )?;
            applied.into_session()
        }
        StepTransition::Stable(_) | StepTransition::Returned(_) | StepTransition::Failed(_) => {
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
    let limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(10),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let source = "(once)a=b\na=c";
    let program = parse_program(source)?;
    let first = program.start_run(runtime_input(b"aa", limits)?)?;
    let second = program.start_run(runtime_input(b"aa", limits)?)?;
    let third = program.start_run(runtime_input(b"aa", limits)?)?;

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
    let limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(10),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );

    let borrowed = parse_program(source)?.run(runtime_input(b"a", limits)?)?;
    let owned = parse_program(source)?
        .into_run(runtime_input(b"a", limits)?)?
        .finish()?;

    ensure_eq!(borrowed, owned)
}

/// # Errors
///
/// Returns `TestFailure` if owned stepwise terminal states cannot return the
/// parsed program to the caller.
#[test]
fn execution_owned_terminals_can_return_program() -> TestResult {
    let limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(10),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );

    let stable_session = parse_program("a=b")?.into_run(runtime_input(b"a", limits)?)?;
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
        .into_run(runtime_input(b"a", limits)?)?
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

    let failed_limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(1),
        DEFAULT_MAX_STATE_LEN,
        ReturnByteLimit::new(1),
    );
    let (error, failed_session) = match parse_program("a=(return)ok")?
        .into_run(runtime_input(b"a", failed_limits)?)?
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
            rsaeb::error::RunError::Limit(LimitError::Return { .. })
        ),
        "expected owned return limit failure",
    )?;
    ensure_eq!(failed_session.into_program().rule_count().get(), 1)
}

/// # Errors
///
/// Returns `TestFailure` if owned rule-attempt terminal states cannot return
/// the parsed program to the caller.
#[test]
fn execution_owned_rule_attempt_terminals_can_return_program() -> TestResult {
    let limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(10),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );

    let stable_program = match parse_program("a=b")?
        .into_rule_attempt_run(runtime_input(b"z", limits)?, RuleAttemptLimit::new(10))?
        .step()
    {
        OwnedRuleAttemptTransition::Stable(stable) => stable.into_program(),
        OwnedRuleAttemptTransition::Missed(_)
        | OwnedRuleAttemptTransition::Applied(_)
        | OwnedRuleAttemptTransition::Returned(_)
        | OwnedRuleAttemptTransition::Failed(_) => {
            return Err(TestFailure::message("expected owned rule-attempt stable"));
        }
    };
    ensure_eq!(stable_program.rule_count().get(), 1)?;

    let returned_program = match parse_program("a=(return)ok")?
        .into_rule_attempt_run(runtime_input(b"a", limits)?, RuleAttemptLimit::new(10))?
        .step()
    {
        OwnedRuleAttemptTransition::Returned(returned) => returned.into_program(),
        OwnedRuleAttemptTransition::Missed(_)
        | OwnedRuleAttemptTransition::Applied(_)
        | OwnedRuleAttemptTransition::Stable(_)
        | OwnedRuleAttemptTransition::Failed(_) => {
            return Err(TestFailure::message("expected owned rule-attempt return"));
        }
    };
    ensure_eq!(returned_program.rule_count().get(), 1)
}

/// # Errors
///
/// Returns `TestFailure` if a failed step does not preserve the uncommitted
/// state as a terminal transition.
#[test]
fn execution_step_failure_is_terminal_transition() -> TestResult {
    let program = parse_program("a=(return)ok")?;
    let limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(1),
        DEFAULT_MAX_STATE_LEN,
        ReturnByteLimit::new(1),
    );
    let execution = program.start_run(runtime_input(b"a", limits)?)?;

    let failed = expect_failed_transition(execution.step())?;
    ensure_eq!(failed.completed_steps().get(), 0)?;
    ensure_eq!(
        runtime_view_bytes(failed.state())?.as_slice(),
        b"a".as_slice(),
    )?;
    ensure_matches(
        matches!(
            failed.error(),
            rsaeb::error::RunError::Limit(LimitError::Return {
                limit,
                attempted_len,
            }) if limit == &ReturnByteLimit::new(1) && attempted_len.get() == 2
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
    let limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(1),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let execution = program.start_run(runtime_input(b"a", limits)?)?;

    let running = match expect_step_transition(execution.step())? {
        StepTransition::Applied(applied) => applied.into_session(),
        StepTransition::Stable(_) | StepTransition::Returned(_) | StepTransition::Failed(_) => {
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
            rsaeb::error::RunError::Limit(LimitError::Step {
                completed_steps,
                ..
            }) if completed_steps.get() == 1
        ),
        "expected completed-step limit failure",
    )
}
