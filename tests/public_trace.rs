//! Public trace API contract tests.

#[path = "support/runtime.rs"]
mod runtime_support;
mod support;

use rsaeb::error::{
    RunError, RunFinishError, RunStepError, TraceSnapshotError, TraceSnapshotRunError,
    TracedRunError,
};
use rsaeb::execution::BorrowedStepTransition;
use rsaeb::input::AdmittedRun;
use rsaeb::inspect::{ReturnRuleView, RewriteRuleView};
use rsaeb::limits::TraceSnapshotByteLimit;
use rsaeb::policy::{DefaultTraceSnapshotPolicy, StaticTraceSnapshotPolicy};
use rsaeb::program::{ExecutableProgram, RunOutcome, RunResult};
use rsaeb::trace::{BorrowedTrace, BorrowedTraceEvent, SnapshotTrace, TraceSnapshotEvent};
use runtime_support::{DEFAULT_BYTE_BUDGET, DefaultInputRunPolicy, TestRunPolicy};
use support::{TestFailure, TestResult, ensure_eq, ensure_matches, parse_program};

/// Returns the expected trace snapshot run error.
///
/// # Errors
///
/// Returns `TestFailure` if the traced run succeeds.
fn expect_trace_snapshot_error<T>(
    result: Result<T, TraceSnapshotRunError<TestFailure>>,
) -> Result<TraceSnapshotRunError<TestFailure>, TestFailure> {
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
    program: &ExecutableProgram,
) -> Result<(RunResult, Vec<TraceSnapshotEvent<'_>>), TestFailure> {
    let mut events = Vec::new();
    let limits = DefaultInputRunPolicy::<10_000, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new();
    let result = program.trace(
        runtime_input(b"a", limits)?,
        SnapshotTrace::<DefaultTraceSnapshotPolicy, _>::new(|event| {
            events.push(event);
            Ok::<(), TestFailure>(())
        }),
    )?;

    Ok((result, events))
}

fn snapshot_event_bytes<'event>(event: &'event TraceSnapshotEvent<'_>) -> &'event [u8] {
    match event {
        TraceSnapshotEvent::Initial { state } => state.as_slice(),
        TraceSnapshotEvent::Rewritten { state, .. } => state.as_slice(),
        TraceSnapshotEvent::Returned { output, .. } => output.as_slice(),
    }
}

