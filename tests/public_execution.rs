pub mod support;

use rsaeb::execution::{
    AppliedExecution, ExecutionStepError, ExecutionTransition, ReturnedExecution, RunningExecution,
    StableExecution,
};
use rsaeb::limits::{DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, StepLimit};
use rsaeb::{RunLimits, RunOutcome, RunResult};
use support::{TestFailure, TestResult, ensure_eq, ensure_matches, parse_program, runtime_input};

/// Returns stable output bytes when they match `expected`.
///
/// # Errors
///
/// Returns `TestFailure` if the run result is not stable or stable bytes differ.
fn expect_stable_bytes(result: &RunResult, expected: &[u8]) -> TestResult {
    match result.outcome() {
        RunOutcome::Stable(output) if output.as_bytes() == expected => Ok(()),
        RunOutcome::Stable(_) => Err(TestFailure::message("stable output bytes differed")),
        RunOutcome::Return(_) => Err(TestFailure::message("expected stable outcome")),
    }
}

fn runtime_view_bytes(state: rsaeb::trace::RuntimeStateView<'_>) -> Vec<u8> {
    state.bytes().collect()
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
fn applied_signature(applied: &AppliedExecution<'_>) -> Result<StepSignature, TestFailure> {
    Ok(StepSignature::Applied {
        step: applied.step().get(),
        rule: applied.rule().canonical_source()?,
        state: runtime_view_bytes(applied.state()),
    })
}

fn stable_signature(stable: &StableExecution<'_>) -> StepSignature {
    StepSignature::Stable {
        steps: stable.steps().get(),
        state: runtime_view_bytes(stable.state()),
    }
}

/// Builds a comparable signature for a returned step.
///
/// # Errors
///
/// Returns `TestFailure` if canonical rule source or return output
/// materialization fails.
fn returned_signature(returned: &ReturnedExecution<'_>) -> Result<StepSignature, TestFailure> {
    Ok(StepSignature::Return {
        step: returned.step().get(),
        rule: returned.rule().canonical_source()?,
        output: returned.output().to_vec()?,
    })
}

/// Runs stepwise execution and collects comparable transition signatures.
///
/// # Errors
///
/// Returns `TestFailure` if a step fails or transition materialization fails.
fn finish_step_signatures(
    mut execution: RunningExecution<'_>,
) -> Result<Vec<StepSignature>, TestFailure> {
    let mut signatures = Vec::new();
    loop {
        match expect_step_transition(execution.step())? {
            ExecutionTransition::Applied(applied) => {
                signatures.push(applied_signature(&applied)?);
                execution = applied.into_running();
            }
            ExecutionTransition::Stable(stable) => {
                signatures.push(stable_signature(&stable));
                return Ok(signatures);
            }
            ExecutionTransition::Returned(returned) => {
                signatures.push(returned_signature(&returned)?);
                return Ok(signatures);
            }
        }
    }
}

/// Returns the expected successful step transition.
///
/// # Errors
///
/// Returns `TestFailure` if stepping fails.
fn expect_step_transition<'program>(
    result: Result<ExecutionTransition<'program>, ExecutionStepError<'program>>,
) -> Result<ExecutionTransition<'program>, TestFailure> {
    match result {
        Ok(transition) => Ok(transition),
        Err(error) => Err(TestFailure::from(error.into_error())),
    }
}

/// # Errors
///
/// Returns `TestFailure` if rewrite order, anchors, once rules, or runtime-only
/// byte preservation drift from the public contract.
#[test]
fn execution_rewrite_semantics_follow_public_contract() -> TestResult {
    let limits = RunLimits::new(
        StepLimit::new(10_000),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );

    let program = parse_program("aa=x\na=y")?;
    let result = program.run(&runtime_input(b"aaaa")?, limits)?;
    expect_stable_bytes(&result, b"xx")?;

    let program = parse_program("(start)a=x")?;
    let result = program.run(&runtime_input(b"aba")?, limits)?;
    expect_stable_bytes(&result, b"xba")?;

    let program = parse_program("(end)a=x")?;
    let result = program.run(&runtime_input(b"aba")?, limits)?;
    expect_stable_bytes(&result, b"abx")?;

    let program = parse_program("(once)a=b\na=c")?;
    let result = program.run(&runtime_input(b"aa")?, limits)?;
    expect_stable_bytes(&result, b"bc")?;

    let program = parse_program("ab=x")?;
    let result = program.run(&runtime_input(b"a=b")?, limits)?;
    expect_stable_bytes(&result, b"a=b")?;

    let program = parse_program("a= b")?;
    let result = program.run(&runtime_input(b"a bc")?, limits)?;
    expect_stable_bytes(&result, b"b bc")
}

