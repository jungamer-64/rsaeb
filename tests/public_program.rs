//! Public parsed-program contract tests.

#[path = "support/runtime.rs"]
mod runtime_support;
mod support;

use rsaeb::error::{EmptyProgramParseError, ExecutableProgramParseError};
use rsaeb::input::AdmittedRun;
use rsaeb::inspect::RuleView;
use rsaeb::policy::{DefaultParsePolicy, ExecutionPolicy, StaticParsePolicy};
use rsaeb::program::{EmptyProgram, ExecutableProgram, RunOutcome, RunResult};
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

/// Executes a parsed program that is expected to contain executable rules.
///
/// # Errors
///
/// Returns `TestFailure` if the program is empty or execution fails.
fn execute_program<E>(
    program: &ExecutableProgram,
    admitted: AdmittedRun<E>,
) -> Result<RunResult, TestFailure>
where
    E: ExecutionPolicy,
{
    Ok(program.execute(admitted)?)
}

/// Stabilizes a parsed program that is expected to contain no executable rules.
///
/// # Errors
///
/// Returns `TestFailure` if the program is executable or stabilization fails.
fn stabilize_empty_program<E>(
    program: EmptyProgram,
    admitted: AdmittedRun<E>,
) -> Result<RunResult, TestFailure>
where
    E: ExecutionPolicy,
{
    Ok(program.stabilize(admitted)?)
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
    let result = execute_program(&program, input)?;
    expect_stable_bytes(&result, b"b")?;
    ensure_matches(result.steps().get() == 1, "expected one execution step")?;

    let program = ExecutableProgram::parse_bytes::<DefaultParsePolicy>(b"a=b#\xff")?;
    let input = runtime_input(b"a", limits)?;
    let result = execute_program(&program, input)?;
    expect_stable_bytes(&result, b"b")?;
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if construction policy provenance leaks into parsed
/// program value types.
#[test]
fn construction_parse_policy_does_not_parameterize_program_values() -> TestResult {
    type TightParse = StaticParsePolicy<32, 16, 8, 2>;

    let default: ExecutableProgram = ExecutableProgram::parse_text::<DefaultParsePolicy>("a=b")?;
    let tight: ExecutableProgram = ExecutableProgram::parse_text::<TightParse>("a=b")?;
    let _empty: EmptyProgram = EmptyProgram::parse_text::<TightParse>("# empty")?;

    ensure_eq!(default.rule_count(), tight.rule_count())?;
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
    let result = execute_program(&program, runtime_input(b"abc", limits)?)?;
    expect_stable_bytes(&result, b"bbc")?;

    let program = parse_program("a=b\r\nb=c\r\n")?;
    let result = execute_program(&program, runtime_input(b"a", limits)?)?;
    expect_stable_bytes(&result, b"c")?;

    let program = parse_program("a\tb = c\tc")?;
    let result = execute_program(&program, runtime_input(b"ab", limits)?)?;
    expect_stable_bytes(&result, b"cc")?;

    let program = parse_program("a=b#ignored")?;
    let result = execute_program(&program, runtime_input(b"a", limits)?)?;
    expect_stable_bytes(&result, b"b")?;

    let program = EmptyProgram::parse_text::<DefaultParsePolicy>("#a=b")?;
    let result = stabilize_empty_program(program, runtime_input(b"a", limits)?)?;
    expect_stable_bytes(&result, b"a")?;

    let program = parse_program("a=(start)x")?;
    let result = execute_program(&program, runtime_input(b"ba", limits)?)?;
    expect_stable_bytes(&result, b"xb")?;

    let program = parse_program("a=(end)x")?;
    let result = execute_program(&program, runtime_input(b"ba", limits)?)?;
    expect_stable_bytes(&result, b"bx")?;

    let program = parse_program("a=(return)ok")?;
    let result = execute_program(&program, runtime_input(b"a", limits)?)?;
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
    let first = execute_program(&program, runtime_input(b"aa", limits)?)?;
    let second = execute_program(&program, runtime_input(b"aa", limits)?)?;

    expect_stable_bytes(&first, b"bc")?;
    expect_stable_bytes(&second, b"bc")?;
    ensure_eq!(program.rule_count().get(), 2)?;
    let first_rule = program
        .rules()
        .next()
        .ok_or(TestFailure::message("expected first parsed rule"))?;
    ensure_matches(
        matches!(first_rule, RuleView::OnceRewrite(_)),
        "expected once rewrite rule shape",
    )
}

/// # Errors
///
/// Returns `TestFailure` if typed parse shape errors lose their public variants.
#[test]
fn typed_program_parse_reports_shape_mismatches() -> TestResult {
    let executable_error = ExecutableProgram::parse_text::<DefaultParsePolicy>("# empty");
    ensure_matches(
        matches!(
            executable_error,
            Err(ExecutableProgramParseError::NoExecutableRules)
        ),
        "expected executable parse to reject empty source",
    )?;

    let empty_error = EmptyProgram::parse_text::<DefaultParsePolicy>("a=b");
    ensure_matches(
        matches!(
            empty_error,
            Err(EmptyProgramParseError::ExecutableRule { line_number })
                if line_number.get() == 1
        ),
        "expected empty parse to reject executable source",
    )?;

    let empty_error =
        EmptyProgram::parse_text::<DefaultParsePolicy>("# comment\n\n(once)a=b\ninvalid");
    ensure_matches(
        matches!(
            empty_error,
            Err(EmptyProgramParseError::ExecutableRule { line_number })
                if line_number.get() == 3
        ),
        "expected empty parse to reject the first executable rule immediately",
    )
}
