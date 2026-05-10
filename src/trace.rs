use alloc::vec::Vec;

use crate::allocation::{AllocationContext, AllocationError, try_push, try_reserve_total_exact};
use crate::bytes::{RuntimeByte, RuntimeStateByteCount, TraceSnapshotByteCount};
use crate::error::TraceSnapshotError;
use crate::program::{ReturnOutput, RuntimeStateSnapshot, StepCount, TraceSnapshotByteLimit};
use crate::rule::{PayloadView, RuleView};

/// Borrowed view of runtime-state bytes.
///
/// This lets trace sinks inspect state without forcing the runtime to allocate a
/// `Vec<u8>` for every event. Internally the runtime state is not stored as raw
/// `u8`, so public byte access is an iterator/materialization boundary.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RuntimeStateView<'run> {
    bytes: &'run [RuntimeByte],
}

impl<'run> RuntimeStateView<'run> {
    pub(crate) const fn new(bytes: &'run [RuntimeByte]) -> Self {
        Self { bytes }
    }

    /// Whether the state is empty.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.bytes.is_empty()
    }

    /// Runtime state bytes as a materializing iterator.
    pub fn bytes(self) -> impl Iterator<Item = u8> + 'run {
        self.bytes.iter().copied().map(RuntimeByte::materialize)
    }

    /// Runtime state length in bytes.
    #[must_use]
    pub const fn byte_count(self) -> RuntimeStateByteCount {
        RuntimeStateByteCount::new(self.bytes.len())
    }

    /// Materializes this runtime-state view as owned bytes with explicit
    /// fallible allocation.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the output buffer cannot be allocated.
    pub fn to_vec(self) -> Result<Vec<u8>, AllocationError> {
        self.to_vec_with_context(AllocationContext::RuntimeStateView)
    }

    pub(crate) fn to_vec_with_context(
        self,
        context: AllocationContext,
    ) -> Result<Vec<u8>, AllocationError> {
        let mut output = Vec::new();
        try_reserve_total_exact(&mut output, self.bytes.len(), context)?;
        for byte in self.bytes() {
            try_push(&mut output, byte, context)?;
        }
        Ok(output)
    }
}

impl core::fmt::Debug for RuntimeStateView<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries((*self).bytes()).finish()
    }
}

/// Owned trace effect emitted by trace snapshot APIs.
#[derive(Debug, PartialEq, Eq)]
pub enum TraceSnapshotEffect {
    /// The step produced the next runtime state and execution may continue.
    Continue {
        /// Materialized runtime state after the rewrite step.
        state: RuntimeStateSnapshot,
    },
    /// The step executed `(return)` and produced final output bytes.
    Return {
        /// Materialized `(return)` output bytes.
        output: ReturnOutput,
    },
}

/// Borrowed trace effect emitted by borrowed tracing APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowedTraceEffect<'program, 'run> {
    /// The step produced the next runtime state and execution may continue.
    Continue {
        /// Borrowed runtime state after the rewrite step.
        state: RuntimeStateView<'run>,
    },
    /// The step executed `(return)` and produced final output bytes.
    Return {
        /// Borrowed `(return)` payload bytes from the parsed program.
        output: PayloadView<'program>,
    },
}

impl BorrowedTraceEffect<'_, '_> {
    /// Byte length that would be materialized by snapshot tracing.
    #[must_use]
    pub fn byte_count(self) -> TraceSnapshotByteCount {
        match self {
            Self::Continue { state } => TraceSnapshotByteCount::new(state.byte_count().get()),
            Self::Return { output } => TraceSnapshotByteCount::new(output.byte_count().get()),
        }
    }

    /// Whether the carried bytes are empty.
    #[must_use]
    pub fn is_empty(self) -> bool {
        match self {
            Self::Continue { state } => state.is_empty(),
            Self::Return { output } => output.is_empty(),
        }
    }

    fn to_snapshot(
        self,
        limit: TraceSnapshotByteLimit,
    ) -> Result<TraceSnapshotEffect, TraceSnapshotError> {
        ensure_trace_len(self.byte_count(), limit)?;
        match self {
            Self::Continue { state } => Ok(TraceSnapshotEffect::Continue {
                state: RuntimeStateSnapshot::from_vec(
                    state.to_vec_with_context(AllocationContext::TraceSnapshot)?,
                ),
            }),
            Self::Return { output } => Ok(TraceSnapshotEffect::Return {
                output: ReturnOutput::from_vec(
                    output
                        .to_vec_with_context(AllocationContext::TraceSnapshot)
                        .map_err(TraceSnapshotError::from)?,
                ),
            }),
        }
    }
}

/// Trace event emitted by trace snapshot APIs.
///
/// State and return-output bytes are materialized as owned `Vec<u8>` snapshots.
/// Step events still borrow the structured rule view from `Program`, so these
/// events cannot outlive the parsed program. Return steps cannot be confused
/// with ordinary continuation steps by forgetting to inspect a boolean flag.
#[derive(Debug, PartialEq, Eq)]
pub enum TraceSnapshotEvent<'program> {
    /// Initial runtime state before any rewrite step.
    Initial {
        /// Materialized initial runtime state.
        state: RuntimeStateSnapshot,
    },
    /// One applied rule.
    Step {
        /// One-based applied step count.
        step: StepCount,
        /// Structured view of the applied rule.
        rule: RuleView<'program>,
        /// Structured result of the rewrite step.
        effect: TraceSnapshotEffect,
    },
}

