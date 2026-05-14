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
    TestFailure, TestResult, ensure, ensure_eq, ensure_matches, expect_return_output,
    expect_run_error, expect_step_limit, into_result_bytes, result_bytes, run_program, run_source,
    runtime_input, source_column, source_line_number, test_limits,
};
use crate::trace::{BorrowedTraceEffect, BorrowedTraceEvent, RuntimeStateView};
use crate::{Program, ReturnOutput, RunLimits, RunOutcome, RunResult, RuntimeStateSnapshot};
use std::string::String;
use std::vec::Vec;

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

fn runtime_view_bytes(view: RuntimeStateView<'_>) -> Vec<u8> {
    view.bytes().collect()
}

fn run_program_by_steps(
    program: &Program,
    input: &[u8],
    limits: RunLimits,
) -> Result<RunResult, TestFailure> {
    let mut execution = program.start_execution(runtime_input(input)?, limits)?;

    loop {
        match execution.step()? {
            ExecutionStep::Applied { .. } => {}
            ExecutionStep::Stable { steps, state } => {
                return Ok(RunResult::stable(
                    RuntimeStateSnapshot::from_vec(state.to_vec()?),
                    steps,
                ));
            }
            ExecutionStep::Return { step, output, .. } => {
                return Ok(RunResult::from_return(
                    ReturnOutput::from_vec(output.to_vec()?),
                    step,
                ));
            }
        }
    }
}

fn expect_applied_step(
    step: ExecutionStep<'_, '_>,
    expected_step: usize,
    expected_rule: &[u8],
    expected_state: &[u8],
) -> TestResult {
    match step {
        ExecutionStep::Applied { step, rule, state } => {
            ensure_eq!(step.get(), expected_step)?;
            ensure_eq!(rule.canonical_source()?.as_slice(), expected_rule)?;
            let actual_state = runtime_view_bytes(state);
            ensure_eq!(actual_state.as_slice(), expected_state)?;
            ensure_eq!(
                state.byte_count(),
                RuntimeStateByteCount::new(expected_state.len())
            )?;
            ensure_eq!(state.is_empty(), expected_state.is_empty())?;
            Ok(())
        }
        ExecutionStep::Stable { .. } | ExecutionStep::Return { .. } => {
            Err(TestFailure::message("expected applied step"))
        }
    }
}

fn expect_stable_completion(
    step: ExecutionStep<'_, '_>,
    expected_steps: usize,
    expected_state: &[u8],
) -> TestResult {
    match step {
        ExecutionStep::Stable { steps, state } => {
            ensure_eq!(steps.get(), expected_steps)?;
            ensure_eq!(runtime_view_bytes(state).as_slice(), expected_state)?;
            Ok(())
        }
        ExecutionStep::Applied { .. } | ExecutionStep::Return { .. } => {
            Err(TestFailure::message("expected stable completion"))
        }
    }
}

fn expect_return_completion(
    step: ExecutionStep<'_, '_>,
    expected_step: usize,
    expected_rule: &[u8],
    expected_output: &[u8],
) -> TestResult {
    match step {
        ExecutionStep::Return { step, rule, output } => {
            ensure_eq!(step.get(), expected_step)?;
            ensure_eq!(rule.canonical_source()?.as_slice(), expected_rule)?;
            ensure(
                output.eq_bytes(expected_output),
                "expected return completion output",
            )?;
            Ok(())
        }
        ExecutionStep::Applied { .. } | ExecutionStep::Stable { .. } => {
            Err(TestFailure::message("expected return completion"))
        }
    }
}

#[test]
fn normal_replacement_is_ordered_and_leftmost() -> TestResult {
    let source = "aa=x\na=y";
    ensure_eq!(run_source(source, "aaaa")?, "xx")?;
    Ok(())
}

#[test]
fn execution_step_applies_one_rule_and_waits() -> TestResult {
    let program = Program::parse(crate::ProgramSource::from_str("a=b\nb=c"))?;
    let mut execution = program.start_execution(
        runtime_input(b"a")?,
        RunLimits::new(
            StepLimit::new(10),
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        ),
    )?;

    ensure_eq!(execution.completed_steps(), StepCount::ZERO)?;

    expect_applied_step(execution.step()?, 1, b"a=b", b"b")?;
    ensure_eq!(execution.completed_steps().get(), 1)?;

    expect_applied_step(execution.step()?, 2, b"b=c", b"c")?;
    ensure_eq!(execution.completed_steps().get(), 2)?;

    expect_stable_completion(execution.step()?, 2, b"c")?;
    expect_stable_completion(execution.step()?, 2, b"c")?;
    Ok(())
}

