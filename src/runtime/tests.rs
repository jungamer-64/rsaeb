use super::state::State;
use crate::bytes::Payload;
use crate::error::{
    ReturnOutputLimitError, RunStepError, RuntimeInputError, RuntimeStateLimitError, StepLimitError,
};
use crate::execution::{BorrowedFailedRun, BorrowedStepTransition};
use crate::input::{RuntimeInput, RuntimeInputSource};
use crate::limits::{
    ReturnByteLimit, ReturnOutputByteCount, RuntimeInputByteCount, RuntimeInputByteLimit,
    RuntimeStateByteCount, RuntimeStateByteLimit, StepCount, StepLimit,
};
use crate::policy::{DefaultRuntimeInputPolicy, ExecutionPolicy};
use crate::program::RunOutcome;
use crate::test_support::{
    DEFAULT_BYTE_BUDGET, DefaultInputRunPolicy, TestFailure, TestInputPolicy, TestResult,
    admitted_run, ensure_eq, ensure_matches, executable_program, execute_program, parse_program,
};
use crate::trace::RuntimeStateView;
use alloc::vec::Vec;

fn runtime_view_bytes(view: RuntimeStateView<'_>) -> Vec<u8> {
    view.materialized_bytes().collect()
}

/// Returns the materialized runtime byte at `index`.
///
/// # Errors
///
/// Returns `TestFailure` if the state has no byte at `index`.
fn expect_runtime_byte(state: &State, index: usize) -> Result<u8, TestFailure> {
    state
        .view()
        .materialized_bytes()
        .nth(index)
        .ok_or(TestFailure::message("expected runtime byte"))
}

/// Returns the program payload byte at `index`.
///
/// # Errors
///
/// Returns `TestFailure` if the payload has no byte at `index`.
fn expect_payload_byte(payload: &Payload, index: usize) -> Result<u8, TestFailure> {
    payload
        .bytes()
        .nth(index)
        .ok_or(TestFailure::message("expected payload byte"))
}

/// Returns the expected step limit error.
///
/// # Errors
///
/// Returns `TestFailure` if `error` is not a step limit error.
fn expect_step_limit(error: RunStepError) -> Result<StepLimitError, TestFailure> {
    match error {
        RunStepError::StepLimit(error) => Ok(error),
        RunStepError::Allocation(_)
        | RunStepError::RewriteSize(_)
        | RunStepError::RuntimeStateLimit(_)
        | RunStepError::ReturnOutputLimit(_) => {
            Err(TestFailure::message("expected step limit error"))
        }
    }
}

/// Returns the expected step error.
///
/// # Errors
///
/// Returns `TestFailure` if stepping succeeds.
fn expect_step_error<'program, P: crate::policy::ParsePolicy, E: ExecutionPolicy>(
    result: BorrowedStepTransition<'program, P, E>,
) -> Result<BorrowedFailedRun<'program, P>, TestFailure> {
    match result {
        BorrowedStepTransition::Failed(failed) => Ok(failed),
        BorrowedStepTransition::Applied(_)
        | BorrowedStepTransition::Stable(_)
        | BorrowedStepTransition::Returned(_) => Err(TestFailure::message("expected step error")),
    }
}

/// Returns the expected successful step transition.
///
/// # Errors
///
/// Returns `TestFailure` if stepping fails.
fn expect_step_transition<'program, P: crate::policy::ParsePolicy, E: ExecutionPolicy>(
    result: BorrowedStepTransition<'program, P, E>,
) -> Result<BorrowedStepTransition<'program, P, E>, TestFailure> {
    match result {
        BorrowedStepTransition::Failed(failed) => Err(TestFailure::from(failed.into_error())),
        transition => Ok(transition),
    }
}