/// Trace event emitted by borrowed tracing APIs.
///
/// The event borrows runtime bytes only for the duration of the callback. This
/// API is the allocation-free tracing primitive; snapshot tracing is derived
/// from it by materializing snapshots under `RunLimits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowedTraceEvent<'program, 'run> {
    /// Initial runtime state before any rewrite step.
    Initial {
        /// Borrowed initial runtime state.
        state: RuntimeStateView<'run>,
    },
    /// One applied rule.
    Step {
        /// One-based applied step count.
        step: StepCount,
        /// Structured view of the applied rule.
        rule: RuleView<'program>,
        /// Structured result of the rewrite step.
        effect: BorrowedTraceEffect<'program, 'run>,
    },
}

impl<'program> BorrowedTraceEvent<'program, '_> {
    /// Byte length that would be materialized by snapshot tracing.
    #[must_use]
    pub fn byte_count(self) -> TraceSnapshotByteCount {
        match self {
            Self::Initial { state } => TraceSnapshotByteCount::new(state.byte_count().get()),
            Self::Step { effect, .. } => effect.byte_count(),
        }
    }

    /// Whether the carried bytes are empty.
    #[must_use]
    pub fn is_empty(self) -> bool {
        match self {
            Self::Initial { state } => state.is_empty(),
            Self::Step { effect, .. } => effect.is_empty(),
        }
    }

    /// Materializes this borrowed event as a trace snapshot event.
    ///
    /// # Errors
    ///
    /// Returns `TraceSnapshotError::Limit` if the event bytes exceed `limit`.
    /// Returns `TraceSnapshotError::Allocation` if snapshot allocation fails.
    pub fn to_snapshot(
        self,
        limit: TraceSnapshotByteLimit,
    ) -> Result<TraceSnapshotEvent<'program>, TraceSnapshotError> {
        ensure_trace_len(self.byte_count(), limit)?;
        match self {
            Self::Initial { state } => Ok(TraceSnapshotEvent::Initial {
                state: RuntimeStateSnapshot::from_vec(
                    state.to_vec_with_context(AllocationContext::TraceSnapshot)?,
                ),
            }),
            Self::Step { step, rule, effect } => Ok(TraceSnapshotEvent::Step {
                step,
                rule,
                effect: effect.to_snapshot(limit)?,
            }),
        }
    }
}

