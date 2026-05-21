//! Public `Program` contract tests.

mod support;

use rsaeb::inspect::OnceRuleCount;
use rsaeb::limits::{
    DEFAULT_MAX_INPUT_LEN, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_STEPS,
    DEFAULT_PARSE_LIMITS, StepLimit,
};
use rsaeb::{Program, ProgramSource, RunLimits, RunOutcome, RunResult, RuntimeInput};
use support::{TestFailure, TestResult, ensure_eq, ensure_matches, parse_program};

/// Returns stable output bytes when they match `expected`.
///
/// # Errors
///
/// Returns `TestFailure` if the run result is not stable or stable bytes differ.
fn expect_stable_bytes<'result>(
    result: &'result RunResult,
    expected: &[u8],
) -> Result<&'result [u8], TestFailure> {
    match result.outcome() {
        RunOutcome::Stable(output) if output.as_slice() == expected => Ok(output.as_slice()),
        RunOutcome::Stable(_) => Err(TestFailure::message("stable output bytes differed")),
        RunOutcome::Return(_) => Err(TestFailure::message("expected stable outcome")),
    }
}

/// Returns return output bytes when they match `expected`.
///
/// # Errors
///
/// Returns `TestFailure` if the run result is not returned or return bytes
/// differ.
fn expect_return_bytes<'result>(
    result: &'result RunResult,
    expected: &[u8],
) -> Result<&'result [u8], TestFailure> {
    match result.outcome() {
        RunOutcome::Return(output) if output.as_slice() == expected => Ok(output.as_slice()),
        RunOutcome::Return(_) => Err(TestFailure::message("return output bytes differed")),
        RunOutcome::Stable(_) => Err(TestFailure::message("expected return outcome")),
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
/// Returns `TestFailure` if public typed boundaries cannot parse or run simple
/// programs.
#[test]
fn program_parse_accepts_text_and_byte_sources() -> TestResult {
    let limits = RunLimits::new(
        DEFAULT_MAX_STEPS,
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );

    let program = parse_program("a=b")?;
    let input = runtime_input(b"a")?;
    let result = program.run(&input, limits)?;
    expect_stable_bytes(&result, b"b")?;
    ensure_matches(result.steps().get() == 1, "expected one rewrite step")?;

    let program = Program::parse(ProgramSource::from_bytes(b"a=b#\xff"), DEFAULT_PARSE_LIMITS)?;
    let input = runtime_input(b"a")?;
    let result = program.run(&input, limits)?;
    expect_stable_bytes(&result, b"b")?;
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if public language whitespace, comments, or actions
/// drift from the expected contract.
#[test]
fn program_language_surface_handles_spacing_comments_and_actions() -> TestResult {
    let limits = RunLimits::new(
        StepLimit::new(10_000),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );

    let program = parse_program("a b=bb")?;
    let result = program.run(&runtime_input(b"abc")?, limits)?;
    expect_stable_bytes(&result, b"bbc")?;

    let program = parse_program("a=b\r\nb=c\r\n")?;
    let result = program.run(&runtime_input(b"a")?, limits)?;
    expect_stable_bytes(&result, b"c")?;

    let program = parse_program("a\tb = c\tc")?;
    let result = program.run(&runtime_input(b"ab")?, limits)?;
    expect_stable_bytes(&result, b"cc")?;

    let program = parse_program("a=b#ignored")?;
    let result = program.run(&runtime_input(b"a")?, limits)?;
    expect_stable_bytes(&result, b"b")?;

    let program = parse_program("#a=b")?;
    let result = program.run(&runtime_input(b"a")?, limits)?;
    expect_stable_bytes(&result, b"a")?;

    let program = parse_program("a=(start)x")?;
    let result = program.run(&runtime_input(b"ba")?, limits)?;
    expect_stable_bytes(&result, b"xb")?;

    let program = parse_program("a=(end)x")?;
    let result = program.run(&runtime_input(b"ba")?, limits)?;
    expect_stable_bytes(&result, b"bx")?;

    let program = parse_program("a=(return)ok")?;
    let result = program.run(&runtime_input(b"a")?, limits)?;
    expect_return_bytes(&result, b"ok")?;
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if parsed programs are not reusable.
#[test]
fn program_values_are_reusable_across_runs() -> TestResult {
    let limits = RunLimits::new(
        StepLimit::new(10_000),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let program = parse_program("(once)a=b\na=c")?;
    let first = program.run(&runtime_input(b"aa")?, limits)?;
    let second = program.run(&runtime_input(b"aa")?, limits)?;

    expect_stable_bytes(&first, b"bc")?;
    expect_stable_bytes(&second, b"bc")?;
    ensure_eq!(program.rule_count().get(), 2)?;
    let once_rules: OnceRuleCount = program.once_rule_count();
    ensure_eq!(once_rules.get(), 1)
}