fn expect_step_loop_matches_full_run(source: &str, input: &[u8]) -> TestResult {
    let limits = RunLimits::new(
        StepLimit::new(10),
        crate::DEFAULT_MAX_STATE_LEN,
        crate::DEFAULT_MAX_RETURN_LEN,
    );
    let program = Program::parse(crate::ProgramSource::from_str(source))?;
    let full_run = program.run(runtime_input(input)?, limits)?;
    let stepped_run = run_program_by_steps(&program, input, limits)?;
    ensure_eq!(stepped_run, full_run)?;
    Ok(())
}

#[test]
fn step_loop_matches_full_run_for_rewrite_and_once_rules() -> TestResult {
    expect_step_loop_matches_full_run("a=b\nb=c", b"a")?;
    expect_step_loop_matches_full_run("(once)a=b\na=c", b"aa")
}

#[test]
fn step_loop_matches_full_run_for_anchor_and_delete_rules() -> TestResult {
    expect_step_loop_matches_full_run("(start)a=x", b"aba")?;
    expect_step_loop_matches_full_run("(end)a=x", b"aba")?;
    expect_step_loop_matches_full_run("a=", b"aa")
}

#[test]
fn step_loop_matches_full_run_for_return_and_stable_rules() -> TestResult {
    expect_step_loop_matches_full_run("=(return)empty", b"")?;
    expect_step_loop_matches_full_run("a=(return)ok", b"a")?;
    expect_step_loop_matches_full_run("x=y", b"a")
}

#[test]
fn execution_finish_resumes_after_manual_steps() -> TestResult {
    let program = Program::parse(crate::ProgramSource::from_str(
        "(once)a=b\na=c\nc=(return)ok",
    ))?;
    let limits = RunLimits::new(
        StepLimit::new(10),
        crate::DEFAULT_MAX_STATE_LEN,
        crate::DEFAULT_MAX_RETURN_LEN,
    );
    let full_run = program.run(runtime_input(b"aa")?, limits)?;
    let mut execution = program.start_execution(runtime_input(b"aa")?, limits)?;

    expect_applied_step(execution.step()?, 1, b"(once)a=b", b"ba")?;
    let resumed = execution.finish()?;

    ensure_matches(
        matches!(resumed.outcome(), RunOutcome::Return(output) if output.as_bytes() == b"ok"),
        "expected resumed execution to return ok",
    )?;
    ensure_eq!(resumed, full_run)?;
    Ok(())
}

#[test]
fn execution_finish_after_stable_step_returns_stable_result() -> TestResult {
    let program = Program::parse(crate::ProgramSource::from_str("x=y"))?;
    let limits = RunLimits::new(
        StepLimit::new(10),
        crate::DEFAULT_MAX_STATE_LEN,
        crate::DEFAULT_MAX_RETURN_LEN,
    );
    let mut execution = program.start_execution(runtime_input(b"a")?, limits)?;

    expect_stable_completion(execution.step()?, 0, b"a")?;
    ensure_eq!(execution.completed_steps(), StepCount::ZERO)?;

    let finished = execution.finish()?;
    ensure_eq!(finished.steps(), StepCount::ZERO)?;
    ensure_matches(
        matches!(finished.outcome(), RunOutcome::Stable(state) if state.as_bytes() == b"a"),
        "expected stable finish output",
    )?;
    Ok(())
}

#[test]
fn execution_finish_after_return_step_preserves_return_result() -> TestResult {
    let program = Program::parse(crate::ProgramSource::from_str("a=(return)ok"))?;
    let limits = RunLimits::new(
        StepLimit::new(10),
        crate::DEFAULT_MAX_STATE_LEN,
        crate::DEFAULT_MAX_RETURN_LEN,
    );
    let mut execution = program.start_execution(runtime_input(b"a")?, limits)?;

    expect_return_completion(execution.step()?, 1, b"a=(return)ok", b"ok")?;
    ensure_eq!(execution.completed_steps().get(), 1)?;

    let finished = execution.finish()?;
    expect_return_output(&finished, b"ok")?;
    ensure_eq!(finished.steps().get(), 1)?;
    Ok(())
}