fn ensure_trace_len(
    len: TraceSnapshotByteCount,
    limit: TraceSnapshotByteLimit,
) -> Result<(), TraceSnapshotError> {
    if len.get() > limit.get() {
        return Err(TraceSnapshotError::Limit {
            limit,
            attempted_len: len,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::test_support::{
        TestFailure, TestResult, ensure, ensure_eq, ensure_matches, expect_event,
        expect_return_output, result_bytes, runtime_input, trace_event_bytes,
    };
    use crate::{
        BorrowedTraceEffect, BorrowedTraceEvent, FallibleTraceSnapshotRunError, Program,
        RuleActionView, RunError, RunLimits, RuntimeStateByteCount, StepLimit,
        TraceSnapshotByteCount, TraceSnapshotByteLimit, TraceSnapshotEffect, TraceSnapshotError,
        TraceSnapshotEvent, TraceSnapshotRunError,
    };
    use std::vec::Vec;

    #[test]
    fn borrowed_trace_events_are_emitted_without_snapshots() -> TestResult {
        let program = Program::parse_str("a=b\nb=(return)ok")?;
        let mut seen = Vec::new();
        let limits = RunLimits::new(StepLimit::new(10_000));

        let result = program.run_with_borrowed_trace(runtime_input(b"a")?, limits, |event| {
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

        expect_return_output(&result, b"ok")?;
        ensure_eq(
            seen.as_slice(),
            &[(1, b"a".to_vec()), (1, b"b".to_vec()), (2, b"ok".to_vec())],
        )?;
        Ok(())
    }

    #[test]
    fn trace_snapshot_events_are_emitted_without_core_stderr() -> TestResult {
        let program = Program::parse_str("a=b\nb=(return)ok")?;
        let mut events = Vec::new();
        let limits = RunLimits::new(StepLimit::new(10_000));
        let result = program.run_with_trace_snapshots(runtime_input(b"a")?, limits, |event| {
            events.push(event);
        })?;

        expect_return_output(&result, b"ok")?;
        ensure_eq(events.len(), 3)?;

        let initial = expect_event(&events, 0)?;
        let first_step = expect_event(&events, 1)?;
        let second_step = expect_event(&events, 2)?;

        ensure_matches(
            matches!(initial, TraceSnapshotEvent::Initial { .. }),
            "expected initial trace event",
        )?;
        ensure_eq(trace_event_bytes(initial), b"a".as_slice())?;
        ensure_eq(trace_event_bytes(first_step), b"b".as_slice())?;
        ensure_eq(trace_event_bytes(second_step), b"ok".as_slice())?;
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
        )?;

        match first_step {
            TraceSnapshotEvent::Step {
                rule,
                effect: TraceSnapshotEffect::Continue { state },
                ..
            } => {
                ensure_eq(state.as_bytes(), b"b".as_slice())?;
                ensure_eq(state.byte_count(), RuntimeStateByteCount::new(1))?;
                ensure_eq(rule.position().number().get(), 1)?;
                ensure_eq(rule.line_number().get(), 1)?;
                ensure(rule.lhs().eq_bytes(b"a"), "expected lhs")?;
                ensure_matches(
                    matches!(
                        rule.action(),
                        RuleActionView::Replace(payload) if payload.eq_bytes(b"b")
                    ),
                    "expected replace action",
                )?;
                ensure_eq(rule.canonical_source()?, b"a=b".as_slice())?;
            }
            TraceSnapshotEvent::Initial { .. } | TraceSnapshotEvent::Step { .. } => {
                return Err(TestFailure::Message("expected continuing step event"));
            }
        }

        Ok(())
    }

    #[test]
    fn borrowed_trace_to_snapshot_uses_only_snapshot_limit() -> TestResult {
        let program = Program::parse_str("a=b")?;
        let mut materialization = None;

        program.run_with_borrowed_trace(
            runtime_input(b"a")?,
            RunLimits::new(StepLimit::new(10)),
            |event| {
                if materialization.is_none() {
                    materialization = Some(event.to_snapshot(TraceSnapshotByteLimit::new(0)));
                }
            },
        )?;

        ensure_eq(
            materialization.ok_or(TestFailure::Message("expected trace event"))?,
            Err(TraceSnapshotError::Limit {
                limit: TraceSnapshotByteLimit::new(0),
                attempted_len: TraceSnapshotByteCount::new(1),
            }),
        )
    }

    #[test]
    fn trace_snapshot_api_splits_runtime_snapshot_and_sink_failures() -> TestResult {
        let program = Program::parse_str("a=b")?;
        let runtime_error = program.run_with_trace_snapshots(
            runtime_input(b"a")?,
            RunLimits::new(StepLimit::new(0)),
            |_event| {},
        );

        ensure_matches(
            matches!(
                runtime_error,
                Err(TraceSnapshotRunError::Run(RunError::Limit(_)))
            ),
            "expected runtime failure variant",
        )?;

        let snapshot_error = program.run_with_trace_snapshots(
            runtime_input(b"a")?,
            RunLimits::bounded(
                StepLimit::new(10),
                crate::StateByteLimit::new(10),
                crate::ReturnByteLimit::new(10),
                TraceSnapshotByteLimit::new(0),
            ),
            |_event| {},
        );

        ensure_eq(
            snapshot_error,
            Err(TraceSnapshotRunError::Snapshot(TraceSnapshotError::Limit {
                limit: TraceSnapshotByteLimit::new(0),
                attempted_len: TraceSnapshotByteCount::new(1),
            })),
        )?;

        let sink_error = program.try_run_with_trace_snapshots(
            runtime_input(b"a")?,
            RunLimits::new(StepLimit::new(10)),
            |_event| Err::<(), _>("trace sink full"),
        );

        ensure_eq(
            sink_error,
            Err(FallibleTraceSnapshotRunError::Trace("trace sink full")),
        )
    }

    #[test]
    fn fallible_trace_callback_can_abort_execution() -> TestResult {
        let program = Program::parse_str("a=b\nb=c")?;
        let limits = RunLimits::new(StepLimit::new(10_000));
        let result = program.try_run_with_trace_snapshots(runtime_input(b"a")?, limits, |_event| {
            Err::<(), _>("trace sink full")
        });

        ensure_eq(
            result,
            Err(FallibleTraceSnapshotRunError::Trace("trace sink full")),
        )?;
        Ok(())
    }

    #[test]
    fn traced_final_event_matches_run_result() -> TestResult {
        let program = Program::parse_str("a=b\nb=(return)c")?;
        let mut events = Vec::new();
        let limits = RunLimits::new(StepLimit::new(10));

        let result = program.run_with_trace_snapshots(runtime_input(b"a")?, limits, |event| {
            events.push(event);
        })?;

        let last = events
            .last()
            .ok_or(TestFailure::Message("expected final trace event"))?;
        ensure_eq(trace_event_bytes(last), result_bytes(&result))?;
        let expected_events = result
            .steps()
            .checked_next()
            .ok_or(TestFailure::Message("expected trace event count"))?;
        ensure_eq(events.len(), expected_events.get())?;
        ensure_matches(
            matches!(
                last,
                TraceSnapshotEvent::Step {
                    effect: TraceSnapshotEffect::Return { .. },
                    ..
                }
            ),
            "expected final return step",
        )?;
        Ok(())
    }
}
