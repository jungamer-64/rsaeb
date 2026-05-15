use super::input::InitialStateBytes;
use super::matcher::RuleSearch;
use super::state::State;
use super::*;
use crate::bytes::{CompactByte, Payload, ProgramByte, RuntimeByte};
use crate::error::{InputError, LimitError, PayloadKind, RunError, StateLimitContext};
use crate::limits::{
    ReturnByteLimit, ReturnOutputByteCount, RuntimeStateByteCount, StateByteLimit, StepCount,
    StepLimit,
};
use crate::test_support::{
    TestFailure, TestResult, ensure, ensure_eq, ensure_matches, expect_run_error, source_column,
    source_line_number,
};
use crate::trace::RuntimeStateView;
use crate::{Program, ProgramSource, RunLimits, RuntimeInput};
use std::vec::Vec;

fn runtime_view_bytes(view: RuntimeStateView<'_>) -> Vec<u8> {
    view.bytes().collect()
}

fn expect_runtime_byte(state: &State, index: usize) -> Result<u8, TestFailure> {
    state
        .materialized_byte_at(index)
        .ok_or(TestFailure::message("expected runtime byte"))
}

fn expect_program_constructible_byte(state: &State, index: usize) -> Result<u8, TestFailure> {
    match state.bytes.get(index).copied() {
        Some(RuntimeByte::ProgramConstructible(byte)) => Ok(byte.get()),
        Some(RuntimeByte::Opaque(_)) => {
            Err(TestFailure::message("expected program-constructible byte"))
        }
        None => Err(TestFailure::message("expected runtime byte")),
    }
}

fn expect_opaque_runtime_byte(state: &State, index: usize) -> Result<u8, TestFailure> {
    match state.bytes.get(index).copied() {
        Some(RuntimeByte::Opaque(byte)) => Ok(byte.materialize()),
        Some(RuntimeByte::ProgramConstructible(_)) => {
            Err(TestFailure::message("expected opaque runtime byte"))
        }
        None => Err(TestFailure::message("expected runtime byte")),
    }
}

fn expect_payload_byte(payload: &Payload, index: usize) -> Result<u8, TestFailure> {
    payload
        .program_bytes()
        .get(index)
        .copied()
        .map(ProgramByte::get)
        .ok_or(TestFailure::message("expected payload byte"))
}

fn expect_step_limit(error: RunError) -> Result<LimitError, TestFailure> {
    match error {
        RunError::Limit(error @ LimitError::Step { .. }) => Ok(error),
        RunError::Allocation(_)
        | RunError::StateSize(_)
        | RunError::Limit(_)
        | RunError::Invariant(_) => Err(TestFailure::message("expected step limit error")),
    }
}

#[test]
fn once_rule_lookup_does_not_consume_before_step_commit() -> TestResult {
    let program = Program::parse(ProgramSource::from_str("(once)a=b"))?;
    let runtime = Execution::new(
        &program,
        RuntimeInput::validate(b"a")?,
        RunLimits::new(
            StepLimit::new(1),
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        ),
    )?;

    ensure(
        matches!(runtime.find_next_match()?, RuleSearch::Matched(_)),
        "expected first lookup to find the once rule",
    )?;
    ensure(
        matches!(runtime.find_next_match()?, RuleSearch::Matched(_)),
        "lookup must not consume a once rule before the step commits",
    )
}

#[test]
fn execution_step_limit_failure_preserves_uncommitted_state() -> TestResult {
    let program = Program::parse(ProgramSource::from_str("a=b"))?;
    let mut no_match = program.start_execution(
        RuntimeInput::validate(b"x")?,
        RunLimits::new(
            StepLimit::new(0),
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        ),
    )?;
    match no_match.step()? {
        ExecutionStep::Stable { steps, state } => {
            ensure_eq!(steps.get(), 0)?;
            ensure_eq!(runtime_view_bytes(state).as_slice(), b"x".as_slice())?;
        }
        ExecutionStep::Applied { .. } | ExecutionStep::Return { .. } => {
            return Err(TestFailure::message("expected stable completion"));
        }
    }

    let mut would_match = program.start_execution(
        RuntimeInput::validate(b"a")?,
        RunLimits::new(
            StepLimit::new(0),
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        ),
    )?;
    let error = expect_step_limit(expect_run_error(would_match.step())?)?;

    ensure_eq!(
        error,
        LimitError::Step {
            max_steps: StepLimit::new(0),
            completed_steps: StepCount::ZERO,
            state_len: RuntimeStateByteCount::new(1),
        },
    )?;
    ensure_eq!(would_match.completed_steps(), StepCount::ZERO)?;
    ensure_eq!(
        runtime_view_bytes(would_match.state.view()).as_slice(),
        b"a".as_slice(),
    )?;

    let repeated_error = expect_step_limit(expect_run_error(would_match.step())?)?;
    ensure_eq!(repeated_error, error)
}