#[test]
fn execution_step_uses_the_same_once_state_as_full_run() -> TestResult {
    let program = Program::parse(crate::ProgramSource::from_str("(once)a=b\na=c"))?;
    let limits = RunLimits::new(
        StepLimit::new(10),
        crate::DEFAULT_MAX_STATE_LEN,
        crate::DEFAULT_MAX_RETURN_LEN,
    );
    let full_run = program.run(runtime_input(b"aa")?, limits)?;
    let mut execution = program.start_execution(runtime_input(b"aa")?, limits)?;

    expect_applied_step(execution.step()?, 1, b"(once)a=b", b"ba")?;
    expect_applied_step(execution.step()?, 2, b"a=c", b"bc")?;
    expect_stable_completion(
        execution.step()?,
        full_run.steps().get(),
        result_bytes(&full_run),
    )?;
    Ok(())
}

#[test]
fn execution_step_return_completes_without_continuation() -> TestResult {
    let program = Program::parse(crate::ProgramSource::from_str("a=(return)ok\na=b"))?;
    let mut execution = program.start_execution(
        runtime_input(b"a")?,
        RunLimits::new(
            StepLimit::new(10),
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        ),
    )?;

    expect_return_completion(execution.step()?, 1, b"a=(return)ok", b"ok")?;
    expect_return_completion(execution.step()?, 1, b"a=(return)ok", b"ok")?;
    Ok(())
}

#[test]
fn execution_step_preserves_step_limit_boundary() -> TestResult {
    let program = Program::parse(crate::ProgramSource::from_str("a=b"))?;
    let mut no_match = program.start_execution(
        runtime_input(b"x")?,
        RunLimits::new(
            StepLimit::new(0),
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        ),
    )?;
    expect_stable_completion(no_match.step()?, 0, b"x")?;

    let mut would_match = program.start_execution(
        runtime_input(b"a")?,
        RunLimits::new(
            StepLimit::new(0),
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        ),
    )?;
    let error = expect_run_error(would_match.step())?;
    let error = expect_step_limit(error)?;

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
        b"a"
    )?;

    let repeated_error = expect_step_limit(expect_run_error(would_match.step())?)?;
    ensure_eq!(
        repeated_error,
        LimitError::Step {
            max_steps: StepLimit::new(0),
            completed_steps: StepCount::ZERO,
            state_len: RuntimeStateByteCount::new(1),
        },
    )?;
    Ok(())
}

#[test]
fn step_limit_preempts_rewrite_and_return_size_checks() -> TestResult {
    let rewrite_program = Program::parse(crate::ProgramSource::from_str("=a"))?;
    let rewrite_limits = RunLimits::new(
        StepLimit::new(0),
        StateByteLimit::new(0),
        ReturnByteLimit::new(0),
    );
    let rewrite_error = expect_step_limit(expect_run_error(
        rewrite_program.run(runtime_input(b"")?, rewrite_limits),
    )?)?;
    ensure_eq!(
        rewrite_error,
        LimitError::Step {
            max_steps: StepLimit::new(0),
            completed_steps: StepCount::ZERO,
            state_len: RuntimeStateByteCount::new(0),
        },
    )?;

    let return_program = Program::parse(crate::ProgramSource::from_str("=(return)a"))?;
    let return_limits = RunLimits::new(
        StepLimit::new(0),
        StateByteLimit::new(0),
        ReturnByteLimit::new(0),
    );
    let return_error = expect_step_limit(expect_run_error(
        return_program.run(runtime_input(b"")?, return_limits),
    )?)?;
    ensure_eq!(
        return_error,
        LimitError::Step {
            max_steps: StepLimit::new(0),
            completed_steps: StepCount::ZERO,
            state_len: RuntimeStateByteCount::new(0),
        },
    )?;
    Ok(())
}

