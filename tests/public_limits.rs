//! Public limit model contract tests.

#[path = "support/runtime.rs"]
mod runtime_support;
mod support;

use rsaeb::error::{
    ParseErrorKind, ParseLimitError, RunAdmissionError, RunError, RunFinishError, RunStepError,
    RuntimeStateLimitError, StepLimitError,
};
use rsaeb::input::AdmittedRun;
use rsaeb::limits::{
    CodeLineByteLimit, PayloadByteLimit, ReturnByteLimit, RuleLimit, RuntimeStateByteLimit,
    SourceByteLimit, StepLimit,
};
use rsaeb::policy::{ExecutionPolicy, ParsePolicy, StaticParsePolicy};
use rsaeb::program::Program;
use rsaeb::source::ProgramSource;
use runtime_support::{DEFAULT_BYTE_BUDGET, DefaultInputRunPolicy, TestRunPolicy};
use support::{TestFailure, TestResult, ensure_eq, ensure_matches, parse_program};

enum ExpectedParseLimit {
    Source {
        limit: SourceByteLimit,
        attempted_len: usize,
    },
    CodeLine {
        limit: CodeLineByteLimit,
        attempted_len: usize,
    },
    Payload {
        limit: PayloadByteLimit,
        attempted_len: usize,
    },
    Rules {
        limit: RuleLimit,
        attempted_count: usize,
    },
}

enum ExpectedRunLimit {
    State {
        limit: RuntimeStateByteLimit,
        attempted_len: usize,
    },
    Return {
        limit: ReturnByteLimit,
        attempted_len: usize,
    },
}

struct ParseLimitCase {
    source: &'static str,
    expected: ExpectedParseLimit,
    message: &'static str,
}

struct RunLimitCase<I: rsaeb::policy::RuntimeInputPolicy, E: ExecutionPolicy> {
    program_source: &'static str,
    input: &'static [u8],
    limits: TestRunPolicy<I, E>,
    expected: ExpectedRunLimit,
    message: &'static str,
}

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
fn expect_step_limit(error: RunError) -> Result<StepLimitError, TestFailure> {
    match error {
        RunError::Finish(RunFinishError::Step(RunStepError::StepLimit(error))) => Ok(error),
        RunError::Start(_)
        | RunError::Finish(RunFinishError::Step(_) | RunFinishError::FinalOutput(_)) => {
            Err(TestFailure::message("expected step limit error"))
        }
    }
}

/// Returns the expected state limit error.
///
/// # Errors
///
/// Returns `TestFailure` if `error` is not a state limit error.
fn expect_state_limit(error: RunError) -> Result<RuntimeStateLimitError, TestFailure> {
    match error {
        RunError::Finish(RunFinishError::Step(RunStepError::RuntimeStateLimit(error))) => Ok(error),
        RunError::Start(_)
        | RunError::Finish(RunFinishError::Step(_) | RunFinishError::FinalOutput(_)) => {
            Err(TestFailure::message("expected state limit error"))
        }
    }
}

/// Ensures `error` preserves the expected step-limit details.
///
/// # Errors
///
/// Returns `TestFailure` if the step-limit details differ.
fn ensure_step_limit_details(error: &StepLimitError, message: &'static str) -> TestResult {
    ensure_matches(
        error.max_steps() == StepLimit::new(0)
            && error.completed_steps().get() == 0
            && error.state_len().get() == 1,
        message,
    )
}

/// Runs `program_source` under `limits` and ensures execution stops at step admission.
///
/// # Errors
///
/// Returns `TestFailure` if execution does not fail with the expected step-limit details.
fn ensure_step_limit_run<I: rsaeb::policy::RuntimeInputPolicy, E: ExecutionPolicy>(
    program_source: &'static str,
    limits: TestRunPolicy<I, E>,
    message: &'static str,
) -> TestResult {
    let result = parse_program(program_source)?.run(runtime_input(b"a", limits)?);
    let error = expect_step_limit(expect_run_error(result)?)?;
    ensure_step_limit_details(&error, message)
}

