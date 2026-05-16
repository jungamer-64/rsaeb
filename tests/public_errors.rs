mod support;

use rsaeb::error::{
    AebError, LimitError, ParseErrorKind, ParseErrorLocation, ParseLimitError, PayloadKind,
    RunError, RuntimeInputError, StateLimitContext,
};
use rsaeb::limits::{
    CodeLineByteLimit, DEFAULT_MAX_INPUT_LEN, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN,
    DEFAULT_PARSE_LIMITS, PayloadByteLimit, ReturnByteLimit, RuleLimit, RuntimeInputByteLimit,
    RuntimeStateByteLimit, SourceByteLimit, StepLimit,
};
use rsaeb::{Program, ProgramSource, RunLimits, RuntimeInput};
use support::{TestFailure, TestResult, ensure_eq, ensure_matches, parse_program, runtime_input};

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

/// # Errors
///
/// Returns `TestFailure` if parse errors lose structured location or kind
/// information.
#[test]
fn parse_error_location_and_kind_are_structured() -> TestResult {
    let Err(error) = parse_program("a=b=c") else {
        return Err(TestFailure::message("expected parse error"));
    };

    ensure_eq!(error.line().get(), 1)?;
    match error.location() {
        ParseErrorLocation::Position(position) => {
            ensure_eq!(position.line().get(), 1)?;
            ensure_eq!(position.column().get(), 4)?;
        }
        ParseErrorLocation::Line(_) => {
            return Err(TestFailure::message("expected positioned parse error"));
        }
    }
    ensure_matches(
        matches!(error.kind(), ParseErrorKind::MultipleEquals),
        "expected multiple-equals parse error",
    )
}

/// # Errors
///
/// Returns `TestFailure` if parser resource limits are not reported through
/// structured parse-limit errors.
#[test]
fn parse_resource_limits_are_structured() -> TestResult {
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
/// Returns `TestFailure` if payload or modifier errors lose domain-specific
/// information.
#[test]
fn payload_and_modifier_errors_keep_domain_information() -> TestResult {
    let Err(error) = parse_program("a = b (") else {
        return Err(TestFailure::message("expected reserved syntax error"));
    };
    ensure_matches(
        matches!(
            error.kind(),
            ParseErrorKind::ReservedSyntaxInPayload {
                payload_kind: PayloadKind::RightSideData,
                ..
            }
        ),
        "expected right payload syntax error",
    )?;

    let Err(error) = parse_program("(start)(once)a=b") else {
        return Err(TestFailure::message("expected modifier order error"));
    };
    ensure_matches(
        matches!(
            error.kind(),
            ParseErrorKind::UnsupportedLeftModifierOrder { .. }
        ),
        "expected left modifier order error",
    )
}

/// # Errors
///
/// Returns `TestFailure` if input errors or the top-level error wrapper lose
/// structured variants.
#[test]
fn input_error_and_top_level_aeb_error_are_structured() -> TestResult {
    let Err(error) = runtime_input(&[0xff]) else {
        return Err(TestFailure::message("expected input error"));
    };

    ensure_matches(
        matches!(
            error,
            RuntimeInputError::NonAscii { column, .. } if column.get() == 1
        ),
        "expected runtime input error",
    )?;

    let error = AebError::from(error);
    ensure_matches(
        matches!(error, AebError::Input(_)),
        "expected top-level input error",
    )?;

    let Err(limit_error) = RuntimeInput::validate(b"aa", RuntimeInputByteLimit::new(1)) else {
        return Err(TestFailure::message(
            "expected input construction limit error",
        ));
    };
    ensure_matches(
        matches!(
            limit_error,
            RuntimeInputError::Limit {
                limit,
                attempted_len,
            } if limit == RuntimeInputByteLimit::new(1) && attempted_len.get() == 2
        ),
        "expected runtime input construction limit details",
    )?;

    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if runtime input debug output exposes internal byte
/// domain names or bytes drift.
#[test]
fn runtime_input_debug_materializes_public_bytes() -> TestResult {
    let input = RuntimeInput::validate(b"a=\n", DEFAULT_MAX_INPUT_LEN)?;
    let debug = format!("{input:?}");

    ensure_eq!(debug.as_str(), "RuntimeInput { bytes: [97, 61, 10] }")?;
    ensure_matches(
        !debug.contains("RuntimeByte")
            && !debug.contains("ProgramConstructible")
            && !debug.contains("NonProgramAsciiByte"),
        "expected runtime input debug to hide internal byte domain",
    )
}

/// # Errors
///
/// Returns `TestFailure` if display output no longer names the expected domain
/// contexts.
#[test]
fn display_errors_name_their_domain_contexts() -> TestResult {
    let Err(parse_error) = parse_program("a=b=c") else {
        return Err(TestFailure::message("expected parse error"));
    };
    ensure_eq!(
        parse_error.to_string(),
        "parse error at line 1, column 4: multiple '=' characters are not allowed",
    )?;

    let Err(input_error) = runtime_input(&[0xff]) else {
        return Err(TestFailure::message("expected input error"));
    };
    ensure_eq!(
        input_error.to_string(),
        "input error: non-ASCII byte 0xff at column 1",
    )?;

    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if step, state, or return limit errors lose their
/// public domain details.
#[test]
fn limit_errors_report_step_state_and_return_domains() -> TestResult {
    let state_error = parse_program("# no executable rules")?.run(
        &runtime_input(b"aa")?,
        RunLimits::new(
            StepLimit::new(10),
            RuntimeStateByteLimit::new(1),
            ReturnByteLimit::new(10),
        ),
    );
    let state_error = expect_state_limit(expect_run_error(state_error)?)?;
    ensure_matches(
        matches!(
            state_error,
            LimitError::State {
                context: StateLimitContext::Input,
                limit,
                attempted_len,
            } if limit == RuntimeStateByteLimit::new(1) && attempted_len.get() == 2
        ),
        "expected input state limit details",
    )?;
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
    )?;

    let return_error = parse_program("a=(return)ok")?.run(
        &runtime_input(b"a")?,
        RunLimits::new(
            StepLimit::new(1),
            DEFAULT_MAX_STATE_LEN,
            ReturnByteLimit::new(1),
        ),
    );
    ensure_matches(
        matches!(
            expect_run_error(return_error)?,
            RunError::Limit(LimitError::Return { .. })
        ),
        "expected return limit error",
    )
}
