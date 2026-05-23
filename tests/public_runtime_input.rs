//! Public `RunInput` contract tests.

mod support;

use rsaeb::input::{RunInput, RuntimeInputSource};
use rsaeb::limits::{
    DEFAULT_MAX_INPUT_LEN, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_STEPS,
    RunLimits, RuntimeInputByteLimit, RuntimeStateByteLimit, StepLimit,
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
    let limits = RunLimits::new(
        DEFAULT_MAX_INPUT_LEN,
        DEFAULT_MAX_STEPS,
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let input = RunInput::validate(RuntimeInputSource::from_bytes(b"a=()# "), limits)?;

    ensure_eq!(input.byte_count().get(), 6)?;
    ensure_matches(!input.is_empty(), "expected non-empty owned input")?;

    let program = parse_program("a=b")?;
    let result = program.run(input)?;
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
    let limits = RunLimits::new(
        DEFAULT_MAX_INPUT_LEN,
        DEFAULT_MAX_STEPS,
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let result = program.run(RunInput::validate(
        RuntimeInputSource::from_bytes(&input),
        limits,
    )?)?;
    expect_stable_bytes(&result, input.as_slice())?;
    ensure_eq!(result.steps().get(), 0)?;

    for byte in 0x80..=0xff {
        ensure_matches(
            RunInput::validate(RuntimeInputSource::from_bytes(&[byte]), limits).is_err(),
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
    let Err(error) = RunInput::validate(
        RuntimeInputSource::from_bytes(&[0xff]),
        RunLimits::new(
            DEFAULT_MAX_INPUT_LEN,
            DEFAULT_MAX_STEPS,
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        ),
    ) else {
        return Err(TestFailure::message("expected input error"));
    };

    ensure_matches(
        matches!(
            error,
            rsaeb::error::RunInputError::NonAscii { column, .. } if column.get() == 1
        ),
        "expected runtime input error",
    )?;

    let Err(limit_error) = RunInput::validate(
        RuntimeInputSource::from_bytes(b"aa"),
        RunLimits::new(
            RuntimeInputByteLimit::new(1),
            DEFAULT_MAX_STEPS,
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        ),
    ) else {
        return Err(TestFailure::message(
            "expected input construction limit error",
        ));
    };
    ensure_matches(
        matches!(
            limit_error,
            rsaeb::error::RunInputError::InputLimit {
                limit,
                attempted_len,
            } if limit == RuntimeInputByteLimit::new(1) && attempted_len.get() == 2
        ),
        "expected runtime input construction limit details",
    )?;

    let Err(state_limit_error) = RunInput::validate(
        RuntimeInputSource::from_bytes(b"aa"),
        RunLimits::new(
            DEFAULT_MAX_INPUT_LEN,
            StepLimit::new(1),
            RuntimeStateByteLimit::new(1),
            DEFAULT_MAX_RETURN_LEN,
        ),
    ) else {
        return Err(TestFailure::message(
            "expected initial state admission limit error",
        ));
    };
    ensure_matches(
        matches!(
            state_limit_error,
            rsaeb::error::RunInputError::InitialStateLimit {
                limit,
                attempted_len,
            } if limit == RuntimeStateByteLimit::new(1) && attempted_len.get() == 2
        ),
        "expected initial state admission limit details",
    )?;

    let input = RunInput::validate(
        RuntimeInputSource::from_bytes(b"a=\n"),
        RunLimits::new(
            DEFAULT_MAX_INPUT_LEN,
            DEFAULT_MAX_STEPS,
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        ),
    )?;
    let debug = format!("{input:?}");
    ensure_eq!(debug.as_str(), "RunInput { bytes: [97, 61, 10] }")?;
    ensure_matches(
        !debug.contains("RuntimeByte")
            && !debug.contains("ProgramConstructible")
            && !debug.contains("NonProgramAsciiByte"),
        "expected runtime input debug to hide internal byte domain",
    )
}
