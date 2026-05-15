mod support;

use rsaeb::error::{
    FallibleTraceSnapshotRunError, RunError, TraceSnapshotError, TraceSnapshotRunError,
};
use rsaeb::limits::{
    DEFAULT_MAX_TRACE_SNAPSHOT_LEN, StepLimit, TraceSnapshotByteLimit, TraceSnapshotLimits,
};
use rsaeb::trace::{
    BorrowedTraceEffect, BorrowedTraceEvent, TraceSnapshotEffect, TraceSnapshotEvent,
};
use rsaeb::{
    DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, Program, ProgramSource, RunLimits, RunOutcome,
};
use support::{TestFailure, TestResult, ensure_eq, ensure_matches, runtime_input};

/// Returns the expected trace snapshot run error.
///
/// # Errors
///
/// Returns `TestFailure` if the traced run succeeds.
fn expect_trace_snapshot_error<T>(
    result: Result<T, TraceSnapshotRunError>,
) -> Result<TraceSnapshotRunError, TestFailure> {
    match result {
        Ok(_) => Err(TestFailure::message("expected trace snapshot error")),
        Err(error) => Ok(error),
    }
}

/// Runs a standard trace snapshot example and returns its result and events.
///
/// # Errors
///
/// Returns `TestFailure` if parsing, input validation, runtime execution, or
/// trace snapshot materialization fails.
fn trace_snapshot_example(
    program: &Program,
) -> Result<(rsaeb::RunResult, Vec<TraceSnapshotEvent<'_>>), TestFailure> {
    let mut events = Vec::new();
    let limits = TraceSnapshotLimits::new(
        RunLimits::new(
            StepLimit::new(10_000),
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        ),
        DEFAULT_MAX_TRACE_SNAPSHOT_LEN,
    );
    let result = program.run_with_trace_snapshots(&runtime_input(b"a")?, limits, |event| {
        events.push(event);
    })?;

    Ok((result, events))
}

fn snapshot_event_bytes<'event>(event: &'event TraceSnapshotEvent<'_>) -> &'event [u8] {
    match event {
        TraceSnapshotEvent::Initial { state } => state.as_bytes(),
        TraceSnapshotEvent::Step { effect, .. } => match effect {
            TraceSnapshotEffect::Continue { state } => state.as_bytes(),
            TraceSnapshotEffect::Return { output } => output.as_bytes(),
        },
    }
}

/// # Errors
///
/// Returns `TestFailure` if borrowed trace events allocate snapshots or expose
/// incorrect byte data.
#[test]
fn borrowed_trace_events_are_emitted_without_snapshots() -> TestResult {
    let program = Program::parse(ProgramSource::from_str("a=b\nb=(return)ok"))?;
    let mut seen = Vec::new();
    let limits = RunLimits::new(
        StepLimit::new(10_000),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );

    let result = program.run_with_borrowed_trace(&runtime_input(b"a")?, limits, |event| {
        let bytes = match event {
            BorrowedTraceEvent::Initial { state }
            | BorrowedTraceEvent::Step {
                effect: BorrowedTraceEffect::Continue { state },
                ..
            } => state.bytes().collect::<Vec<_>>(),
            BorrowedTraceEvent::Step {
                effect: BorrowedTraceEffect::Return { output },
                ..
            } => output.bytes().collect::<Vec<_>>(),
        };
        seen.push((event.byte_count().get(), bytes));
    })?;

    ensure_matches(
        matches!(result.outcome(), RunOutcome::Return(output) if output.as_bytes() == b"ok"),
        "expected return output",
    )?;
    ensure_eq!(
        seen.as_slice(),
        &[(1, b"a".to_vec()), (1, b"b".to_vec()), (2, b"ok".to_vec())],
    )
}

/// # Errors
///
/// Returns `TestFailure` if trace snapshot events lose materialized bytes or
/// structured effects.
#[test]
fn trace_snapshot_events_carry_bytes_and_structured_effects() -> TestResult {
    let program = Program::parse(ProgramSource::from_str("a=b\nb=(return)ok"))?;
    let (result, events) = trace_snapshot_example(&program)?;

    ensure_matches(
        matches!(result.outcome(), RunOutcome::Return(output) if output.as_bytes() == b"ok"),
        "expected return output",
    )?;
    ensure_eq!(events.len(), 3)?;

    let initial = events
        .first()
        .ok_or(TestFailure::message("expected initial trace event"))?;
    let first_step = events
        .get(1)
        .ok_or(TestFailure::message("expected first trace step"))?;
    let second_step = events
        .get(2)
        .ok_or(TestFailure::message("expected second trace step"))?;

    ensure_eq!(snapshot_event_bytes(initial), b"a".as_slice())?;
    ensure_eq!(snapshot_event_bytes(first_step), b"b".as_slice())?;
    ensure_eq!(snapshot_event_bytes(second_step), b"ok".as_slice())?;
    ensure_matches(
        matches!(
            first_step,
            TraceSnapshotEvent::Step {
                effect: TraceSnapshotEffect::Continue { .. },
                ..
            }
        ),
        "expected continue step",
    )?;
    ensure_matches(
        matches!(
            second_step,
            TraceSnapshotEvent::Step {
                effect: TraceSnapshotEffect::Return { .. },
                ..
            }
        ),
        "expected return step",
    )
}