#[test]
fn execution_step_preserves_byte_limit_boundaries() -> TestResult {
    let state_limits = RunLimits::new(
        StepLimit::new(1),
        StateByteLimit::new(2),
        ReturnByteLimit::new(10),
    );
    let state_program = Program::parse(crate::ProgramSource::from_str("=a"))?;
    let mut state_limited = state_program.start_execution(runtime_input(b"aa")?, state_limits)?;
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
        b"aa"
    )?;

    let repeated_state_error = expect_run_error(state_limited.step())?;
    ensure_eq!(
        repeated_state_error,
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
    let return_program = Program::parse(crate::ProgramSource::from_str("a=(return)ok"))?;
    let mut return_limited = return_program.start_execution(runtime_input(b"a")?, return_limits)?;
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
        b"a"
    )?;

    let repeated_return_error = expect_run_error(return_limited.step())?;
    ensure_eq!(
        repeated_return_error,
        RunError::Limit(LimitError::Return {
            limit: ReturnByteLimit::new(1),
            attempted_len: ReturnOutputByteCount::new(2),
        }),
    )?;
    Ok(())
}

#[test]
fn anchors_match_only_at_their_edges() -> TestResult {
    ensure_eq!(run_source("(start)a=x", "aba")?, "xba")?;
    ensure_eq!(run_source("(start)a=x", "ba")?, "ba")?;
    ensure_eq!(run_source("(end)a=x", "aba")?, "abx")?;
    ensure_eq!(run_source("(end)a=x", "ab")?, "ab")?;
    Ok(())
}

#[test]
fn move_actions_work() -> TestResult {
    ensure_eq!(run_source("a=(start)x", "ba")?, "xb")?;
    ensure_eq!(run_source("a=(end)x", "ba")?, "bx")?;
    Ok(())
}

#[test]
fn empty_lhs_anywhere_matches_at_start() -> TestResult {
    let source = "(once)=x\n(start)x=(return)ok";
    let result = run_program(
        &Program::parse(crate::ProgramSource::from_str(source))?,
        b"ab",
        RunLimits::new(
            StepLimit::new(2),
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        ),
    )?;

    expect_return_output(&result, b"ok")?;
    ensure_eq!(result.steps().get(), 2)?;
    Ok(())
}

#[test]
fn empty_lhs_start_and_end_anchors_pick_different_edges() -> TestResult {
    let limits = RunLimits::new(
        StepLimit::new(2),
        crate::DEFAULT_MAX_STATE_LEN,
        crate::DEFAULT_MAX_RETURN_LEN,
    );
    let start_result = run_program(
        &Program::parse(crate::ProgramSource::from_str(
            "(once)(start)=x\nxab=(return)start",
        ))?,
        b"ab",
        limits,
    )?;
    let end_result = run_program(
        &Program::parse(crate::ProgramSource::from_str(
            "(once)(end)=x\nabx=(return)end",
        ))?,
        b"ab",
        limits,
    )?;

    ensure_eq!(result_bytes(&start_result), b"start".as_slice())?;
    ensure_eq!(result_bytes(&end_result), b"end".as_slice())?;
    Ok(())
}

#[test]
fn once_rule_is_used_at_most_once() -> TestResult {
    let source = "(once)a=b\na=c";
    ensure_eq!(run_source(source, "aa")?, "bc")?;
    Ok(())
}

#[test]
fn once_rule_lookup_does_not_consume_before_step_commit() -> TestResult {
    let program = Program::parse(crate::ProgramSource::from_str("(once)a=b"))?;
    let runtime = Execution::new(
        &program,
        runtime_input(b"a")?,
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
    )?;
    Ok(())
}

#[test]
fn return_discards_current_state() -> TestResult {
    let source = "aa=(return)ok\na=x";
    ensure_eq!(run_source(source, "aabb")?, "ok")?;
    Ok(())
}

#[test]
fn runtime_only_bytes_are_preserved_until_return_discards_them() -> TestResult {
    ensure_eq!(run_source("a=b", "a=()#c")?, "b=()#c")?;
    let result = run_program(
        &Program::parse(crate::ProgramSource::from_str("a=(return)x"))?,
        b"a=()#c",
        RunLimits::new(
            StepLimit::new(1),
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        ),
    )?;
    expect_return_output(&result, b"x")?;
    Ok(())
}