/// # Errors
///
/// Returns `TestFailure` if a failed once-rule commit attempt mutates runtime
/// state before the commit boundary.
#[test]
fn once_rule_failure_preserves_state_before_step_commit() -> TestResult {
    let program = parse_program("(once)a=(return)ok")?;
    let limits = DefaultInputRunPolicy::<1, DEFAULT_BYTE_BUDGET, 1>::new();
    let input = admitted_run(b"a", limits)?;
    let runtime = executable_program(&program)?.steps(input)?;
    let error = expect_step_error(runtime.step())?;
    ensure_eq!(
        error.error(),
        &RunStepError::ReturnOutputLimit(ReturnOutputLimitError::new(
            ReturnByteLimit::new(1),
            ReturnOutputByteCount::new(2),
        )),
    )?;

    ensure_eq!(error.completed_steps(), StepCount::ZERO)?;
    ensure_eq!(
        runtime_view_bytes(error.state()).as_slice(),
        b"a".as_slice()
    )
}

/// # Errors
///
/// Returns `TestFailure` if a step-limit failure commits state or loses the
/// running execution.
#[test]
fn execution_step_limit_failure_preserves_uncommitted_state() -> TestResult {
    let program = parse_program("a=b")?;
    let limits = DefaultInputRunPolicy::<0, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new();
    let no_match_input = admitted_run(b"x", limits)?;
    let no_match = executable_program(&program)?.steps(no_match_input)?;
    match expect_step_transition(no_match.step())? {
        BorrowedStepTransition::Stable(stable) => {
            ensure_eq!(stable.steps().get(), 0)?;
            ensure_eq!(
                runtime_view_bytes(stable.state()).as_slice(),
                b"x".as_slice()
            )?;
        }
        BorrowedStepTransition::Applied(_)
        | BorrowedStepTransition::Returned(_)
        | BorrowedStepTransition::Failed(_) => {
            return Err(TestFailure::message("expected stable completion"));
        }
    }

    let program = parse_program("a=b")?;
    let would_match_input = admitted_run(b"a", limits)?;
    let would_match = executable_program(&program)?.steps(would_match_input)?;
    let error = expect_step_error(would_match.step())?;
    ensure_eq!(
        expect_step_limit(error.into_error())?,
        StepLimitError::new(
            StepLimit::new(0),
            StepCount::ZERO,
            RuntimeStateByteCount::new(1),
        ),
    )?;
    let program = parse_program("a=b")?;
    let would_match = executable_program(&program)?.steps(admitted_run(b"a", limits)?)?;
    let error = expect_step_error(would_match.step())?;
    ensure_eq!(error.completed_steps(), StepCount::ZERO)?;
    ensure_eq!(
        runtime_view_bytes(error.state()).as_slice(),
        b"a".as_slice(),
    )?;

    ensure_eq!(
        expect_step_limit(error.into_error())?,
        StepLimitError::new(
            StepLimit::new(0),
            StepCount::ZERO,
            RuntimeStateByteCount::new(1),
        ),
    )
}

