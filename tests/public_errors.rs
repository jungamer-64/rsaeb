//! Public error model contract tests.

#[path = "support/runtime.rs"]
mod runtime_support;
mod support;

use rsaeb::error::{
    ParseErrorKind, ParseErrorLocation, ParseRepresentationError, PayloadKind, RunError,
    RunFinishError, RunStepError,
};
use rsaeb::input::{AdmittedRun, RuntimeInput, RuntimeInputSource};
use rsaeb::policy::{DefaultParsePolicy, DefaultRuntimeInputPolicy, ExecutionPolicy};
use rsaeb::program::{ParsedProgram, RunResult};
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
    program: &ParsedProgram<DefaultParsePolicy>,
    admitted: AdmittedRun<E>,
) -> Result<Result<RunResult, RunError>, TestFailure>
where
    E: ExecutionPolicy,
{
    match program {
        ParsedProgram::Executable(program) => Ok(program.execute(admitted)),
        ParsedProgram::Empty(_) => Err(TestFailure::message("expected executable program")),
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

    let Err(input_error) =
        RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(&[
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
/// Returns `TestFailure` if exposed parser representation errors lose display output.
#[test]
fn errors_representation_subdomain_is_public() -> TestResult {
    ensure_eq!(
        ParseRepresentationError::RulePosition.to_string(),
        "rule position could not be represented",
    )
}