/// # Errors
///
/// Returns `TestFailure` if stepwise execution diverges from full-run behavior
/// or fails to pause after each applied rule.
#[test]
fn execution_stepwise_transition_surface_is_rule_by_rule() -> TestResult {
    let limits = RunLimits::new(
        StepLimit::new(10),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let program = parse_program("a=b\nb=c")?;
    let input = runtime_input(b"a")?;
    let execution = program.start_execution(&input, limits)?;
    ensure_eq!(execution.completed_steps().get(), 0)?;

    let execution = match expect_step_transition(execution.step())? {
        ExecutionTransition::Applied(applied) => {
            ensure_eq!(applied.step().get(), 1)?;
            ensure_eq!(
                applied.rule().canonical_source()?.as_slice(),
                b"a=b".as_slice()
            )?;
            ensure_eq!(
                runtime_view_bytes(applied.state()).as_slice(),
                b"b".as_slice()
            )?;
            ensure_eq!(applied.state().byte_count().get(), 1)?;
            applied.into_running()
        }
        ExecutionTransition::Stable(_) | ExecutionTransition::Returned(_) => {
            return Err(TestFailure::message("expected first applied step"));
        }
    };

    let execution = match expect_step_transition(execution.step())? {
        ExecutionTransition::Applied(applied) => {
            ensure_eq!(applied.step().get(), 2)?;
            ensure_eq!(
                applied.rule().canonical_source()?.as_slice(),
                b"b=c".as_slice()
            )?;
            ensure_eq!(
                runtime_view_bytes(applied.state()).as_slice(),
                b"c".as_slice()
            )?;
            applied.into_running()
        }
        ExecutionTransition::Stable(_) | ExecutionTransition::Returned(_) => {
            return Err(TestFailure::message("expected second applied step"));
        }
    };

    match expect_step_transition(execution.step())? {
        ExecutionTransition::Stable(stable) => {
            ensure_eq!(stable.steps().get(), 2)?;
            ensure_eq!(
                runtime_view_bytes(stable.state()).as_slice(),
                b"c".as_slice()
            )?;
        }
        ExecutionTransition::Applied(_) | ExecutionTransition::Returned(_) => {
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
    let limits = RunLimits::new(
        StepLimit::new(10),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let program = parse_program("a=b")?;
    let input = runtime_input(b"a")?;
    let execution = program.start_execution(&input, limits)?;

    ensure_eq!(
        runtime_view_bytes(execution.state()).as_slice(),
        b"a".as_slice(),
    )?;

    let execution = match expect_step_transition(execution.step())? {
        ExecutionTransition::Applied(applied) => {
            ensure_eq!(
                runtime_view_bytes(applied.state()).as_slice(),
                b"b".as_slice()
            )?;
            applied.into_running()
        }
        ExecutionTransition::Stable(_) | ExecutionTransition::Returned(_) => {
            return Err(TestFailure::message("expected applied step"));
        }
    };

    ensure_eq!(
        runtime_view_bytes(execution.state()).as_slice(),
        b"b".as_slice(),
    )
}

/// # Errors
///
/// Returns `TestFailure` if repeated stepwise executions with the same runtime
/// input diverge.
#[test]
fn execution_reuses_runtime_input_without_session_leakage() -> TestResult {
    let limits = RunLimits::new(
        StepLimit::new(10),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let program = parse_program("(once)a=b\na=c")?;
    let input = runtime_input(b"aa")?;

    let first = program.start_execution(&input, limits)?;
    let second = program.start_execution(&input, limits)?;

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
        finish_step_signatures(program.start_execution(&input, limits)?)?,
    )?;
    ensure_matches(
        input.byte_count().get() == 2,
        "expected reusable input size",
    )
}
