use super::input::{InitialStateBytes, RuntimeInput};
use super::state::State;
use crate::RunLimits;
use crate::bytes::{CompactByte, Payload};
use crate::error::{LimitError, PayloadKind, RunError, RuntimeInputError, StateLimitContext};
use crate::limits::{
    DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, ReturnByteLimit, ReturnOutputByteCount,
    RuntimeInputByteCount, RuntimeInputByteLimit, RuntimeStateByteCount, RuntimeStateByteLimit,
    StepCount, StepLimit,
};
use crate::test_support::{
    TestFailure, TestResult, ensure_eq, ensure_matches, parse_program, runtime_input,
    source_column, source_line_number,
};
use crate::trace::RuntimeStateView;
use std::vec::Vec;

use super::session::{RuntimeSession, RuntimeStep, RuntimeStepError};

fn runtime_view_bytes(view: RuntimeStateView<'_>) -> Vec<u8> {
    view.bytes().collect()
}

/// Returns the materialized runtime byte at `index`.
///
/// # Errors
///
/// Returns `TestFailure` if the state has no byte at `index`.
fn expect_runtime_byte(state: &State, index: usize) -> Result<u8, TestFailure> {
    state
        .view()
        .bytes()
        .nth(index)
        .ok_or(TestFailure::message("expected runtime byte"))
}

/// Returns the program payload byte at `index`.
///
/// # Errors
///
/// Returns `TestFailure` if the payload has no byte at `index`.
fn expect_payload_byte(payload: &Payload, index: usize) -> Result<u8, TestFailure> {
    payload
        .bytes()
        .nth(index)
        .ok_or(TestFailure::message("expected payload byte"))
}

/// Returns the expected step limit error.
///
/// # Errors
///
/// Returns `TestFailure` if `error` is not a step limit error.
fn expect_step_limit(error: RunError) -> Result<LimitError, TestFailure> {
    match error {
        RunError::Limit(error @ LimitError::Step { .. }) => Ok(error),
        RunError::Allocation(_) | RunError::StateSize(_) | RunError::Limit(_) => {
            Err(TestFailure::message("expected step limit error"))
        }
    }
}

/// Returns the expected step error.
///
/// # Errors
///
/// Returns `TestFailure` if stepping succeeds.
fn expect_step_error<'program>(
    result: Result<RuntimeStep<'program>, RuntimeStepError<'program>>,
) -> Result<RuntimeStepError<'program>, TestFailure> {
    match result {
        Ok(_) => Err(TestFailure::message("expected step error")),
        Err(error) => Ok(error),
    }
}

/// Returns the expected successful step transition.
///
/// # Errors
///
/// Returns `TestFailure` if stepping fails.
fn expect_step_transition<'program>(
    result: Result<RuntimeStep<'program>, RuntimeStepError<'program>>,
) -> Result<RuntimeStep<'program>, TestFailure> {
    match result {
        Ok(transition) => Ok(transition),
        Err(error) => Err(TestFailure::from(error.into_error())),
    }
}

/// # Errors
///
/// Returns `TestFailure` if a failed once-rule commit attempt consumes rule
/// availability.
#[test]
fn once_rule_failure_does_not_consume_before_step_commit() -> TestResult {
    let program = parse_program("(once)a=(return)ok")?;
    let input = runtime_input(b"a")?;
    let runtime = RuntimeSession::new(
        &program,
        &input,
        RunLimits::new(
            StepLimit::new(1),
            DEFAULT_MAX_STATE_LEN,
            ReturnByteLimit::new(1),
        ),
    )?;
    let error = expect_step_error(runtime.step())?;
    ensure_eq!(
        error.error(),
        &RunError::Limit(LimitError::Return {
            limit: ReturnByteLimit::new(1),
            attempted_len: ReturnOutputByteCount::new(2),
        }),
    )?;

    let (_, runtime) = error.into_parts();
    let repeated_error = expect_step_error(runtime.step())?;
    ensure_eq!(
        repeated_error.into_error(),
        RunError::Limit(LimitError::Return {
            limit: ReturnByteLimit::new(1),
            attempted_len: ReturnOutputByteCount::new(2),
        }),
    )
}

/// # Errors
///
/// Returns `TestFailure` if a step-limit failure commits state or loses the
/// running execution.
#[test]
fn execution_step_limit_failure_preserves_uncommitted_state() -> TestResult {
    let program = parse_program("a=b")?;
    let no_match_input = runtime_input(b"x")?;
    let no_match = RuntimeSession::new(
        &program,
        &no_match_input,
        RunLimits::new(
            StepLimit::new(0),
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        ),
    )?;
    match expect_step_transition(no_match.step())? {
        RuntimeStep::Stable(stable) => {
            ensure_eq!(stable.steps().get(), 0)?;
            ensure_eq!(
                runtime_view_bytes(stable.state()).as_slice(),
                b"x".as_slice()
            )?;
        }
        RuntimeStep::Applied(_) | RuntimeStep::Returned(_) => {
            return Err(TestFailure::message("expected stable completion"));
        }
    }

    let would_match_input = runtime_input(b"a")?;
    let would_match = RuntimeSession::new(
        &program,
        &would_match_input,
        RunLimits::new(
            StepLimit::new(0),
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        ),
    )?;
    let error = expect_step_error(would_match.step())?;
    ensure_eq!(
        expect_step_limit(error.into_error())?,
        LimitError::Step {
            max_steps: StepLimit::new(0),
            completed_steps: StepCount::ZERO,
            state_len: RuntimeStateByteCount::new(1),
        },
    )?;
    let would_match = RuntimeSession::new(
        &program,
        &would_match_input,
        RunLimits::new(
            StepLimit::new(0),
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        ),
    )?;
    let error = expect_step_error(would_match.step())?;
    ensure_eq!(error.session().completed_steps(), StepCount::ZERO)?;
    ensure_eq!(
        runtime_view_bytes(error.session().state()).as_slice(),
        b"a".as_slice(),
    )?;

    let (_, runtime) = error.into_parts();
    let repeated_error = expect_step_limit(expect_step_error(runtime.step())?.into_error())?;
    ensure_eq!(
        repeated_error,
        LimitError::Step {
            max_steps: StepLimit::new(0),
            completed_steps: StepCount::ZERO,
            state_len: RuntimeStateByteCount::new(1),
        },
    )
}

