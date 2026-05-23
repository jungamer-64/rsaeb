//! Public stepwise execution contract tests.

#[path = "support/runtime.rs"]
mod runtime_support;
mod support;

use rsaeb::error::LimitError;
use rsaeb::execution::{
    AppliedStep, FailedRun, OwnedRunSession, OwnedStepTransition, ReturnedRun, RunSession,
    StableRun, StepTransition,
};
use rsaeb::input::RunSeed;
use rsaeb::limits::{
    DEFAULT_MAX_INPUT_LEN, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, ReturnByteLimit,
    StepLimit,
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
        rule: Vec<u8>,
        state: Vec<u8>,
    },
    Stable {
        steps: usize,
        state: Vec<u8>,
    },
    Return {
        step: usize,
        rule: Vec<u8>,
        output: Vec<u8>,
    },
}

/// Builds a comparable signature for an applied step.
///
/// # Errors
///
/// Returns `TestFailure` if canonical rule source materialization fails.
fn applied_signature(applied: &AppliedStep<'_>) -> Result<StepSignature, TestFailure> {
    Ok(StepSignature::Applied {
        step: applied.step().get(),
        rule: applied.rule()?.canonical_source()?.into_raw_bytes(),
        state: runtime_view_bytes(applied.state())?,
    })
}

/// Builds a comparable signature for a stable terminal state.
///
/// # Errors
///
/// Returns `TestFailure` if stable-state materialization fails.
fn stable_signature(stable: &StableRun) -> Result<StepSignature, TestFailure> {
    Ok(StepSignature::Stable {
        steps: stable.steps().get(),
        state: runtime_view_bytes(stable.state())?,
    })
}

/// Builds a comparable signature for a returned step.
///
/// # Errors
///
/// Returns `TestFailure` if canonical rule source or return output
/// materialization fails.
fn returned_signature(returned: &ReturnedRun<'_>) -> Result<StepSignature, TestFailure> {
    Ok(StepSignature::Return {
        step: returned.step().get(),
        rule: returned.rule()?.canonical_source()?.into_raw_bytes(),
        output: returned.output()?.materialize()?.into_raw_bytes(),
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

/// Runs owned stepwise execution and collects comparable transition signatures.
///
/// # Errors
///
/// Returns `TestFailure` if a step fails or transition materialization fails.
fn finish_owned_step_signatures(
    mut execution: OwnedRunSession,
) -> Result<Vec<StepSignature>, TestFailure> {
    let mut signatures = Vec::new();
    loop {
        match execution.step() {
            OwnedStepTransition::Applied(applied) => {
                signatures.push(StepSignature::Applied {
                    step: applied.step().get(),
                    rule: applied.rule()?.canonical_source()?.into_raw_bytes(),
                    state: runtime_view_bytes(applied.state())?,
                });
                execution = applied.into_session();
            }
            OwnedStepTransition::Stable(stable) => {
                signatures.push(StepSignature::Stable {
                    steps: stable.steps().get(),
                    state: runtime_view_bytes(stable.state())?,
                });
                return Ok(signatures);
            }
            OwnedStepTransition::Returned(returned) => {
                signatures.push(StepSignature::Return {
                    step: returned.step().get(),
                    rule: returned.rule()?.canonical_source()?.into_raw_bytes(),
                    output: returned.output()?.materialize()?.into_raw_bytes(),
                });
                return Ok(signatures);
            }
            OwnedStepTransition::Failed(failed) => {
                return Err(TestFailure::from(failed.into_error()));
            }
        }
    }
}

/// Returns the expected successful step transition.
///
/// # Errors
///
/// Returns `TestFailure` if stepping fails.
fn expect_step_transition(result: StepTransition<'_>) -> Result<StepTransition<'_>, TestFailure> {
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
fn expect_failed_transition(result: StepTransition<'_>) -> Result<FailedRun<'_>, TestFailure> {
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
            ensure_eq!(
                applied.rule()?.canonical_source()?.as_slice(),
                b"a=b".as_slice()
            )?;
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
            ensure_eq!(
                applied.rule()?.canonical_source()?.as_slice(),
                b"b=c".as_slice()
            )?;
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
/// Returns `TestFailure` if repeated stepwise executions from separately
/// validated equivalent input diverge.
#[test]
fn execution_consumes_runtime_input_without_session_leakage() -> TestResult {
    let limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(10),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let program = parse_program("(once)a=b\na=c")?;

    let first = program.start_run(runtime_input(b"aa", limits)?)?;
    let second = program.start_run(runtime_input(b"aa", limits)?)?;

    ensure_eq!(
        finish_step_signatures(first)?,
        [
            StepSignature::Applied {
                step: 1,
                rule: b"(once)a=b".to_vec(),
                state: b"ba".to_vec(),
            },
            StepSignature::Applied {
                step: 2,
                rule: b"a=c".to_vec(),
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
        finish_step_signatures(program.start_run(runtime_input(b"aa", limits)?)?)?,
    )
}

/// # Errors
///
/// Returns `TestFailure` if borrowed and owned run sessions diverge for the
/// same source, input, and limits.
#[test]
fn execution_borrowed_and_owned_sessions_share_step_contract() -> TestResult {
    let source = "a=b\nb=(return)ok";
    let limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(10),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );

    let borrowed =
        finish_step_signatures(parse_program(source)?.start_run(runtime_input(b"a", limits)?)?)?;
    let owned = finish_owned_step_signatures(
        parse_program(source)?.into_run(runtime_input(b"a", limits)?)?,
    )?;

    ensure_eq!(borrowed, owned)
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
            }) if *limit == ReturnByteLimit::new(1) && attempted_len.get() == 2
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
