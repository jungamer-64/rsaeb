//! Public limit model contract tests.

#[path = "support/runtime.rs"]
mod runtime_support;
mod support;

use rsaeb::error::{LimitError, ParseErrorKind, ParseLimitError, RunAdmissionError, RunError};
use rsaeb::input::RunSeed;
use rsaeb::limits::{
    CodeLineByteLimit, DEFAULT_MAX_INPUT_LEN, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN,
    DEFAULT_PARSE_LIMITS, ParseLimits, PayloadByteLimit, ReturnByteLimit, RuleLimit,
    RuntimeStateByteLimit, SourceByteLimit, StepLimit,
};
use rsaeb::program::Program;
use rsaeb::source::ProgramSource;
use runtime_support::TestRunPolicy;
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
    limits: ParseLimits,
    expected: ExpectedParseLimit,
    message: &'static str,
}

struct RunLimitCase {
    program_source: &'static str,
    input: &'static [u8],
    limits: TestRunPolicy,
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
fn expect_step_limit(error: RunError) -> Result<LimitError, TestFailure> {
    match error {
        RunError::Limit(error @ LimitError::Step { .. }) => Ok(error),
        RunError::Allocation(_) | RunError::StateSize(_) | RunError::Limit(_) => {
            Err(TestFailure::message("expected step limit error"))
        }
    }
}

/// Returns the expected state limit error.
///
/// # Errors
///
/// Returns `TestFailure` if `error` is not a state limit error.
fn expect_state_limit(error: RunError) -> Result<LimitError, TestFailure> {
    match error {
        RunError::Limit(error @ LimitError::State { .. }) => Ok(error),
        RunError::Allocation(_) | RunError::StateSize(_) | RunError::Limit(_) => {
            Err(TestFailure::message("expected state limit error"))
        }
    }
}

/// Ensures `error` preserves the expected step-limit details.
///
/// # Errors
///
/// Returns `TestFailure` if the step-limit details differ.
fn ensure_step_limit_details(error: &LimitError, message: &'static str) -> TestResult {
    ensure_matches(
        matches!(
            error,
            LimitError::Step {
                max_steps,
                completed_steps,
                state_len,
            } if *max_steps == StepLimit::new(0)
                && completed_steps.get() == 0
                && state_len.get() == 1
        ),
        message,
    )
}

/// Runs `program_source` under `limits` and ensures execution stops at step admission.
///
/// # Errors
///
/// Returns `TestFailure` if execution does not fail with the expected step-limit details.
fn ensure_step_limit_run(
    program_source: &'static str,
    limits: TestRunPolicy,
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
fn ensure_parse_limit_error(case: ParseLimitCase) -> TestResult {
    let Err(error) = Program::parse(ProgramSource::from_text(case.source), case.limits) else {
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
fn ensure_run_limit(case: RunLimitCase) -> TestResult {
    let result = parse_program(case.program_source)?.run(runtime_input(case.input, case.limits)?);
    let error = expect_run_error(result)?;
    ensure_matches(
        match (error, case.expected) {
            (
                RunError::Limit(LimitError::State {
                    limit,
                    attempted_len,
                }),
                ExpectedRunLimit::State {
                    limit: expected_limit,
                    attempted_len: expected_len,
                },
            ) => limit == expected_limit && attempted_len.get() == expected_len,
            (
                RunError::Limit(LimitError::Return {
                    limit,
                    attempted_len,
                }),
                ExpectedRunLimit::Return {
                    limit: expected_limit,
                    attempted_len: expected_len,
                },
            ) => limit == expected_limit && attempted_len.get() == expected_len,
            (
                RunError::Allocation(_) | RunError::StateSize(_) | RunError::Limit(_),
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
fn ensure_display_state_limit(error: &LimitError) -> TestResult {
    ensure_matches(
        matches!(
            error,
            LimitError::State {
                limit,
                attempted_len,
            } if *limit == RuntimeStateByteLimit::new(2)
                && attempted_len.get() == 3
        ),
        "expected rewrite state limit",
    )
}

/// Validates test bytes as runtime input.
///
/// # Errors
///
/// Returns `RuntimeInputError` if the bytes are not valid runtime input.
fn runtime_input(bytes: &[u8], limits: TestRunPolicy) -> Result<RunSeed, TestFailure> {
    runtime_support::run_seed(bytes, limits)
}

/// # Errors
///
/// Returns `TestFailure` if parser resource limits are not reported through
/// structured parse-limit errors.
#[test]
fn parse_resource_limit_errors_are_structured() -> TestResult {
    for case in [
        ParseLimitCase {
            source: "a=b\n",
            limits: ParseLimits::new(
                SourceByteLimit::new(3),
                DEFAULT_PARSE_LIMITS.code_line_byte_limit(),
                DEFAULT_PARSE_LIMITS.payload_byte_limit(),
                DEFAULT_PARSE_LIMITS.rule_limit(),
            ),
            expected: ExpectedParseLimit::Source {
                limit: SourceByteLimit::new(3),
                attempted_len: 4,
            },
            message: "expected source limit error",
        },
        ParseLimitCase {
            source: "ab=c",
            limits: ParseLimits::new(
                DEFAULT_PARSE_LIMITS.source_byte_limit(),
                CodeLineByteLimit::new(3),
                DEFAULT_PARSE_LIMITS.payload_byte_limit(),
                DEFAULT_PARSE_LIMITS.rule_limit(),
            ),
            expected: ExpectedParseLimit::CodeLine {
                limit: CodeLineByteLimit::new(3),
                attempted_len: 4,
            },
            message: "expected code-line limit error",
        },
        ParseLimitCase {
            source: "ab=c",
            limits: ParseLimits::new(
                DEFAULT_PARSE_LIMITS.source_byte_limit(),
                DEFAULT_PARSE_LIMITS.code_line_byte_limit(),
                PayloadByteLimit::new(1),
                DEFAULT_PARSE_LIMITS.rule_limit(),
            ),
            expected: ExpectedParseLimit::Payload {
                limit: PayloadByteLimit::new(1),
                attempted_len: 2,
            },
            message: "expected payload limit error",
        },
        ParseLimitCase {
            source: "a=b\nb=c",
            limits: ParseLimits::new(
                DEFAULT_PARSE_LIMITS.source_byte_limit(),
                DEFAULT_PARSE_LIMITS.code_line_byte_limit(),
                DEFAULT_PARSE_LIMITS.payload_byte_limit(),
                RuleLimit::new(1),
            ),
            expected: ExpectedParseLimit::Rules {
                limit: RuleLimit::new(1),
                attempted_count: 2,
            },
            message: "expected rule limit error",
        },
    ] {
        ensure_parse_limit_error(case)?;
    }
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if step limit errors no longer preserve their typed
/// public domain details.
#[test]
fn step_limit_preserves_public_domain_details() -> TestResult {
    let step_limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(0),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    ensure_step_limit_run("a=b", step_limits, "expected step limit details")
}

/// # Errors
///
/// Returns `TestFailure` if rewrite admission still happens after step budget
/// exhaustion.
#[test]
fn step_limit_precedes_rewrite_state_growth() -> TestResult {
    let oversized_rewrite = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(0),
        RuntimeStateByteLimit::new(1),
        DEFAULT_MAX_RETURN_LEN,
    );
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
    let oversized_return = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(0),
        DEFAULT_MAX_STATE_LEN,
        ReturnByteLimit::new(1),
    );
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
    let initial_state_limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(10),
        RuntimeStateByteLimit::new(1),
        ReturnByteLimit::new(10),
    );
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
    for case in [
        RunLimitCase {
            program_source: "=a",
            input: b"aa",
            limits: TestRunPolicy::new(
                DEFAULT_MAX_INPUT_LEN,
                StepLimit::new(1),
                RuntimeStateByteLimit::new(2),
                ReturnByteLimit::new(10),
            ),
            expected: ExpectedRunLimit::State {
                limit: RuntimeStateByteLimit::new(2),
                attempted_len: 3,
            },
            message: "expected rewrite state limit",
        },
        RunLimitCase {
            program_source: "a=(return)ok",
            input: b"a",
            limits: TestRunPolicy::new(
                DEFAULT_MAX_INPUT_LEN,
                StepLimit::new(1),
                RuntimeStateByteLimit::new(10),
                ReturnByteLimit::new(1),
            ),
            expected: ExpectedRunLimit::Return {
                limit: ReturnByteLimit::new(1),
                attempted_len: 2,
            },
            message: "expected return limit details",
        },
    ] {
        ensure_run_limit(case)?;
    }
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if step or state limit display strings lose their
/// public domain details.
#[test]
fn limits_display_output_names_public_contexts() -> TestResult {
    let input_limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(10),
        RuntimeStateByteLimit::new(1),
        ReturnByteLimit::new(10),
    );
    let Err(input_error) = runtime_input(b"aa", input_limits) else {
        return Err(TestFailure::message("expected input admission error"));
    };
    ensure_matches(
        format!("{input_error:?}").contains("InitialStateTooLarge"),
        "expected admission error details",
    )?;

    let rewrite_limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(1),
        RuntimeStateByteLimit::new(2),
        ReturnByteLimit::new(10),
    );
    let rewrite_error = parse_program("=a")?.run(runtime_input(b"aa", rewrite_limits)?);
    let rewrite_error = expect_state_limit(expect_run_error(rewrite_error)?)?;
    ensure_display_state_limit(&rewrite_error)?;
    ensure_eq!(
        rewrite_error.to_string(),
        "rewrite state limit exceeded; attempted length: 3, limit: 2",
    )?;

    let step_limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(0),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let step_error = parse_program("a=b")?.run(runtime_input(b"a", step_limits)?);
    let step_error = expect_step_limit(expect_run_error(step_error)?)?;
    ensure_eq!(
        step_error.to_string(),
        "step limit exceeded after 0 steps; max steps: 0, state length: 1 bytes",
    )
}