/// # Errors
///
/// Returns `TestFailure` if state or return-size limit failures commit state.
#[test]
fn execution_size_limit_failures_preserve_uncommitted_state() -> TestResult {
    let state_limits = DefaultInputRunPolicy::<1, 2, 10>::new();
    let state_program = parse_program("=a")?;
    let state_input = admitted_run(b"aa", state_limits)?;
    let state_limited = executable_program(&state_program)?.steps(state_input)?;
    let state_error = expect_step_error(state_limited.step())?;
    ensure_eq!(
        state_error.error(),
        &RunStepError::RuntimeStateLimit(RuntimeStateLimitError::new(
            RuntimeStateByteLimit::new(2),
            RuntimeStateByteCount::new(3),
        )),
    )?;
    ensure_eq!(state_error.completed_steps(), StepCount::ZERO)?;
    ensure_eq!(
        runtime_view_bytes(state_error.state()).as_slice(),
        b"aa".as_slice(),
    )?;
    ensure_eq!(
        state_error.into_error(),
        RunStepError::RuntimeStateLimit(RuntimeStateLimitError::new(
            RuntimeStateByteLimit::new(2),
            RuntimeStateByteCount::new(3),
        )),
    )?;

    let return_limits = DefaultInputRunPolicy::<1, 10, 1>::new();
    let return_program = parse_program("a=(return)ok")?;
    let return_input = admitted_run(b"a", return_limits)?;
    let return_limited = executable_program(&return_program)?.steps(return_input)?;
    let return_error = expect_step_error(return_limited.step())?;
    ensure_eq!(
        return_error.error(),
        &RunStepError::ReturnOutputLimit(ReturnOutputLimitError::new(
            ReturnByteLimit::new(1),
            ReturnOutputByteCount::new(2),
        )),
    )?;
    ensure_eq!(return_error.completed_steps(), StepCount::ZERO)?;
    ensure_eq!(
        runtime_view_bytes(return_error.state()).as_slice(),
        b"a".as_slice(),
    )?;
    ensure_eq!(
        return_error.into_error(),
        RunStepError::ReturnOutputLimit(ReturnOutputLimitError::new(
            ReturnByteLimit::new(1),
            ReturnOutputByteCount::new(2),
        )),
    )
}

/// # Errors
///
/// Returns `TestFailure` if a return action enters rewrite state-limit
/// accounting instead of the return-output path.
#[test]
fn return_action_bypasses_rewrite_state_mutation_path() -> TestResult {
    let program = parse_program("a=(return)ok")?;
    let limits = DefaultInputRunPolicy::<1, 1, 2>::new();
    let session = executable_program(&program)?.steps(admitted_run(b"a", limits)?)?;

    match expect_step_transition(session.step())? {
        BorrowedStepTransition::Returned(returned) => {
            let result = returned.into_result();
            ensure_eq!(result.steps().get(), 1)?;
            ensure_matches(
                matches!(
                    result.outcome(),
                    RunOutcome::Return(output) if output.as_slice() == b"ok"
                ),
                "expected return output to bypass rewrite state limit",
            )
        }
        BorrowedStepTransition::Applied(_)
        | BorrowedStepTransition::Stable(_)
        | BorrowedStepTransition::Failed(_) => {
            Err(TestFailure::message("expected return transition"))
        }
    }
}

/// # Errors
///
/// Returns `TestFailure` if runtime input errors lose structured boundary
/// information.
#[test]
fn runtime_input_error_is_structured_at_the_runtime_boundary() -> TestResult {
    let Err(error) =
        RuntimeInput::<TestInputPolicy<2>>::validate(RuntimeInputSource::from_bytes(b"abc"))
    else {
        return Err(TestFailure::message("expected input limit error"));
    };

    ensure_eq!(
        error,
        RuntimeInputError::InputLimit {
            limit: RuntimeInputByteLimit::new(2),
            attempted_len: RuntimeInputByteCount::new(3),
        },
    )?;

    let Err(error) = RuntimeInput::<TestInputPolicy<1>>::validate(RuntimeInputSource::from_bytes(
        "a\u{80}".as_bytes(),
    )) else {
        return Err(TestFailure::message(
            "expected input limit before byte error",
        ));
    };

    ensure_eq!(
        error,
        RuntimeInputError::InputLimit {
            limit: RuntimeInputByteLimit::new(1),
            attempted_len: RuntimeInputByteCount::new(3),
        },
    )?;

    let Err(error) = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(
        RuntimeInputSource::from_bytes("a\u{80}".as_bytes()),
    ) else {
        return Err(TestFailure::message("expected input error"));
    };

    ensure_matches(
        matches!(
            error,
            RuntimeInputError::NonAscii { column, .. } if column.get() == 2
        ),
        "expected non-ASCII input error at the original column",
    )
}