/// # Errors
///
/// Returns `TestFailure` if a continuing trace snapshot step does not carry the
/// expected rule view.
#[test]
fn trace_snapshot_continue_step_carries_rule_view() -> TestResult {
    let program = Program::parse(ProgramSource::from_str("a=b\nb=(return)ok"))?;
    let (_, events) = trace_snapshot_example(&program)?;
    let first_step = events
        .get(1)
        .ok_or(TestFailure::message("expected first trace step"))?;

    match first_step {
        TraceSnapshotEvent::Step {
            rule,
            effect: TraceSnapshotEffect::Continue { state },
            ..
        } => {
            ensure_eq!(state.as_bytes(), b"b".as_slice())?;
            ensure_eq!(rule.canonical_source()?, b"a=b".as_slice())?;
            Ok(())
        }
        TraceSnapshotEvent::Initial { .. } | TraceSnapshotEvent::Step { .. } => {
            Err(TestFailure::message("expected continuing step event"))
        }
    }
}

/// # Errors
///
/// Returns `TestFailure` if borrowed-to-snapshot conversion uses runtime limits
/// instead of only the snapshot limit.
#[test]
fn borrowed_trace_to_snapshot_uses_only_snapshot_limit() -> TestResult {
    let program = Program::parse(ProgramSource::from_str("a=b"))?;
    let mut materialization = None;

    program.run_with_borrowed_trace(
        &runtime_input(b"a")?,
        RunLimits::new(
            StepLimit::new(10),
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        ),
        |event| {
            if materialization.is_none() {
                materialization = Some(event.to_snapshot(TraceSnapshotByteLimit::new(0)));
            }
        },
    )?;

    ensure_matches(
        matches!(
            materialization.ok_or(TestFailure::message("expected trace event"))?,
            Err(TraceSnapshotError::Limit {
                limit,
                attempted_len,
            }) if limit == TraceSnapshotByteLimit::new(0) && attempted_len.get() == 1
        ),
        "expected trace snapshot byte limit",
    )
}

/// # Errors
///
/// Returns `TestFailure` if the snapshot API conflates runtime, snapshot, and
/// sink failures.
#[test]
fn trace_snapshot_api_splits_runtime_snapshot_and_sink_failures() -> TestResult {
    let program = Program::parse(ProgramSource::from_str("a=b"))?;
    let runtime_error = program.run_with_trace_snapshots(
        &runtime_input(b"a")?,
        TraceSnapshotLimits::new(
            RunLimits::new(
                StepLimit::new(0),
                DEFAULT_MAX_STATE_LEN,
                DEFAULT_MAX_RETURN_LEN,
            ),
            TraceSnapshotByteLimit::new(10),
        ),
        |_event| {},
    );
    let runtime_error = expect_trace_snapshot_error(runtime_error)?;
    ensure_matches(
        matches!(
            runtime_error,
            TraceSnapshotRunError::Run(RunError::Limit(_))
        ),
        "expected runtime failure variant",
    )?;

    let snapshot_error = program.run_with_trace_snapshots(
        &runtime_input(b"a")?,
        TraceSnapshotLimits::new(
            RunLimits::new(
                StepLimit::new(10),
                DEFAULT_MAX_STATE_LEN,
                DEFAULT_MAX_RETURN_LEN,
            ),
            TraceSnapshotByteLimit::new(0),
        ),
        |_event| {},
    );
    ensure_matches(
        matches!(
            snapshot_error,
            Err(TraceSnapshotRunError::Snapshot(TraceSnapshotError::Limit {
                limit,
                attempted_len,
            })) if limit == TraceSnapshotByteLimit::new(0) && attempted_len.get() == 1
        ),
        "expected snapshot materialization limit",
    )?;

    let sink_error = program.try_run_with_trace_snapshots(
        &runtime_input(b"a")?,
        TraceSnapshotLimits::new(
            RunLimits::new(
                StepLimit::new(10),
                DEFAULT_MAX_STATE_LEN,
                DEFAULT_MAX_RETURN_LEN,
            ),
            TraceSnapshotByteLimit::new(10),
        ),
        |_event| Err::<(), _>("trace sink full"),
    );
    ensure_eq!(
        sink_error,
        Err(FallibleTraceSnapshotRunError::Trace("trace sink full")),
    )
}

/// # Errors
///
/// Returns `TestFailure` if the final trace event no longer matches the run
/// result.
#[test]
fn traced_final_event_matches_run_result() -> TestResult {
    let program = Program::parse(ProgramSource::from_str("a=b\nb=(return)c"))?;
    let mut events = Vec::new();
    let limits = TraceSnapshotLimits::new(
        RunLimits::new(
            StepLimit::new(10),
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        ),
        DEFAULT_MAX_TRACE_SNAPSHOT_LEN,
    );

    let result = program.run_with_trace_snapshots(&runtime_input(b"a")?, limits, |event| {
        events.push(event);
    })?;

    let last = events
        .last()
        .ok_or(TestFailure::message("expected final trace event"))?;
    let result_bytes = match result.outcome() {
        RunOutcome::Stable(output) => output.as_bytes(),
        RunOutcome::Return(output) => output.as_bytes(),
    };
    ensure_eq!(snapshot_event_bytes(last), result_bytes)?;
    ensure_eq!(events.len(), result.steps().get() + 1)?;
    ensure_matches(
        matches!(
            last,
            TraceSnapshotEvent::Step {
                effect: TraceSnapshotEffect::Return { .. },
                ..
            }
        ),
        "expected final return step",
    )
}
