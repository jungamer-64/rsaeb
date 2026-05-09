use alloc::vec::Vec;

use crate::allocation::{AllocationContext, AllocationError, try_push, try_reserve_total_exact};
use crate::bytes::{ByteCount, RuntimeByte};
use crate::error::{LimitError, RunError};
use crate::program::{ReturnOutput, RunLimits, RuntimeStateSnapshot, StepCount};
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

    /// Runtime state length in bytes.
    #[must_use]
    pub const fn byte_count(self) -> ByteCount {
        ByteCount::new(self.bytes.len())
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

    /// Returns whether this state has exactly the expected bytes.
    #[must_use]
    pub fn eq_bytes(self, expected: &[u8]) -> bool {
        self.byte_count().get() == expected.len()
            && self
                .bytes()
                .zip(expected.iter().copied())
                .all(|(actual, expected)| actual == expected)
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
        try_reserve_total_exact(&mut output, self.byte_count().get(), context)?;
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

#[derive(Debug, PartialEq, Eq)]
pub enum TraceSnapshotEffect {
    /// The step produced the next runtime state and execution may continue.
    Continue { state: RuntimeStateSnapshot },
    /// The step executed `(return)` and produced final output bytes.
    Return { output: ReturnOutput },
}

impl TraceSnapshotEffect {
    /// State/output bytes carried by this effect.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Continue { state } => state.as_bytes(),
            Self::Return { output } => output.as_bytes(),
        }
    }
}

/// Borrowed trace effect emitted by borrowed tracing APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowedTraceEffect<'program, 'run> {
    /// The step produced the next runtime state and execution may continue.
    Continue { state: RuntimeStateView<'run> },
    /// The step executed `(return)` and produced final output bytes.
    Return { output: PayloadView<'program> },
}

impl BorrowedTraceEffect<'_, '_> {
    /// Byte length carried by this effect.
    #[must_use]
    pub fn byte_count(self) -> ByteCount {
        match self {
            Self::Continue { state } => state.byte_count(),
            Self::Return { output } => output.byte_count(),
        }
    }

    /// Whether the carried bytes are empty.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.byte_count().is_zero()
    }

    /// Returns whether this effect carries exactly the expected bytes.
    #[must_use]
    pub fn eq_bytes(self, expected: &[u8]) -> bool {
        match self {
            Self::Continue { state } => state.eq_bytes(expected),
            Self::Return { output } => output.eq_bytes(expected),
        }
    }

    fn to_snapshot(self, limits: RunLimits) -> Result<TraceSnapshotEffect, RunError> {
        ensure_trace_len(self.byte_count(), limits)?;
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
                        .map_err(RunError::from)?,
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
    Initial { state: RuntimeStateSnapshot },
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

impl TraceSnapshotEvent<'_> {
    /// State/output bytes carried by this event.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Initial { state } => state.as_bytes(),
            Self::Step { effect, .. } => effect.as_bytes(),
        }
    }
}

/// Trace event emitted by borrowed tracing APIs.
///
/// The event borrows runtime bytes only for the duration of the callback. This
/// API is the allocation-free tracing primitive; snapshot tracing is derived
/// from it by materializing snapshots under `RunLimits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowedTraceEvent<'program, 'run> {
    /// Initial runtime state before any rewrite step.
    Initial { state: RuntimeStateView<'run> },
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
    /// Byte length carried by this event.
    #[must_use]
    pub fn byte_count(self) -> ByteCount {
        match self {
            Self::Initial { state } => state.byte_count(),
            Self::Step { effect, .. } => effect.byte_count(),
        }
    }

    /// Whether the carried bytes are empty.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.byte_count().is_zero()
    }

    /// Returns whether this event carries exactly the expected bytes.
    #[must_use]
    pub fn eq_bytes(self, expected: &[u8]) -> bool {
        match self {
            Self::Initial { state } => state.eq_bytes(expected),
            Self::Step { effect, .. } => effect.eq_bytes(expected),
        }
    }

    /// Materializes this borrowed event as a trace snapshot event.
    ///
    /// # Errors
    ///
    /// Returns `RunError::Limit` if the event bytes exceed
    /// `RunLimits::trace_snapshot_byte_limit`. Returns `RunError::Allocation` if
    /// snapshot allocation fails.
    pub fn to_snapshot(self, limits: RunLimits) -> Result<TraceSnapshotEvent<'program>, RunError> {
        ensure_trace_len(self.byte_count(), limits)?;
        match self {
            Self::Initial { state } => Ok(TraceSnapshotEvent::Initial {
                state: RuntimeStateSnapshot::from_vec(
                    state.to_vec_with_context(AllocationContext::TraceSnapshot)?,
                ),
            }),
            Self::Step { step, rule, effect } => Ok(TraceSnapshotEvent::Step {
                step,
                rule,
                effect: effect.to_snapshot(limits)?,
            }),
        }
    }
}