/// # Errors
///
/// Returns `TestFailure` if executable payload bytes and runtime-only bytes are
/// not kept in distinct domains.
#[test]
fn internal_code_and_runtime_bytes_are_distinct_domains() -> TestResult {
    let program = parse_program("a=b")?;
    let executable = executable_program(&program)?;
    let payload = executable
        .rule_scan()
        .iter()
        .next()
        .ok_or(TestFailure::message("expected parsed rule"))?
        .lhs();
    let limits = DefaultInputRunPolicy::<10_000, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new();
    let (input, _) = admitted_run(b"a=()# ", limits)?.into_runtime_parts();
    let state = State::from_input(input);

    ensure_eq!(expect_payload_byte(payload, 0)?, b'a')?;
    ensure_eq!(expect_runtime_byte(&state, 0)?, b'a')?;
    ensure_eq!(expect_runtime_byte(&state, 1)?, b'=')?;
    ensure_eq!(expect_runtime_byte(&state, 2)?, b'(')?;
    ensure_eq!(expect_runtime_byte(&state, 5)?, b' ')?;

    let result = execute_program(&program, admitted_run(b"a=()# ", limits)?)?;
    ensure_matches(
        matches!(
            result.outcome(),
            RunOutcome::Stable(output) if output.as_slice() == b"b=()# "
        ),
        "expected rewrite to leave runtime-only input bytes materialized but unmatched",
    )
}

/// # Errors
///
/// Returns `TestFailure` if a consumed `(once)` rule can be matched again
/// before later rules are considered.
#[test]
fn once_rule_commit_proof_allows_only_one_successful_application() -> TestResult {
    let program = parse_program("(once)a=a\na=b")?;
    let limits = DefaultInputRunPolicy::<10, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new();
    let result = execute_program(&program, admitted_run(b"a", limits)?)?;

    ensure_eq!(result.steps().get(), 2)?;
    ensure_matches(
        matches!(
            result.outcome(),
            RunOutcome::Stable(output) if output.as_slice() == b"b"
        ),
        "expected consumed once rule to give the later rule a chance",
    )
}

/// # Errors
///
/// Returns `TestFailure` if rewrite action variants lose their placement
/// semantics after being prepared from matched state spans.
#[test]
fn rewrite_action_variants_preserve_runtime_placement() -> TestResult {
    for (source, input, expected) in [
        ("a=x", b"ab".as_slice(), b"xb".as_slice()),
        ("b=(start)x", b"ab".as_slice(), b"xa".as_slice()),
        ("a=(end)x", b"ab".as_slice(), b"bx".as_slice()),
    ] {
        let limits = DefaultInputRunPolicy::<1, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new();
        let result = execute_program(&parse_program(source)?, admitted_run(input, limits)?)?;

        ensure_matches(
            matches!(
                result.outcome(),
                RunOutcome::Stable(output) if output.as_slice() == expected
            ),
            "expected rewrite action variant to preserve placement",
        )?;
    }

    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if empty payload matches lose their start/end span
/// placement while deriving matched length from the validated range.
#[test]
fn empty_payload_matches_keep_anchor_specific_span_placement() -> TestResult {
    for (source, expected) in [
        ("=x", b"xab".as_slice()),
        ("(start)=x", b"xab".as_slice()),
        ("(end)=x", b"abx".as_slice()),
    ] {
        let program = parse_program(source)?;
        let limits = DefaultInputRunPolicy::<1, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new();
        let session = executable_program(&program)?.steps(admitted_run(b"ab", limits)?)?;

        match expect_step_transition(session.step())? {
            BorrowedStepTransition::Applied(applied) => {
                ensure_eq!(runtime_view_bytes(applied.state()).as_slice(), expected)?;
            }
            BorrowedStepTransition::Stable(_)
            | BorrowedStepTransition::Returned(_)
            | BorrowedStepTransition::Failed(_) => {
                return Err(TestFailure::message("expected one empty-payload rewrite"));
            }
        }
    }

    Ok(())
}
