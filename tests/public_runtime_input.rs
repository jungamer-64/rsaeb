//! Public `RuntimeInput` contract tests.

mod support;

use rsaeb::error::RunAdmissionError;
use rsaeb::execution::CompleteRun;
use rsaeb::input::{RuntimeInput, RuntimeInputSource};
use rsaeb::limits::{RuntimeInputByteLimit, RuntimeStateByteLimit};
use rsaeb::policy::{
    DefaultExecutionPolicy, DefaultParsePolicy, DefaultRuntimeInputPolicy, StaticExecutionPolicy,
    StaticRuntimeInputPolicy,
};
use rsaeb::program::{Program, RunOutcome, RunResult};
use rsaeb::source::ProgramSource;
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

/// Validates bytes with the default public runtime-input limit.
///
/// # Errors
///
/// Returns `RuntimeInputError` if validation rejects the bytes.
fn runtime_input(
    bytes: &[u8],
) -> Result<RuntimeInput<DefaultRuntimeInputPolicy>, rsaeb::error::RuntimeInputError> {
    RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(bytes))
}

/// # Errors
///
/// Returns `TestFailure` if runtime input loses owned typed bytes before it is
/// consumed by execution.
#[test]
fn runtime_input_moves_owned_bytes_into_execution() -> TestResult {
    let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(
        RuntimeInputSource::from_bytes(b"a=()# "),
    )?;

    ensure_eq!(input.byte_count().get(), 6)?;
    ensure_matches(!input.is_empty(), "expected non-empty owned input")?;

    let program = parse_program("a=b")?;
    let result = program.execute::<CompleteRun, _>(input.admit::<DefaultExecutionPolicy>()?)?;
    expect_stable_bytes(&result, b"b=()# ")
}

/// # Errors
///
/// Returns `TestFailure` if domain-specific default policies stop supporting
/// explicit default names.
#[test]
fn domain_default_policies_support_explicit_names() -> TestResult {
    let explicit_program = Program::<DefaultParsePolicy>::parse(ProgramSource::from_text("a=b"))?;

    let explicit_input =
        RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;

    let explicit_result = explicit_program
        .execute::<CompleteRun, _>(explicit_input.admit::<DefaultExecutionPolicy>()?)?;

    expect_stable_bytes(&explicit_result, b"b")
}

/// # Errors
///
/// Returns `TestFailure` if the runtime input public boundary accepts
/// non-ASCII bytes or rejects ASCII bytes.
#[test]
fn runtime_input_validates_ascii_boundary() -> TestResult {
    let input: Vec<u8> = (0x00..=0x7f).collect();
    let program = parse_program("# no executable rules")?;
    let runtime_input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(
        RuntimeInputSource::from_bytes(&input),
    )?;
    let result =
        program.execute::<CompleteRun, _>(runtime_input.admit::<DefaultExecutionPolicy>()?)?;
    expect_stable_bytes(&result, input.as_slice())?;
    ensure_eq!(result.steps().get(), 0)?;

    for byte in 0x80..=0xff {
        ensure_matches(
            RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(&[
                byte,
            ]))
            .is_err(),
            "byte should be rejected",
        )?;
    }
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if input errors or debug output lose public boundary
/// information.
#[test]
fn runtime_input_reports_public_errors_and_debug_bytes() -> TestResult {
    let Err(error) = runtime_input(&[0xff]) else {
        return Err(TestFailure::message("expected input error"));
    };

    ensure_matches(
        matches!(
            error,
            rsaeb::error::RuntimeInputError::NonAscii { column, .. } if column.get() == 1
        ),
        "expected runtime input error",
    )
}

/// # Errors
///
/// Returns `TestFailure` if runtime input byte-limit errors lose public boundary
/// information.
#[test]
fn runtime_input_reports_public_limit_errors() -> TestResult {
    let Err(limit_error) = RuntimeInput::<StaticRuntimeInputPolicy<1>>::validate(
        RuntimeInputSource::from_bytes(b"aa"),
    ) else {
        return Err(TestFailure::message(
            "expected input construction limit error",
        ));
    };
    ensure_matches(
        matches!(
            limit_error,
            rsaeb::error::RuntimeInputError::InputLimit {
                limit,
                attempted_len,
            } if limit == RuntimeInputByteLimit::new(1) && attempted_len.get() == 2
        ),
        "expected runtime input construction limit details",
    )
}

/// # Errors
///
/// Returns `TestFailure` if run admission loses public state-size boundary details.
#[test]
fn runtime_input_reports_public_admission_errors() -> TestResult {
    type SmallStateExecution = StaticExecutionPolicy<1, 1, 16_777_216>;
    let admitted_input = runtime_input(b"aa")?;
    let Err(state_limit_error) = admitted_input.admit::<SmallStateExecution>() else {
        return Err(TestFailure::message(
            "expected initial state admission limit error",
        ));
    };
    ensure_matches(
        matches!(
            state_limit_error,
            RunAdmissionError::InitialStateTooLarge {
                limit,
                attempted_len,
            } if limit == RuntimeStateByteLimit::new(1) && attempted_len.get() == 2
        ),
        "expected initial state admission limit details",
    )
}

/// # Errors
///
/// Returns `TestFailure` if debug output exposes internal runtime byte domains.
#[test]
fn runtime_input_debug_hides_internal_byte_domains() -> TestResult {
    let input = runtime_input(b"a=\n")?;
    let debug = format!("{input:?}");
    ensure_eq!(debug.as_str(), "RuntimeInput { bytes: [97, 61, 10] }")?;
    ensure_matches(
        !debug.contains("RuntimeByte")
            && !debug.contains("ProgramConstructible")
            && !debug.contains("NonProgramAsciiByte"),
        "expected runtime input debug to hide internal byte domain",
    )
}