fn ensure_trace_len(len: ByteCount, limits: RunLimits) -> Result<(), RunError> {
    if len.get() > limits.trace_snapshot_byte_limit().get() {
        return Err(
            LimitError::trace_snapshot(limits.trace_snapshot_byte_limit(), len.get()).into(),
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::test_support::{
        TestFailure, TestResult, expect_event, expect_return_output, result_bytes,
    };
    use crate::{
        BorrowedTraceEvent, ByteCount, Program, RuleActionView, RunLimits, StepLimit,
        TraceSnapshotEffect, TraceSnapshotEvent, TracedRunError,
    };
    use std::vec::Vec;

    #[test]
    fn borrowed_trace_events_are_emitted_without_snapshots() -> TestResult {
        let program = Program::parse_str("a=b\nb=(return)ok")?;
        let mut seen = Vec::new();

        let result = program.run_with_borrowed_trace(
            b"a",
            RunLimits::new(StepLimit::new(10_000)),
            |event| {
                seen.push((
                    event.byte_count().get(),
                    event.eq_bytes(match event {
                        BorrowedTraceEvent::Initial { .. } => b"a".as_ref(),
                        BorrowedTraceEvent::Step { step, .. } if step.get() == 1 => b"b".as_ref(),
                        BorrowedTraceEvent::Step { .. } => b"ok".as_ref(),
                    }),
                ));
            },
        )?;

        expect_return_output(&result, b"ok")?;
        assert_eq!(seen.as_slice(), &[(1, true), (1, true), (2, true)]);
        Ok(())
    }

    #[test]
    fn trace_snapshot_events_are_emitted_without_core_stderr() -> TestResult {
        let program = Program::parse_str("a=b\nb=(return)ok")?;
        let mut events = Vec::new();
        let result = program.run_with_trace_snapshots(
            b"a",
            RunLimits::new(StepLimit::new(10_000)),
            |event| {
                events.push(event);
            },
        )?;

        expect_return_output(&result, b"ok")?;
        assert_eq!(events.len(), 3);

        let initial = expect_event(&events, 0)?;
        let first_step = expect_event(&events, 1)?;
        let second_step = expect_event(&events, 2)?;

        assert!(matches!(initial, TraceSnapshotEvent::Initial { .. }));
        assert_eq!(initial.as_bytes(), b"a");
        assert_eq!(first_step.as_bytes(), b"b");
        assert_eq!(second_step.as_bytes(), b"ok");
        assert!(matches!(
            first_step,
            TraceSnapshotEvent::Step {
                effect: TraceSnapshotEffect::Continue { .. },
                ..
            }
        ));
        assert!(matches!(
            second_step,
            TraceSnapshotEvent::Step {
                effect: TraceSnapshotEffect::Return { .. },
                ..
            }
        ));

        match first_step {
            TraceSnapshotEvent::Step {
                rule,
                effect: TraceSnapshotEffect::Continue { state },
                ..
            } => {
                assert_eq!(state.as_bytes(), b"b");
                assert_eq!(state.byte_count(), ByteCount::new(1));
                assert_eq!(rule.position().number().get(), 1);
                assert_eq!(rule.line_number().get(), 1);
                assert!(rule.lhs().eq_bytes(b"a"));
                assert!(matches!(
                    rule.action(),
                    RuleActionView::Replace(payload) if payload.eq_bytes(b"b")
                ));
                assert_eq!(rule.canonical_source()?, b"a=b");
            }
            TraceSnapshotEvent::Initial { .. } | TraceSnapshotEvent::Step { .. } => {
                return Err(TestFailure::Message("expected continuing step event"));
            }
        }

        Ok(())
    }

    #[test]
    fn fallible_trace_callback_can_abort_execution() -> TestResult {
        let program = Program::parse_str("a=b\nb=c")?;
        let result = program.try_run_with_trace_snapshots(
            b"a",
            RunLimits::new(StepLimit::new(10_000)),
            |_event| Err::<(), _>("trace sink full"),
        );

        assert_eq!(result, Err(TracedRunError::Trace("trace sink full")));
        Ok(())
    }

    #[test]
    fn traced_final_event_matches_run_result() -> TestResult {
        let program = Program::parse_str("a=b\nb=(return)c")?;
        let mut events = Vec::new();

        let result = program.run_with_trace_snapshots(
            b"a",
            RunLimits::new(StepLimit::new(10)),
            |event| {
                events.push(event);
            },
        )?;

        let last = events
            .last()
            .ok_or(TestFailure::Message("expected final trace event"))?;
        assert_eq!(last.as_bytes(), result_bytes(&result));
        assert_eq!(events.len(), result.steps().get() + 1);
        assert!(matches!(
            last,
            TraceSnapshotEvent::Step {
                effect: TraceSnapshotEffect::Return { .. },
                ..
            }
        ));
        Ok(())
    }
}