/// Ensures parsing reports the expected resource-limit domain.
///
/// # Errors
///
/// Returns `TestFailure` if parsing succeeds or reports another error domain.
fn ensure_parse_limit_error<P: ParsePolicy>(case: ParseLimitCase) -> TestResult {
    let Err(error) = Program::<P>::parse(ProgramSource::from_text(case.source)) else {
        return Err(TestFailure::message(case.message));
    };
    let matches_expected = match (error.kind(), case.expected) {
        (
            ParseErrorKind::Limit(ParseLimitError::Source {
                limit,
                attempted_len,
            }),
            ExpectedParseLimit::Source {
                limit: expected_limit,
                attempted_len: expected_len,
            },
        ) => *limit == expected_limit && attempted_len.get() == expected_len,
        (
            ParseErrorKind::Limit(ParseLimitError::CodeLine {
                limit,
                attempted_len,
            }),
            ExpectedParseLimit::CodeLine {
                limit: expected_limit,
                attempted_len: expected_len,
            },
        ) => *limit == expected_limit && attempted_len.get() == expected_len,
        (
            ParseErrorKind::Limit(ParseLimitError::Payload {
                limit,
                attempted_len,
            }),
            ExpectedParseLimit::Payload {
                limit: expected_limit,
                attempted_len: expected_len,
            },
        ) => *limit == expected_limit && attempted_len.get() == expected_len,
        (
            ParseErrorKind::Limit(ParseLimitError::Rules {
                limit,
                attempted_count,
            }),
            ExpectedParseLimit::Rules {
                limit: expected_limit,
                attempted_count: expected_count,
            },
        ) => *limit == expected_limit && attempted_count.get() == expected_count,
        _ => false,
    };
    ensure_matches(matches_expected, case.message)
}

/// Ensures execution reports the expected runtime limit domain.
///
/// # Errors
///
/// Returns `TestFailure` if execution succeeds or reports another limit domain.
fn ensure_run_limit<I: rsaeb::policy::RuntimeInputPolicy, E: ExecutionPolicy>(
    case: RunLimitCase<I, E>,
) -> TestResult {
    let result = parse_program(case.program_source)?.run(runtime_input(case.input, case.limits)?);
    let error = expect_run_error(result)?;
    ensure_matches(
        match (error, case.expected) {
            (
                RunError::Finish(RunFinishError::Step(RunStepError::RuntimeStateLimit(error))),
                ExpectedRunLimit::State {
                    limit: expected_limit,
                    attempted_len: expected_len,
                },
            ) => error.limit() == expected_limit && error.attempted_len().get() == expected_len,
            (
                RunError::Finish(RunFinishError::Step(RunStepError::ReturnOutputLimit(error))),
                ExpectedRunLimit::Return {
                    limit: expected_limit,
                    attempted_len: expected_len,
                },
            ) => error.limit() == expected_limit && error.attempted_len().get() == expected_len,
            (
                RunError::Start(_)
                | RunError::Finish(RunFinishError::Step(_) | RunFinishError::FinalOutput(_)),
                ExpectedRunLimit::State { .. } | ExpectedRunLimit::Return { .. },
            ) => false,
        },
        case.message,
    )
}

/// Ensures `error` is the expected state limit used by display contract tests.
///
/// # Errors
///
/// Returns `TestFailure` if the error is not the expected state limit.
fn ensure_display_state_limit(error: &RuntimeStateLimitError) -> TestResult {
    ensure_matches(
        error.limit() == RuntimeStateByteLimit::new(2) && error.attempted_len().get() == 3,
        "expected rewrite state limit",
    )
}

/// Validates test bytes as runtime input.
///
/// # Errors
///
/// Returns `RuntimeInputError` if the bytes are not valid runtime input.
fn runtime_input<I: rsaeb::policy::RuntimeInputPolicy, E: ExecutionPolicy>(
    bytes: &[u8],
    limits: TestRunPolicy<I, E>,
) -> Result<AdmittedRun<E>, TestFailure> {
    runtime_support::admitted_run(bytes, limits)
}

