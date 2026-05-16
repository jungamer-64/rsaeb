pub mod support;

use rsaeb::error::{ParseErrorKind, ParseErrorLocation, PayloadKind, RunError};
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

/// # Errors
///
/// Returns `TestFailure` if parse errors lose structured location or kind
/// information.
#[test]
fn errors_parse_location_and_kind_are_structured() -> TestResult {
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
/// Returns `TestFailure` if payload or modifier errors lose domain-specific
/// information.
#[test]
fn errors_payload_and_modifier_kinds_keep_domain_information() -> TestResult {
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
/// Returns `TestFailure` if display output no longer names the expected domain
/// contexts.
#[test]
fn errors_display_output_names_domain_contexts() -> TestResult {
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

    let return_error = parse_program("a=(return)ok")?.run(
        &runtime_input(b"a")?,
        rsaeb::RunLimits::new(
            rsaeb::limits::StepLimit::new(1),
            rsaeb::limits::DEFAULT_MAX_STATE_LEN,
            rsaeb::limits::ReturnByteLimit::new(1),
        ),
    );
    ensure_matches(
        matches!(
            expect_run_error(return_error)?,
            RunError::Limit(rsaeb::error::LimitError::Return { .. })
        ),
        "expected return limit error",
    )
}