#[test]
fn execution_size_limit_failures_preserve_uncommitted_state() -> TestResult {
    let state_limits = RunLimits::new(
        StepLimit::new(1),
        StateByteLimit::new(2),
        ReturnByteLimit::new(10),
    );
    let state_program = Program::parse(ProgramSource::from_str("=a"))?;
    let mut state_limited =
        state_program.start_execution(RuntimeInput::validate(b"aa")?, state_limits)?;
    let state_error = expect_run_error(state_limited.step())?;
    ensure_eq!(
        state_error,
        RunError::Limit(LimitError::State {
            context: StateLimitContext::Rewrite,
            limit: StateByteLimit::new(2),
            attempted_len: RuntimeStateByteCount::new(3),
        }),
    )?;
    ensure_eq!(state_limited.completed_steps(), StepCount::ZERO)?;
    ensure_eq!(
        runtime_view_bytes(state_limited.state.view()).as_slice(),
        b"aa".as_slice(),
    )?;
    ensure_eq!(expect_run_error(state_limited.step())?, state_error)?;

    let return_limits = RunLimits::new(
        StepLimit::new(1),
        StateByteLimit::new(10),
        ReturnByteLimit::new(1),
    );
    let return_program = Program::parse(ProgramSource::from_str("a=(return)ok"))?;
    let mut return_limited =
        return_program.start_execution(RuntimeInput::validate(b"a")?, return_limits)?;
    let return_error = expect_run_error(return_limited.step())?;
    ensure_eq!(
        return_error,
        RunError::Limit(LimitError::Return {
            limit: ReturnByteLimit::new(1),
            attempted_len: ReturnOutputByteCount::new(2),
        }),
    )?;
    ensure_eq!(return_limited.completed_steps(), StepCount::ZERO)?;
    ensure_eq!(
        runtime_view_bytes(return_limited.state.view()).as_slice(),
        b"a".as_slice(),
    )?;
    ensure_eq!(expect_run_error(return_limited.step())?, return_error)
}

#[test]
fn runtime_input_error_is_structured_at_the_runtime_boundary() -> TestResult {
    let Err(error) = RuntimeInput::validate("a\u{80}".as_bytes()) else {
        return Err(TestFailure::message("expected input error"));
    };

    ensure_matches(
        matches!(
            error,
            InputError::NonAscii { column, .. } if column.get() == 2
        ),
        "expected non-ASCII input error at the original column",
    )
}

#[test]
fn internal_code_and_runtime_bytes_are_distinct_domains() -> TestResult {
    let compact = [CompactByte::new(b'a', source_column(1)?)];
    let payload = Payload::parse(&compact, source_line_number(1)?, PayloadKind::LeftSideData)?;
    let state = State::from_input(InitialStateBytes::materialize(
        RuntimeInput::validate(b"a=()# ")?,
        RunLimits::new(
            StepLimit::new(10_000),
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        ),
    )?);

    ensure_eq!(expect_payload_byte(&payload, 0)?, b'a')?;
    ensure_eq!(expect_runtime_byte(&state, 0)?, b'a')?;
    ensure_eq!(expect_runtime_byte(&state, 1)?, b'=')?;
    ensure_eq!(expect_runtime_byte(&state, 2)?, b'(')?;
    ensure_eq!(expect_runtime_byte(&state, 5)?, b' ')?;
    ensure_eq!(expect_program_constructible_byte(&state, 0)?, b'a')?;
    ensure_eq!(expect_opaque_runtime_byte(&state, 1)?, b'=')?;
    ensure_eq!(expect_opaque_runtime_byte(&state, 2)?, b'(')?;
    ensure_eq!(expect_opaque_runtime_byte(&state, 5)?, b' ')
}