/// # Errors
///
/// Returns `TestFailure` if parser resource limits are not reported through
/// structured parse-limit errors.
#[test]
fn parse_resource_limit_errors_are_structured() -> TestResult {
    ensure_parse_limit_error::<
        StaticParsePolicy<3, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET, 1_000_000>,
    >(ParseLimitCase {
        source: "a=b\n",
        expected: ExpectedParseLimit::Source {
            limit: SourceByteLimit::new(3),
            attempted_len: 4,
        },
        message: "expected source limit error",
    })?;
    ensure_parse_limit_error::<
        StaticParsePolicy<DEFAULT_BYTE_BUDGET, 3, DEFAULT_BYTE_BUDGET, 1_000_000>,
    >(ParseLimitCase {
        source: "ab=c",
        expected: ExpectedParseLimit::CodeLine {
            limit: CodeLineByteLimit::new(3),
            attempted_len: 4,
        },
        message: "expected code-line limit error",
    })?;
    ensure_parse_limit_error::<
        StaticParsePolicy<DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET, 1, 1_000_000>,
    >(ParseLimitCase {
        source: "ab=c",
        expected: ExpectedParseLimit::Payload {
            limit: PayloadByteLimit::new(1),
            attempted_len: 2,
        },
        message: "expected payload limit error",
    })?;
    ensure_parse_limit_error::<
        StaticParsePolicy<DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET, 1>,
    >(ParseLimitCase {
        source: "a=b\nb=c",
        expected: ExpectedParseLimit::Rules {
            limit: RuleLimit::new(1),
            attempted_count: 2,
        },
        message: "expected rule limit error",
    })?;
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if step limit errors no longer preserve their typed
/// public domain details.
#[test]
fn step_limit_preserves_public_domain_details() -> TestResult {
    let step_limits = DefaultInputRunPolicy::<0, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new();
    ensure_step_limit_run("a=b", step_limits, "expected step limit details")
}

/// # Errors
///
/// Returns `TestFailure` if rewrite admission still happens after step budget
/// exhaustion.
#[test]
fn step_limit_precedes_rewrite_state_growth() -> TestResult {
    let oversized_rewrite = DefaultInputRunPolicy::<0, 1, DEFAULT_BYTE_BUDGET>::new();
    ensure_step_limit_run(
        "a=aaa",
        oversized_rewrite,
        "expected step limit before rewrite state limit",
    )
}

/// # Errors
///
/// Returns `TestFailure` if return materialization still happens after step
/// budget exhaustion.
#[test]
fn step_limit_precedes_return_materialization() -> TestResult {
    let oversized_return = DefaultInputRunPolicy::<0, DEFAULT_BYTE_BUDGET, 1>::new();
    ensure_step_limit_run(
        "a=(return)ok",
        oversized_return,
        "expected step limit before return materialization limit",
    )
}

/// # Errors
///
/// Returns `TestFailure` if initial runtime input admission no longer reports
/// the public state-size domain.
#[test]
fn initial_state_admission_preserves_public_domain_details() -> TestResult {
    let initial_state_limits = DefaultInputRunPolicy::<10, 1, 10>::new();
    let Err(initial_state_limited) = runtime_input(b"aa", initial_state_limits) else {
        return Err(TestFailure::message(
            "expected initial state admission error",
        ));
    };
    ensure_matches(
        matches!(
            initial_state_limited,
            TestFailure::Admission(RunAdmissionError::InitialStateTooLarge {
                limit,
                attempted_len,
            }) if limit == RuntimeStateByteLimit::new(1)
                && attempted_len.get() == 2
        ),
        "expected run admission state limit",
    )?;
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if runtime state or return limits no longer report
/// their public typed domains.
#[test]
fn runtime_limit_errors_preserve_public_domain_details() -> TestResult {
    ensure_run_limit(RunLimitCase {
        program_source: "=a",
        input: b"aa",
        limits: DefaultInputRunPolicy::<1, 2, 10>::new(),
        expected: ExpectedRunLimit::State {
            limit: RuntimeStateByteLimit::new(2),
            attempted_len: 3,
        },
        message: "expected rewrite state limit",
    })?;
    ensure_run_limit(RunLimitCase {
        program_source: "a=(return)ok",
        input: b"a",
        limits: DefaultInputRunPolicy::<1, 10, 1>::new(),
        expected: ExpectedRunLimit::Return {
            limit: ReturnByteLimit::new(1),
            attempted_len: 2,
        },
        message: "expected return limit details",
    })?;
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if step or state limit display strings lose their
/// public domain details.
#[test]
fn limits_display_output_names_public_contexts() -> TestResult {
    let input_limits = DefaultInputRunPolicy::<10, 1, 10>::new();
    let Err(input_error) = runtime_input(b"aa", input_limits) else {
        return Err(TestFailure::message("expected input admission error"));
    };
    ensure_matches(
        format!("{input_error:?}").contains("InitialStateTooLarge"),
        "expected admission error details",
    )?;

    let rewrite_limits = DefaultInputRunPolicy::<1, 2, 10>::new();
    let rewrite_error = parse_program("=a")?.run(runtime_input(b"aa", rewrite_limits)?);
    let rewrite_error = expect_state_limit(expect_run_error(rewrite_error)?)?;
    ensure_display_state_limit(&rewrite_error)?;
    ensure_eq!(
        rewrite_error.to_string(),
        "rewrite state limit exceeded; attempted length: 3, limit: 2",
    )?;

    let step_limits = DefaultInputRunPolicy::<0, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new();
    let step_error = parse_program("a=b")?.run(runtime_input(b"a", step_limits)?);
    let step_error = expect_step_limit(expect_run_error(step_error)?)?;
    ensure_eq!(
        step_error.to_string(),
        "step limit exceeded after 0 steps; max steps: 0, state length: 1 bytes",
    )
}