#[test]
fn input_spaces_are_preserved_and_do_not_bridge_matches() -> TestResult {
    ensure_eq!(run_source("a= b", "a bc")?, "b bc")?;
    ensure_eq!(run_source("a b=bb", "a bc")?, "a bc")?;
    ensure_eq!(run_source("ab=bb", "a bc")?, "a bc")?;
    Ok(())
}

#[test]
fn opaque_reserved_input_bytes_do_not_bridge_program_payload_matches() -> TestResult {
    ensure_eq!(run_source("ab=x", "a=b")?, "a=b")?;
    ensure_eq!(run_source("ab=x", "a#b")?, "a#b")?;
    ensure_eq!(run_source("ab=x", "a(b")?, "a(b")?;
    ensure_eq!(run_source("ab=x", "a)b")?, "a)b")?;
    Ok(())
}

#[test]
fn runtime_input_error_is_structured() -> TestResult {
    let error = match runtime_input("aあ".as_bytes()) {
        Ok(_) => return Err(TestFailure::message("expected input error")),
        Err(TestFailure::Input(error)) => error,
        Err(error) => return Err(error),
    };

    ensure_matches(
        matches!(
            error,
            InputError::NonAscii { column, .. } if column.get() == 2
        ),
        "expected non-ASCII input error at the original column",
    )?;
    Ok(())
}

#[test]
fn runtime_state_can_hold_reserved_bytes_that_program_payloads_cannot_construct() -> TestResult {
    let program = Program::parse(crate::ProgramSource::from_str("a=b"))?;
    ensure(
        Program::parse(crate::ProgramSource::from_str("a=(return)(")).is_err(),
        "expected invalid return payload",
    )?;
    ensure(
        Program::parse(crate::ProgramSource::from_str("a=b)")).is_err(),
        "expected invalid payload",
    )?;

    let result = run_program(
        &program,
        b"a=#()",
        RunLimits::new(
            StepLimit::new(10_000),
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        ),
    )?;
    ensure_eq!(String::from_utf8(into_result_bytes(result))?, "b=#()")?;
    Ok(())
}

#[test]
fn step_limit_allows_exact_limit_but_blocks_next_match() -> TestResult {
    let exact = run_program(
        &Program::parse(crate::ProgramSource::from_str("a=b"))?,
        b"a",
        RunLimits::new(
            StepLimit::new(1),
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        ),
    )?;
    ensure_eq!(result_bytes(&exact), b"b".as_slice())?;
    ensure_eq!(exact.steps().get(), 1)?;

    let no_match = run_program(
        &Program::parse(crate::ProgramSource::from_str("a=b"))?,
        b"x",
        RunLimits::new(
            StepLimit::new(0),
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        ),
    )?;
    ensure_eq!(result_bytes(&no_match), b"x".as_slice())?;
    ensure_eq!(no_match.steps().get(), 0)?;

    let limits = RunLimits::new(
        StepLimit::new(0),
        crate::DEFAULT_MAX_STATE_LEN,
        crate::DEFAULT_MAX_RETURN_LEN,
    );
    let error = expect_run_error(
        Program::parse(crate::ProgramSource::from_str("a=b"))?.run(runtime_input(b"a")?, limits),
    )?;
    let error = expect_step_limit(error)?;
    ensure_eq!(
        error,
        LimitError::Step {
            max_steps: StepLimit::new(0),
            completed_steps: StepCount::ZERO,
            state_len: RuntimeStateByteCount::new(1),
        },
    )?;
    Ok(())
}

#[test]
fn step_limit_error_reports_state_len_without_owning_state_bytes() -> TestResult {
    let limits = RunLimits::new(
        StepLimit::new(3),
        crate::DEFAULT_MAX_STATE_LEN,
        crate::DEFAULT_MAX_RETURN_LEN,
    );
    let error = expect_run_error(
        Program::parse(crate::ProgramSource::from_str("=a"))?.run(runtime_input(b"")?, limits),
    )?;
    let error = expect_step_limit(error)?;

    ensure_eq!(
        error,
        LimitError::Step {
            max_steps: StepLimit::new(3),
            completed_steps: StepCount::ZERO
                .checked_next()
                .and_then(StepCount::checked_next)
                .and_then(StepCount::checked_next)
                .ok_or(TestFailure::message("expected step count"))?,
            state_len: RuntimeStateByteCount::new(3),
        },
    )?;
    Ok(())
}