fn traced_test_failure(error: TracedRunError<TestFailure>) -> TestFailure {
    match error {
        TracedRunError::Run(error) => TestFailure::from(error),
        TracedRunError::Trace(error) => error,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum CommittedStepSignature {
    Continue {
        step: usize,
        rule_position: usize,
        state: Vec<u8>,
    },
    Return {
        step: usize,
        rule_position: usize,
        output: Vec<u8>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutcomeRuleShape {
    AlwaysRewrite,
    OnceRewrite,
    AlwaysReturn,
    OnceReturn,
}

fn rewrite_rule_shape(rule: RewriteRuleView<'_>) -> OutcomeRuleShape {
    match rule {
        RewriteRuleView::Always(_) => OutcomeRuleShape::AlwaysRewrite,
        RewriteRuleView::Once(_) => OutcomeRuleShape::OnceRewrite,
    }
}

fn return_rule_shape(rule: ReturnRuleView<'_>) -> OutcomeRuleShape {
    match rule {
        ReturnRuleView::Always(_) => OutcomeRuleShape::AlwaysReturn,
        ReturnRuleView::Once(_) => OutcomeRuleShape::OnceReturn,
    }
}

/// Verifies borrowed and snapshot tracing preserve one exact outcome rule shape.
///
/// # Errors
///
/// Returns `TestFailure` if either trace surface erases action or repeat provenance.
fn ensure_trace_rule_shape(source: &str, expected: OutcomeRuleShape) -> TestResult {
    let program = parse_program(source)?;
    let limits = DefaultInputRunPolicy::<10, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new();
    let mut borrowed_shape = None;
    program
        .trace(
            runtime_input(b"a", limits)?,
            BorrowedTrace::new(|event| {
                let shape = match event {
                    BorrowedTraceEvent::Initial { .. } => return Ok(()),
                    BorrowedTraceEvent::Rewritten { rule, .. } => rewrite_rule_shape(rule),
                    BorrowedTraceEvent::Returned { rule, .. } => return_rule_shape(rule),
                };
                borrowed_shape = Some(shape);
                Ok(())
            }),
        )
        .map_err(traced_test_failure)?;
    ensure_eq!(borrowed_shape, Some(expected))?;

    let mut snapshot_shape = None;
    program.trace(
        runtime_input(b"a", limits)?,
        SnapshotTrace::<DefaultTraceSnapshotPolicy, _>::new(|event| {
            let shape = match event {
                TraceSnapshotEvent::Initial { .. } => return Ok(()),
                TraceSnapshotEvent::Rewritten { rule, .. } => rewrite_rule_shape(rule),
                TraceSnapshotEvent::Returned { rule, .. } => return_rule_shape(rule),
            };
            snapshot_shape = Some(shape);
            Ok::<(), TestFailure>(())
        }),
    )?;
    ensure_eq!(snapshot_shape, Some(expected))
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

/// Collects committed step signatures from borrowed tracing.
///
/// # Errors
///
/// Returns `TestFailure` if tracing or materialization fails.
fn borrowed_trace_step_signatures(
    program: &ExecutableProgram,
    admitted: AdmittedRun<impl rsaeb::policy::ExecutionPolicy>,
) -> Result<Vec<CommittedStepSignature>, TestFailure> {
    let mut signatures = Vec::new();
    program
        .trace(
            admitted,
            BorrowedTrace::new(|event| {
                match event {
                    BorrowedTraceEvent::Initial { .. } => {}
                    BorrowedTraceEvent::Rewritten { step, rule, state } => {
                        signatures.push(CommittedStepSignature::Continue {
                            step: step.get(),
                            rule_position: rule.position().number().get(),
                            state: state.materialize()?.into_raw_bytes(),
                        });
                    }
                    BorrowedTraceEvent::Returned { step, rule, output } => {
                        signatures.push(CommittedStepSignature::Return {
                            step: step.get(),
                            rule_position: rule.position().number().get(),
                            output: output.materialize()?.into_raw_bytes(),
                        });
                    }
                }
                Ok(())
            }),
        )
        .map_err(traced_test_failure)?;
    Ok(signatures)
}

/// Collects committed step signatures from borrowed stepwise execution.
///
/// # Errors
///
/// Returns `TestFailure` if stepping or materialization fails.
fn borrowed_step_signatures(
    program: &ExecutableProgram,
    admitted: AdmittedRun<impl rsaeb::policy::ExecutionPolicy>,
) -> Result<Vec<CommittedStepSignature>, TestFailure> {
    let mut signatures = Vec::new();
    let mut session = program.steps(admitted)?;
    loop {
        match session.step() {
            BorrowedStepTransition::Applied(applied) => {
                signatures.push(CommittedStepSignature::Continue {
                    step: applied.step().get(),
                    rule_position: applied.rule().position().number().get(),
                    state: applied.state().materialize()?.into_raw_bytes(),
                });
                session = applied.into_session();
            }
            BorrowedStepTransition::Returned(returned) => {
                signatures.push(CommittedStepSignature::Return {
                    step: returned.step().get(),
                    rule_position: returned.rule().position().number().get(),
                    output: returned.output().as_slice().to_vec(),
                });
                return Ok(signatures);
            }
            BorrowedStepTransition::Stable(_) => return Ok(signatures),
            BorrowedStepTransition::Failed(failed) => {
                return Err(TestFailure::from(failed.into_error()));
            }
        }
    }
}

/// # Errors
///
/// Returns `TestFailure` if borrowed trace events allocate snapshots or expose
/// incorrect byte data.
#[test]
fn trace_events_are_emitted_without_snapshots() -> TestResult {
    let program = parse_program("a=b\nb=(return)ok")?;
    let mut seen = Vec::new();
    let limits = DefaultInputRunPolicy::<10_000, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new();

    let result = program
        .trace(
            runtime_input(b"a", limits)?,
            BorrowedTrace::new(|event| {
                let bytes = match event {
                    BorrowedTraceEvent::Initial { state }
                    | BorrowedTraceEvent::Rewritten { state, .. } => {
                        state.materialize()?.into_raw_bytes()
                    }
                    BorrowedTraceEvent::Returned { output, .. } => {
                        output.materialize()?.into_raw_bytes()
                    }
                };
                seen.push((event.byte_count().get(), bytes));
                Ok(())
            }),
        )
        .map_err(traced_test_failure)?;

    ensure_matches(
        matches!(result.outcome(), RunOutcome::Return(output) if output.as_slice() == b"ok"),
        "expected return output",
    )?;
    ensure_eq!(
        seen.as_slice(),
        &[(1, b"a".to_vec()), (1, b"b".to_vec()), (2, b"ok".to_vec())],
    )
}

/// # Errors
///
/// Returns `TestFailure` if borrowed trace and borrowed stepwise execution report
/// different committed outcomes.
#[test]
fn borrowed_trace_steps_match_borrowed_stepwise_commits() -> TestResult {
    let source = "(once)a=b\nb=(return)ok";
    let limits = DefaultInputRunPolicy::<10, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new();

    let program = parse_program(source)?;
    let trace_steps = borrowed_trace_step_signatures(&program, runtime_input(b"a", limits)?)?;
    let stepwise_steps = borrowed_step_signatures(&program, runtime_input(b"a", limits)?)?;

    ensure_eq!(trace_steps, stepwise_steps)
}

/// # Errors
///
/// Returns `TestFailure` if borrowed or snapshot tracing erases successful
/// outcome action/repeat provenance.
#[test]
fn trace_success_events_preserve_exact_rule_shapes() -> TestResult {
    ensure_trace_rule_shape("a=b", OutcomeRuleShape::AlwaysRewrite)?;
    ensure_trace_rule_shape("(once)a=b", OutcomeRuleShape::OnceRewrite)?;
    ensure_trace_rule_shape("a=(return)ok", OutcomeRuleShape::AlwaysReturn)?;
    ensure_trace_rule_shape("(once)a=(return)ok", OutcomeRuleShape::OnceReturn)
}

/// # Errors
///
/// Returns `TestFailure` if trace snapshot events lose materialized bytes or
/// exact outcome variants.
#[test]
fn trace_snapshot_events_carry_bytes_and_exact_outcomes() -> TestResult {
    let program = parse_program("a=b\nb=(return)ok")?;
    let (result, events) = trace_snapshot_example(&program)?;

    ensure_matches(
        matches!(result.outcome(), RunOutcome::Return(output) if output.as_slice() == b"ok"),
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
        matches!(first_step, TraceSnapshotEvent::Rewritten { .. }),
        "expected rewritten step",
    )?;
    ensure_matches(
        matches!(second_step, TraceSnapshotEvent::Returned { .. }),
        "expected return step",
    )
}

/// # Errors
///
/// Returns `TestFailure` if a rewritten trace snapshot step does not carry the
/// expected rule view.
#[test]
fn trace_snapshot_rewritten_step_carries_rule_view() -> TestResult {
    let program = parse_program("a=b\nb=(return)ok")?;
    let (_, events) = trace_snapshot_example(&program)?;
    let first_step = events
        .get(1)
        .ok_or(TestFailure::message("expected first trace step"))?;

    match first_step {
        TraceSnapshotEvent::Rewritten { rule, state, .. } => {
            ensure_eq!(state.as_slice(), b"b".as_slice())?;
            ensure_eq!(rule.canonical_source()?.as_slice(), b"a=b".as_slice())?;
            Ok(())
        }
        TraceSnapshotEvent::Initial { .. } | TraceSnapshotEvent::Returned { .. } => {
            Err(TestFailure::message("expected rewritten step event"))
        }
    }
}

/// # Errors
///
/// Returns `TestFailure` if empty detection reads bytes from the wrong exact
/// trace variant.
#[test]
fn trace_event_empty_detection_follows_exact_variant_payloads() -> TestResult {
    let limits = DefaultInputRunPolicy::<10, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new();

    let rewritten_program = parse_program("a=")?;
    let mut rewritten_empty = Vec::new();
    rewritten_program
        .trace(
            runtime_input(b"a", limits)?,
            BorrowedTrace::new(|event| {
                rewritten_empty.push(event.is_empty());
                Ok::<(), TestFailure>(())
            }),
        )
        .map_err(traced_test_failure)?;
    ensure_eq!(rewritten_empty, [false, true])?;

    let returned_program = parse_program("=(return)")?;
    let mut returned_empty = Vec::new();
    returned_program
        .trace(
            runtime_input(b"", limits)?,
            BorrowedTrace::new(|event| {
                returned_empty.push(event.is_empty());
                Ok::<(), TestFailure>(())
            }),
        )
        .map_err(traced_test_failure)?;
    ensure_eq!(returned_empty, [true, true])
}

/// # Errors
///
/// Returns `TestFailure` if borrowed-to-snapshot conversion uses runtime limits
/// instead of only the snapshot limit.
#[test]
fn borrowed_trace_to_snapshot_uses_only_snapshot_limit() -> TestResult {
    let program = parse_program("a=b")?;
    let mut materialization = None;
    let limits = DefaultInputRunPolicy::<10, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new();

    program
        .trace(
            runtime_input(b"a", limits)?,
            BorrowedTrace::new(|event| {
                if materialization.is_none() {
                    materialization = Some(event.to_snapshot::<StaticTraceSnapshotPolicy<0>>());
                }
                Ok::<(), TestFailure>(())
            }),
        )
        .map_err(traced_test_failure)?;

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
    let program = parse_program("a=b")?;
    let runtime_limits =
        DefaultInputRunPolicy::<0, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new();
    let runtime_error = program.trace(
        runtime_input(b"a", runtime_limits)?,
        SnapshotTrace::<StaticTraceSnapshotPolicy<10>, _>::new(|_event| Ok::<(), TestFailure>(())),
    );
    let runtime_error = expect_trace_snapshot_error(runtime_error)?;
    ensure_matches(
        matches!(
            runtime_error,
            TraceSnapshotRunError::Run(RunError::Finish(RunFinishError::Step(
                RunStepError::StepLimit(_)
            )))
        ),
        "expected runtime failure variant",
    )?;

    let snapshot_limits =
        DefaultInputRunPolicy::<10, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new();
    let snapshot_error = program.trace(
        runtime_input(b"a", snapshot_limits)?,
        SnapshotTrace::<StaticTraceSnapshotPolicy<0>, _>::new(|_event| Ok::<(), TestFailure>(())),
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

    let sink_limits = DefaultInputRunPolicy::<10, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new();
    let sink_error = program.trace(
        runtime_input(b"a", sink_limits)?,
        SnapshotTrace::<StaticTraceSnapshotPolicy<10>, _>::new(|_event| {
            Err::<(), _>("trace sink full")
        }),
    );
    ensure_eq!(
        sink_error,
        Err(TraceSnapshotRunError::Trace("trace sink full")),
    )
}

/// # Errors
///
/// Returns `TestFailure` if return trace snapshots do not use the trace
/// snapshot permit attached to the return event.
#[test]
fn trace_snapshot_return_event_uses_return_event_permit() -> TestResult {
    let program = parse_program("a=(return)ok")?;
    let limits = DefaultInputRunPolicy::<10, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new();
    let mut events = Vec::new();

    let error = program.trace(
        runtime_input(b"a", limits)?,
        SnapshotTrace::<StaticTraceSnapshotPolicy<1>, _>::new(|event| {
            events.push(event);
            Ok::<(), TestFailure>(())
        }),
    );

    ensure_eq!(events.len(), 1)?;
    ensure_matches(
        matches!(
            error,
            Err(TraceSnapshotRunError::Snapshot(TraceSnapshotError::Limit {
                limit,
                attempted_len,
            })) if limit == TraceSnapshotByteLimit::new(1) && attempted_len.get() == 2
        ),
        "expected return trace snapshot limit",
    )
}

/// # Errors
///
/// Returns `TestFailure` if the final trace event no longer matches the run
/// result.
#[test]
fn trace_final_event_matches_run_result() -> TestResult {
    let program = parse_program("a=b\nb=(return)c")?;
    let mut events = Vec::new();
    let limits = DefaultInputRunPolicy::<10, DEFAULT_BYTE_BUDGET, DEFAULT_BYTE_BUDGET>::new();

    let result = program.trace(
        runtime_input(b"a", limits)?,
        SnapshotTrace::<DefaultTraceSnapshotPolicy, _>::new(|event| {
            events.push(event);
            Ok::<(), TestFailure>(())
        }),
    )?;

    let last = events
        .last()
        .ok_or(TestFailure::message("expected final trace event"))?;
    let result_bytes = match result.outcome() {
        RunOutcome::Stable(output) => output.as_slice(),
        RunOutcome::Return(output) => output.as_slice(),
    };
    ensure_eq!(snapshot_event_bytes(last), result_bytes)?;
    let expected_event_count = result
        .steps()
        .get()
        .checked_add(1)
        .ok_or(TestFailure::message("trace event count overflow"))?;
    ensure_eq!(events.len(), expected_event_count)?;
    ensure_matches(
        matches!(last, TraceSnapshotEvent::Returned { .. }),
        "expected final return step",
    )
}