/// # Errors
///
/// Returns `TestFailure` if state or return-size limit failures commit state.
#[test]
fn execution_size_limit_failures_preserve_uncommitted_state() -> TestResult {
    let state_limits = RunLimits::new(
        StepLimit::new(1),
        RuntimeStateByteLimit::new(2),
        ReturnByteLimit::new(10),
    );
    let state_program = parse_program("=a")?;
    let state_input = runtime_input(b"aa")?;
    let state_limited = RuntimeSession::new(&state_program, &state_input, state_limits)?;
    let state_error = expect_step_error(state_limited.step())?;
    ensure_eq!(
        state_error.error(),
        &RunError::Limit(LimitError::State {
            context: StateLimitContext::Rewrite,
            limit: RuntimeStateByteLimit::new(2),
            attempted_len: RuntimeStateByteCount::new(3),
        }),
    )?;
    ensure_eq!(state_error.session().completed_steps(), StepCount::ZERO)?;
    ensure_eq!(
        runtime_view_bytes(state_error.session().state()).as_slice(),
        b"aa".as_slice(),
    )?;
    let (_, runtime) = state_error.into_parts();
    let state_error = expect_step_error(runtime.step())?;
    ensure_eq!(
        state_error.into_error(),
        RunError::Limit(LimitError::State {
            context: StateLimitContext::Rewrite,
            limit: RuntimeStateByteLimit::new(2),
            attempted_len: RuntimeStateByteCount::new(3),
        }),
    )?;

    let return_limits = RunLimits::new(
        StepLimit::new(1),
        RuntimeStateByteLimit::new(10),
        ReturnByteLimit::new(1),
    );
    let return_program = parse_program("a=(return)ok")?;
    let return_input = runtime_input(b"a")?;
    let return_limited = RuntimeSession::new(&return_program, &return_input, return_limits)?;
    let return_error = expect_step_error(return_limited.step())?;
    ensure_eq!(
        return_error.error(),
        &RunError::Limit(LimitError::Return {
            limit: ReturnByteLimit::new(1),
            attempted_len: ReturnOutputByteCount::new(2),
        }),
    )?;
    ensure_eq!(return_error.session().completed_steps(), StepCount::ZERO)?;
    ensure_eq!(
        runtime_view_bytes(return_error.session().state()).as_slice(),
        b"a".as_slice(),
    )?;
    let (_, runtime) = return_error.into_parts();
    let return_error = expect_step_error(runtime.step())?;
    ensure_eq!(
        return_error.into_error(),
        RunError::Limit(LimitError::Return {
            limit: ReturnByteLimit::new(1),
            attempted_len: ReturnOutputByteCount::new(2),
        }),
    )
}

/// # Errors
///
/// Returns `TestFailure` if runtime input errors lose structured boundary
/// information.
#[test]
fn runtime_input_error_is_structured_at_the_runtime_boundary() -> TestResult {
    let Err(error) = RuntimeInput::validate(b"abc", RuntimeInputByteLimit::new(2)) else {
        return Err(TestFailure::message("expected input limit error"));
    };

    ensure_eq!(
        error,
        RuntimeInputError::Limit {
            limit: RuntimeInputByteLimit::new(2),
            attempted_len: RuntimeInputByteCount::new(3),
        },
    )?;

    let Err(error) = runtime_input("a\u{80}".as_bytes()) else {
        return Err(TestFailure::message("expected input error"));
    };

    ensure_matches(
        matches!(
            error,
            RuntimeInputError::NonAscii { column, .. } if column.get() == 2
        ),
        "expected non-ASCII input error at the original column",
    )
}

/// # Errors
///
/// Returns `TestFailure` if executable payload bytes and runtime-only bytes are
/// not kept in distinct domains.
#[test]
fn internal_code_and_runtime_bytes_are_distinct_domains() -> TestResult {
    let compact = [CompactByte::new(b'a', source_column(1)?)];
    let payload = Payload::parse(&compact, source_line_number(1)?, PayloadKind::LeftSideData)?;
    let input = runtime_input(b"a=()# ")?;
    let state = State::from_input(InitialStateBytes::materialize(
        &input,
        RunLimits::new(
            StepLimit::new(10_000),
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        ),
    )?);

    ensure_eq!(expect_payload_byte(&payload, 0)?, b'a')?;
    ensure_eq!(expect_runtime_byte(&state, 0)?, b'a')?;
    ensure_eq!(expect_runtime_byte(&state, 1)?, b'=')?;
    ensure_eq!(expect_runtime_byte(&state, 2)?, b'(')?;
    ensure_eq!(expect_runtime_byte(&state, 5)?, b' ')?;

    let program = parse_program("a=b")?;
    let result = program.run(
        &input,
        RunLimits::new(
            StepLimit::new(10_000),
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        ),
    )?;
    ensure_matches(
        matches!(
            result.outcome(),
            crate::RunOutcome::Stable(output) if output.as_bytes() == b"b=()# "
        ),
        "expected rewrite to leave runtime-only input bytes materialized but unmatched",
    )
}
