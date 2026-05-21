//! Public limit model contract tests.

mod support;

use rsaeb::error::{LimitError, ParseErrorKind, ParseLimitError, RunError, StateLimitContext};
use rsaeb::limits::{
    CodeLineByteLimit, DEFAULT_MAX_INPUT_LEN, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN,
    DEFAULT_PARSE_LIMITS, PayloadByteLimit, ReturnByteLimit, RuleLimit, RuntimeStateByteLimit,
    SourceByteLimit, StepLimit,
};
use rsaeb::{Program, ProgramSource, RunLimits, RuntimeInput};
use support::{TestFailure, TestResult, ensure_eq, ensure_matches, parse_program};

/// Returns the expected runtime error.
///
/// # Errors
///
/// Returns `TestFailure` if the result succeeds.
fn expect_run_error<T>(result: Result<T, RunError>) -> Result<RunError, TestFailure> {
    match result {
        Ok(_) => Err(TestFailure::message("expected runtime error")),
        Err(error) => Ok(error),
    }
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

/// Returns the expected state limit error.
///
/// # Errors
///
/// Returns `TestFailure` if `error` is not a state limit error.
fn expect_state_limit(error: RunError) -> Result<LimitError, TestFailure> {
    match error {
        RunError::Limit(error @ LimitError::State { .. }) => Ok(error),
        RunError::Allocation(_) | RunError::StateSize(_) | RunError::Limit(_) => {
            Err(TestFailure::message("expected state limit error"))
        }
    }
}

/// Validates test bytes as runtime input.
///
/// # Errors
///
/// Returns `RuntimeInputError` if the bytes are not valid runtime input.
fn runtime_input(bytes: &[u8]) -> Result<RuntimeInput, rsaeb::error::RuntimeInputError> {
    RuntimeInput::validate(bytes, DEFAULT_MAX_INPUT_LEN)
}

/// # Errors
///
/// Returns `TestFailure` if parser resource limits are not reported through
/// structured parse-limit errors.
#[test]
fn limits_parse_resource_errors_are_structured() -> TestResult {
    let limits = DEFAULT_PARSE_LIMITS.with_source_byte_limit(SourceByteLimit::new(3));
    let Err(error) = Program::parse(ProgramSource::from_text("a=b\n"), limits) else {
        return Err(TestFailure::message("expected source limit error"));
    };
    match error.kind() {
        ParseErrorKind::Limit(ParseLimitError::Source {
            limit,
            attempted_len,
        }) => {
            ensure_eq!(*limit, SourceByteLimit::new(3))?;
            ensure_eq!(attempted_len.get(), 4)?;
        }
        _ => return Err(TestFailure::message("expected source limit error")),
    }

    let limits = DEFAULT_PARSE_LIMITS.with_code_line_byte_limit(CodeLineByteLimit::new(3));
    let Err(error) = Program::parse(ProgramSource::from_text("ab=c"), limits) else {
        return Err(TestFailure::message("expected code-line limit error"));
    };
    match error.kind() {
        ParseErrorKind::Limit(ParseLimitError::CodeLine {
            limit,
            attempted_len,
        }) => {
            ensure_eq!(*limit, CodeLineByteLimit::new(3))?;
            ensure_eq!(attempted_len.get(), 4)?;
        }
        _ => return Err(TestFailure::message("expected code-line limit error")),
    }

    let limits = DEFAULT_PARSE_LIMITS.with_payload_byte_limit(PayloadByteLimit::new(1));
    let Err(error) = Program::parse(ProgramSource::from_text("ab=c"), limits) else {
        return Err(TestFailure::message("expected payload limit error"));
    };
    match error.kind() {
        ParseErrorKind::Limit(ParseLimitError::Payload {
            limit,
            attempted_len,
        }) => {
            ensure_eq!(*limit, PayloadByteLimit::new(1))?;
            ensure_eq!(attempted_len.get(), 2)?;
        }
        _ => return Err(TestFailure::message("expected payload limit error")),
    }

    let limits = DEFAULT_PARSE_LIMITS.with_rule_limit(RuleLimit::new(1));
    let Err(error) = Program::parse(ProgramSource::from_text("a=b\nb=c"), limits) else {
        return Err(TestFailure::message("expected rule limit error"));
    };
    match error.kind() {
        ParseErrorKind::Limit(ParseLimitError::Rules {
            limit,
            attempted_count,
        }) => {
            ensure_eq!(*limit, RuleLimit::new(1))?;
            ensure_eq!(attempted_count.get(), 2)?;
        }
        _ => return Err(TestFailure::message("expected rule limit error")),
    }

    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if public limit errors no longer preserve distinct
/// step, state, and return domains.
#[test]
fn limits_runtime_variants_preserve_typed_domains() -> TestResult {
    let step_limited = parse_program("a=b")?.run(
        &runtime_input(b"a")?,
        RunLimits::new(
            StepLimit::new(0),
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        ),
    );
    let step_limited = expect_step_limit(expect_run_error(step_limited)?)?;
    ensure_matches(
        matches!(
            step_limited,
            LimitError::Step {
                max_steps,
                completed_steps,
                state_len,
            } if max_steps == StepLimit::new(0)
                && completed_steps.get() == 0
                && state_len.get() == 1
        ),
        "expected step limit details",
    )?;

    let state_limited = parse_program("# no executable rules")?.run(
        &runtime_input(b"aa")?,
        RunLimits::new(
            StepLimit::new(10),
            RuntimeStateByteLimit::new(1),
            ReturnByteLimit::new(10),
        ),
    );
    let state_limited = expect_state_limit(expect_run_error(state_limited)?)?;
    ensure_matches(
        matches!(
            state_limited,
            LimitError::State {
                context: StateLimitContext::Input,
                limit,
                attempted_len,
            } if limit == RuntimeStateByteLimit::new(1)
                && attempted_len.get() == 2
        ),
        "expected runtime input state limit",
    )?;

    let return_limited = parse_program("a=(return)ok")?.run(
        &runtime_input(b"a")?,
        RunLimits::new(
            StepLimit::new(1),
            RuntimeStateByteLimit::new(10),
            ReturnByteLimit::new(1),
        ),
    );
    let return_limited = expect_run_error(return_limited)?;
    ensure_matches(
        matches!(
            return_limited,
            RunError::Limit(LimitError::Return {
                limit,
                attempted_len,
            }) if limit == ReturnByteLimit::new(1) && attempted_len.get() == 2
        ),
        "expected return limit details",
    )
}

/// # Errors
///
/// Returns `TestFailure` if step or state limit display strings lose their
/// public domain details.
#[test]
fn limits_display_output_names_public_contexts() -> TestResult {
    let state_error = parse_program("# no executable rules")?.run(
        &runtime_input(b"aa")?,
        RunLimits::new(
            StepLimit::new(10),
            RuntimeStateByteLimit::new(1),
            ReturnByteLimit::new(10),
        ),
    );
    let state_error = expect_state_limit(expect_run_error(state_error)?)?;
    ensure_eq!(
        state_error.to_string(),
        "state limit exceeded by runtime input; attempted length: 2, limit: 1",
    )?;

    let step_error = parse_program("a=b")?.run(
        &runtime_input(b"a")?,
        RunLimits::new(
            StepLimit::new(0),
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        ),
    );
    let step_error = expect_step_limit(expect_run_error(step_error)?)?;
    ensure_eq!(
        step_error.to_string(),
        "step limit exceeded after 0 steps; max steps: 0, state length: 1 bytes",
    )
}
