//! Public `Program` contract tests.

#[path = "support/runtime.rs"]
mod runtime_support;
mod support;

use rsaeb::execution::CompleteRun;
use rsaeb::input::AdmittedRun;
use rsaeb::inspect::OnceRuleCount;
use rsaeb::policy::DefaultParsePolicy;
use rsaeb::program::{Program, RunOutcome, RunResult};
use rsaeb::source::ProgramSource;
use runtime_support::{
    DEFAULT_BYTE_BUDGET, DefaultInputRunPolicy, DefaultRunPolicy, TestRunPolicy,
};
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
fn runtime_input<I: rsaeb::policy::RuntimeInputPolicy, E: rsaeb::policy::ExecutionPolicy>(
    bytes: &[u8],
    limits: TestRunPolicy<I, E>,
) -> Result<AdmittedRun<E>, TestFailure> {
    runtime_support::admitted_run(bytes, limits)
}

/// # Errors
///
/// Returns `TestFailure` if public typed boundaries cannot parse or run simple
/// programs.
#[test]
fn program_parse_accepts_text_and_byte_sources() -> TestResult {
    let limits = DefaultRunPolicy::new();

    let program = parse_program("a=b")?;
    let input = runtime_input(b"a", limits)?;
    let result = program.execute::<CompleteRun, _>(input)?;
    expect_stable_bytes(&result, b"b")?;
    ensure_matches(result.steps().get() == 1, "expected one execution step")?;

    let program = Program::<DefaultParsePolicy>::parse(ProgramSource::from_bytes(b"a=b#\xff"))?;
    let input = runtime_input(b"a", limits)?;
    let result = program.execute::<CompleteRun, _>(input)?;
    expect_stable_bytes(&result, b"b")?;
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if public language whitespace, comments, or actions
/// drift from the expected contract.
#[test]
fn program_language_surface_handles_spacing_comments_and_actions() -> TestResult {
    let limits = DefaultInputRunPolicy::<10_000, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new();

    let program = parse_program("a b=bb")?;
    let result = program.execute::<CompleteRun, _>(runtime_input(b"abc", limits)?)?;
    expect_stable_bytes(&result, b"bbc")?;

    let program = parse_program("a=b\r\nb=c\r\n")?;
    let result = program.execute::<CompleteRun, _>(runtime_input(b"a", limits)?)?;
    expect_stable_bytes(&result, b"c")?;

    let program = parse_program("a\tb = c\tc")?;
    let result = program.execute::<CompleteRun, _>(runtime_input(b"ab", limits)?)?;
    expect_stable_bytes(&result, b"cc")?;

    let program = parse_program("a=b#ignored")?;
    let result = program.execute::<CompleteRun, _>(runtime_input(b"a", limits)?)?;
    expect_stable_bytes(&result, b"b")?;

    let program = parse_program("#a=b")?;
    let result = program.execute::<CompleteRun, _>(runtime_input(b"a", limits)?)?;
    expect_stable_bytes(&result, b"a")?;

    let program = parse_program("a=(start)x")?;
    let result = program.execute::<CompleteRun, _>(runtime_input(b"ba", limits)?)?;
    expect_stable_bytes(&result, b"xb")?;

    let program = parse_program("a=(end)x")?;
    let result = program.execute::<CompleteRun, _>(runtime_input(b"ba", limits)?)?;
    expect_stable_bytes(&result, b"bx")?;

    let program = parse_program("a=(return)ok")?;
    let result = program.execute::<CompleteRun, _>(runtime_input(b"a", limits)?)?;
    expect_return_bytes(&result, b"ok")?;
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if parsed programs are not reusable.
#[test]
fn program_values_are_reusable_across_runs() -> TestResult {
    let limits = DefaultInputRunPolicy::<10_000, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new();
    let program = parse_program("(once)a=b\na=c")?;
    let first = program.execute::<CompleteRun, _>(runtime_input(b"aa", limits)?)?;
    let second = program.execute::<CompleteRun, _>(runtime_input(b"aa", limits)?)?;

    expect_stable_bytes(&first, b"bc")?;
    expect_stable_bytes(&second, b"bc")?;
    ensure_eq!(program.rule_count().get(), 2)?;
    let once_rules: OnceRuleCount = program.once_rule_count();
    ensure_eq!(once_rules.get(), 1)
}
