use super::input::InitialStateBytes;
use super::matcher::RuleSearch;
use super::state::State;
use crate::bytes::{CompactByte, Payload, ProgramByte, RuntimeByte};
use crate::error::{LimitError, PayloadKind, RunError, RuntimeInputError, StateLimitContext};
use crate::limits::{
    ReturnByteLimit, ReturnOutputByteCount, RuntimeStateByteCount, StateByteLimit, StepCount,
    StepLimit,
};
use crate::test_support::{
    TestFailure, TestResult, ensure, ensure_eq, ensure_matches, source_column, source_line_number,
};
use crate::trace::RuntimeStateView;
use crate::{
    ExecutionStepError, ExecutionTransition, Program, ProgramSource, RunLimits, RuntimeInput,
};
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
        RunError::Allocation(_) | RunError::StateSize(_) | RunError::Limit(_) => {
            Err(TestFailure::message("expected step limit error"))
        }
    }
}

fn expect_step_error<'program>(
    result: Result<ExecutionTransition<'program>, ExecutionStepError<'program>>,
) -> Result<ExecutionStepError<'program>, TestFailure> {
    match result {
        Ok(_) => Err(TestFailure::message("expected step error")),
        Err(error) => Ok(error),
    }
}

fn expect_step_transition<'program>(
    result: Result<ExecutionTransition<'program>, ExecutionStepError<'program>>,
) -> Result<ExecutionTransition<'program>, TestFailure> {
    match result {
        Ok(transition) => Ok(transition),
        Err(error) => Err(TestFailure::from(error.into_error())),
    }
}

#[test]
fn once_rule_lookup_does_not_consume_before_step_commit() -> TestResult {
    let program = Program::parse(ProgramSource::from_str("(once)a=b"))?;
    let input = RuntimeInput::validate(b"a")?;
    let mut runtime = program.start_execution(
        &input,
        RunLimits::new(
            StepLimit::new(1),
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        ),
    )?;

    ensure(
        matches!(runtime.find_next_match(), RuleSearch::Matched(_)),
        "expected first lookup to find the once rule",
    )?;
    ensure(
        matches!(runtime.find_next_match(), RuleSearch::Matched(_)),
        "lookup must not consume a once rule before the step commits",
    )
}

#[test]
fn execution_step_limit_failure_preserves_uncommitted_state() -> TestResult {
    let program = Program::parse(ProgramSource::from_str("a=b"))?;
    let no_match_input = RuntimeInput::validate(b"x")?;
    let no_match = program.start_execution(
        &no_match_input,
        RunLimits::new(
            StepLimit::new(0),
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        ),
    )?;
    match expect_step_transition(no_match.step())? {
        ExecutionTransition::Stable(stable) => {
            ensure_eq!(stable.steps().get(), 0)?;
            ensure_eq!(
                runtime_view_bytes(stable.state()).as_slice(),
                b"x".as_slice()
            )?;
        }
        ExecutionTransition::Applied(_) | ExecutionTransition::Returned(_) => {
            return Err(TestFailure::message("expected stable completion"));
        }
    }

    let would_match_input = RuntimeInput::validate(b"a")?;
    let would_match = program.start_execution(
        &would_match_input,
        RunLimits::new(
            StepLimit::new(0),
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        ),
    )?;
    let error = expect_step_error(would_match.step())?;
    ensure_eq!(
        expect_step_limit(error.into_error())?,
        LimitError::Step {
            max_steps: StepLimit::new(0),
            completed_steps: StepCount::ZERO,
            state_len: RuntimeStateByteCount::new(1),
        },
    )?;
    let would_match = program.start_execution(
        &would_match_input,
        RunLimits::new(
            StepLimit::new(0),
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        ),
    )?;
    let error = expect_step_error(would_match.step())?;
    ensure_eq!(error.execution().completed_steps(), StepCount::ZERO)?;
    ensure_eq!(
        runtime_view_bytes(error.execution().state()).as_slice(),
        b"a".as_slice(),
    )?;

    let repeated_error =
        expect_step_limit(expect_step_error(error.into_execution().step())?.into_error())?;
    ensure_eq!(
        repeated_error,
        LimitError::Step {
            max_steps: StepLimit::new(0),
            completed_steps: StepCount::ZERO,
            state_len: RuntimeStateByteCount::new(1),
        },
    )
}

#[test]
fn execution_size_limit_failures_preserve_uncommitted_state() -> TestResult {
    let state_limits = RunLimits::new(
        StepLimit::new(1),
        StateByteLimit::new(2),
        ReturnByteLimit::new(10),
    );
    let state_program = Program::parse(ProgramSource::from_str("=a"))?;
    let state_input = RuntimeInput::validate(b"aa")?;
    let state_limited = state_program.start_execution(&state_input, state_limits)?;
    let state_error = expect_step_error(state_limited.step())?;
    ensure_eq!(
        state_error.error(),
        &RunError::Limit(LimitError::State {
            context: StateLimitContext::Rewrite,
            limit: StateByteLimit::new(2),
            attempted_len: RuntimeStateByteCount::new(3),
        }),
    )?;
    ensure_eq!(state_error.execution().completed_steps(), StepCount::ZERO)?;
    ensure_eq!(
        runtime_view_bytes(state_error.execution().state()).as_slice(),
        b"aa".as_slice(),
    )?;
    let state_error = expect_step_error(state_error.into_execution().step())?;
    ensure_eq!(
        state_error.into_error(),
        RunError::Limit(LimitError::State {
            context: StateLimitContext::Rewrite,
            limit: StateByteLimit::new(2),
            attempted_len: RuntimeStateByteCount::new(3),
        }),
    )?;

    let return_limits = RunLimits::new(
        StepLimit::new(1),
        StateByteLimit::new(10),
        ReturnByteLimit::new(1),
    );
    let return_program = Program::parse(ProgramSource::from_str("a=(return)ok"))?;
    let return_input = RuntimeInput::validate(b"a")?;
    let return_limited = return_program.start_execution(&return_input, return_limits)?;
    let return_error = expect_step_error(return_limited.step())?;
    ensure_eq!(
        return_error.error(),
        &RunError::Limit(LimitError::Return {
            limit: ReturnByteLimit::new(1),
            attempted_len: ReturnOutputByteCount::new(2),
        }),
    )?;
    ensure_eq!(return_error.execution().completed_steps(), StepCount::ZERO)?;
    ensure_eq!(
        runtime_view_bytes(return_error.execution().state()).as_slice(),
        b"a".as_slice(),
    )?;
    let return_error = expect_step_error(return_error.into_execution().step())?;
    ensure_eq!(
        return_error.into_error(),
        RunError::Limit(LimitError::Return {
            limit: ReturnByteLimit::new(1),
            attempted_len: ReturnOutputByteCount::new(2),
        }),
    )
}

#[test]
fn runtime_input_error_is_structured_at_the_runtime_boundary() -> TestResult {
    let Err(error) = RuntimeInput::validate("a\u{80}".as_bytes()) else {
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

#[test]
fn internal_code_and_runtime_bytes_are_distinct_domains() -> TestResult {
    let compact = [CompactByte::new(b'a', source_column(1)?)];
    let payload = Payload::parse(&compact, source_line_number(1)?, PayloadKind::LeftSideData)?;
    let input = RuntimeInput::validate(b"a=()# ")?;
    let state = State::from_input(InitialStateBytes::materialize(
        &input,
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