#[test]
fn borrowed_trace_exposes_last_state_before_step_limit() -> TestResult {
    let program = Program::parse(crate::ProgramSource::from_str("=a"))?;
    let mut last_state = Vec::new();
    let limits = RunLimits::new(
        StepLimit::new(3),
        crate::DEFAULT_MAX_STATE_LEN,
        crate::DEFAULT_MAX_RETURN_LEN,
    );

    let error =
        expect_run_error(
            program.run_with_borrowed_trace(runtime_input(b"")?, limits, |event| {
                last_state.clear();
                match event {
                    BorrowedTraceEvent::Initial { state }
                    | BorrowedTraceEvent::Step {
                        effect: BorrowedTraceEffect::Continue { state },
                        ..
                    } => last_state.extend(state.bytes()),
                    BorrowedTraceEvent::Step {
                        effect: BorrowedTraceEffect::Return { output },
                        ..
                    } => last_state.extend(output.bytes()),
                }
            }),
        )?;
    let error = expect_step_limit(error)?;

    ensure_eq!(
        error,
        LimitError::Step {
            max_steps: StepLimit::new(3),
            completed_steps: StepCount::ZERO
                .checked_next()
                .and_then(StepCount::checked_next)
                .and_then(StepCount::checked_next)
                .ok_or(TestFailure::message("expected step count"))?,
            state_len: RuntimeStateByteCount::new(3),
        },
    )?;
    ensure_eq!(last_state.as_slice(), b"aaa".as_slice())?;
    Ok(())
}

#[test]
fn palindrome_example_returns_true_or_false() -> TestResult {
    let source = "\
b=a|a|
c=a|aa|
a|-=
--=(return)false
(start)a|=(end)-
(start)a=(end)|-
=(return)true";

    ensure_eq!(run_source(source, "aba")?, "true")?;
    ensure_eq!(run_source(source, "ab")?, "false")?;
    Ok(())
}

#[test]
fn runtime_accepts_every_ascii_input_byte() -> TestResult {
    let input: Vec<u8> = (0x00..=0x7f).collect();
    let result = run_program(
        &Program::parse(crate::ProgramSource::from_str("# no executable rules"))?,
        &input,
        RunLimits::new(
            crate::DEFAULT_MAX_STEPS,
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        ),
    )?;

    ensure_eq!(result_bytes(&result), input.as_slice())?;
    ensure_eq!(result.steps().get(), 0)?;
    Ok(())
}

#[test]
fn runtime_rejects_every_non_ascii_input_byte() -> TestResult {
    for byte in 0x80..=0xff {
        ensure(runtime_input(&[byte]).is_err(), "byte should be rejected")?;
    }

    Ok(())
}

#[test]
fn internal_code_and_runtime_bytes_are_distinct_domains() -> TestResult {
    let compact = [CompactByte::new(b'a', source_column(1)?)];
    let payload = Payload::parse(&compact, source_line_number(1)?, PayloadKind::LeftSideData)?;
    let state = State::from_input(InitialStateBytes::materialize(
        runtime_input(b"a=()# ")?,
        test_limits(),
    )?);

    ensure_eq!(expect_payload_byte(&payload, 0)?, b'a')?;
    ensure_eq!(expect_runtime_byte(&state, 0)?, b'a')?;
    ensure_eq!(expect_runtime_byte(&state, 1)?, b'=')?;
    ensure_eq!(expect_runtime_byte(&state, 2)?, b'(')?;
    ensure_eq!(expect_runtime_byte(&state, 5)?, b' ')?;
    ensure_eq!(expect_program_constructible_byte(&state, 0)?, b'a')?;
    ensure_eq!(expect_opaque_runtime_byte(&state, 1)?, b'=')?;
    ensure_eq!(expect_opaque_runtime_byte(&state, 2)?, b'(')?;
    ensure_eq!(expect_opaque_runtime_byte(&state, 5)?, b' ')?;
    Ok(())
}
