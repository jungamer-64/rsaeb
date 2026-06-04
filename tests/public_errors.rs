//! Public error model contract tests.

#[path = "support/runtime.rs"]
mod runtime_support;
mod support;

use rsaeb::error::{
    ExecutableProgramParseError, ParseError, ParseErrorKind, ParseErrorLocation,
    ParseRepresentationError, PayloadKind, RunError, RunFinishError, RunStepError,
};
use rsaeb::input::{AdmittedRun, RuntimeInput, RuntimeInputSource};
use rsaeb::policy::{DefaultRuntimeInputPolicy, ExecutionPolicy};
use rsaeb::program::{ExecutableProgram, RunResult};
use runtime_support::{DEFAULT_BYTE_BUDGET, DefaultInputRunPolicy, TestRunPolicy};
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

/// Validates test bytes as runtime input.
///
/// # Errors
///
/// Returns `RuntimeInputError` if the bytes are not valid runtime input.
fn runtime_input<I: rsaeb::policy::RuntimeInputPolicy, E: rsaeb::policy::ExecutionPolicy>(
    bytes: &[u8],
    limits: TestRunPolicy<I, E>,
) -> Result<AdmittedRun<E>, TestFailure> {
    runtime_support::admitted_run(bytes, limits)
}

/// Executes a parsed program that is expected to contain executable rules.
///
/// # Errors
///
/// Returns `TestFailure` if the program is empty before execution can start.
fn run_executable_program<E>(
    program: &ExecutableProgram,
    admitted: AdmittedRun<E>,
) -> Result<Result<RunResult, RunError>, TestFailure>
where
    E: ExecutionPolicy,
{
    Ok(program.execute(admitted))
}

/// Returns the expected source parse error from executable parsing.
///
/// # Errors
///
/// Returns `TestFailure` if parsing succeeds or only fails because the source
/// has no executable rules.
fn expect_parse_error(source: &str) -> Result<ParseError, TestFailure> {
    match parse_program(source) {
        Ok(_) => Err(TestFailure::message("expected parse error")),
        Err(ExecutableProgramParseError::Parse(error)) => Ok(error),
        Err(ExecutableProgramParseError::NoExecutableRules) => Err(TestFailure::message(
            "expected parse error, got empty program",
        )),
    }
}

/// # Errors
///
/// Returns `TestFailure` if parse errors lose structured location or kind
/// information.
#[test]
fn errors_parse_location_and_kind_are_structured() -> TestResult {
    let error = expect_parse_error("a=b=c")?;

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
    let error = expect_parse_error("a = b (")?;
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

    let error = expect_parse_error("(start)(once)a=b")?;
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
    let parse_error = expect_parse_error("a=b=c")?;
    ensure_eq!(
        parse_error.to_string(),
        "parse error at line 1, column 4: multiple '=' characters are not allowed",
    )?;

    let Err(input_error) =
        RuntimeInput::validate::<DefaultRuntimeInputPolicy>(RuntimeInputSource::from_bytes(&[
            0xff,
        ]))
    else {
        return Err(TestFailure::message("expected input error"));
    };
    ensure_eq!(
        input_error.to_string(),
        "input error: non-ASCII byte 0xff at column 1",
    )?;

    let return_limits = DefaultInputRunPolicy::<1, DEFAULT_BYTE_BUDGET, 1>::new();
    let return_error = run_executable_program(
        &parse_program("a=(return)ok")?,
        runtime_input(b"a", return_limits)?,
    )?;
    ensure_matches(
        matches!(
            expect_run_error(return_error)?,
            RunError::Finish(RunFinishError::Step(RunStepError::ReturnOutputLimit(_)))
        ),
        "expected return limit error",
    )
}

/// # Errors
///
/// Returns `TestFailure` if exposed source representation errors lose display output.
#[test]
fn errors_representation_subdomain_is_public() -> TestResult {
    ensure_eq!(
        ParseRepresentationError::SourceLineNumber.to_string(),
        "source line number could not be represented",
    )
}
