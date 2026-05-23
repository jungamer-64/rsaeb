//! Public `RuntimeInput` contract tests.

mod support;

use rsaeb::input::{RuntimeInput, RuntimeInputSource};
use rsaeb::limits::{
    DEFAULT_MAX_INPUT_LEN, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_STEPS,
    RunLimits, RuntimeInputByteLimit,
};
use rsaeb::program::{RunOutcome, RunResult};
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

/// # Errors
///
/// Returns `TestFailure` if runtime input loses owned typed bytes before it is
/// consumed by execution.
#[test]
fn runtime_input_moves_owned_bytes_into_execution() -> TestResult {
    let input = RuntimeInput::validate(
        RuntimeInputSource::from_bytes(b"a=()# "),
        DEFAULT_MAX_INPUT_LEN,
    )?;

    ensure_eq!(input.byte_count().get(), 6)?;
    ensure_matches(!input.is_empty(), "expected non-empty owned input")?;

    let program = parse_program("a=b")?;
    let result = program.run(
        input,
        RunLimits::new(
            DEFAULT_MAX_STEPS,
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        ),
    )?;
    expect_stable_bytes(&result, b"b=()# ")
}

/// # Errors
///
/// Returns `TestFailure` if the runtime input public boundary accepts
/// non-ASCII bytes or rejects ASCII bytes.
#[test]
fn runtime_input_validates_ascii_boundary() -> TestResult {
    let input: Vec<u8> = (0x00..=0x7f).collect();
    let program = parse_program("# no executable rules")?;
    let result = program.run(
        RuntimeInput::validate(
            RuntimeInputSource::from_bytes(&input),
            DEFAULT_MAX_INPUT_LEN,
        )?,
        RunLimits::new(
            DEFAULT_MAX_STEPS,
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        ),
    )?;
    expect_stable_bytes(&result, input.as_slice())?;
    ensure_eq!(result.steps().get(), 0)?;

    for byte in 0x80..=0xff {
        ensure_matches(
            RuntimeInput::validate(
                RuntimeInputSource::from_bytes(&[byte]),
                DEFAULT_MAX_INPUT_LEN,
            )
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
    let Err(error) = RuntimeInput::validate(
        RuntimeInputSource::from_bytes(&[0xff]),
        DEFAULT_MAX_INPUT_LEN,
    ) else {
        return Err(TestFailure::message("expected input error"));
    };

    ensure_matches(
        matches!(
            error,
            rsaeb::error::RuntimeInputError::NonAscii { column, .. } if column.get() == 1
        ),
        "expected runtime input error",
    )?;

    let Err(limit_error) = RuntimeInput::validate(
        RuntimeInputSource::from_bytes(b"aa"),
        RuntimeInputByteLimit::new(1),
    ) else {
        return Err(TestFailure::message(
            "expected input construction limit error",
        ));
    };
    ensure_matches(
        matches!(
            limit_error,
            rsaeb::error::RuntimeInputError::Limit {
                limit,
                attempted_len,
            } if limit == RuntimeInputByteLimit::new(1) && attempted_len.get() == 2
        ),
        "expected runtime input construction limit details",
    )?;

    let input = RuntimeInput::validate(
        RuntimeInputSource::from_bytes(b"a=\n"),
        DEFAULT_MAX_INPUT_LEN,
    )?;
    let debug = format!("{input:?}");
    ensure_eq!(debug.as_str(), "RuntimeInput { bytes: [97, 61, 10] }")?;
    ensure_matches(
        !debug.contains("RuntimeByte")
            && !debug.contains("ProgramConstructible")
            && !debug.contains("NonProgramAsciiByte"),
        "expected runtime input debug to hide internal byte domain",
    )
}
