use alloc::vec::Vec;

use crate::allocation::{AllocationContext, AllocationError, try_push, try_reserve_total_exact};
use crate::bytes::RuntimeByte;
use crate::error::{LimitError, RunError};
use crate::program::RunLimits;
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
    pub const fn len(self) -> usize {
        self.bytes.len()
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
        self.len() == expected.len()
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
        try_reserve_total_exact(&mut output, self.len(), context)?;
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
    Continue { state: Vec<u8> },
    /// The step executed `(return)` and produced final output bytes.
    Return { output: Vec<u8> },
}

impl TraceSnapshotEffect {
    /// State/output bytes carried by this effect.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::Continue { state } => state,
            Self::Return { output } => output,
        }
    }

    /// Whether this effect stopped execution by `(return)`.
    #[must_use]
    pub const fn is_return(&self) -> bool {
        matches!(self, Self::Return { .. })
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
    /// Whether this effect stopped execution by `(return)`.
    #[must_use]
    pub const fn is_return(self) -> bool {
        matches!(self, Self::Return { .. })
    }

    /// Byte length carried by this effect.
    #[must_use]
    pub fn len(self) -> usize {
        match self {
            Self::Continue { state } => state.len(),
            Self::Return { output } => output.len(),
        }
    }

    /// Whether the carried bytes are empty.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.len() == 0
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
        ensure_trace_len(self.len(), limits)?;
        match self {
            Self::Continue { state } => Ok(TraceSnapshotEffect::Continue {
                state: state.to_vec_with_context(AllocationContext::TraceSnapshot)?,
            }),
            Self::Return { output } => Ok(TraceSnapshotEffect::Return {
                output: output
                    .to_vec_with_context(AllocationContext::TraceSnapshot)
                    .map_err(RunError::from)?,
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
    Initial { state: Vec<u8> },
    /// One applied rule.
    Step {
        /// One-based applied step count.
        step: usize,
        /// Structured view of the applied rule.
        rule: RuleView<'program>,
        /// Structured result of the rewrite step.
        effect: TraceSnapshotEffect,
    },
}

impl TraceSnapshotEvent<'_> {
    /// State/output bytes carried by this event.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::Initial { state } => state,
            Self::Step { effect, .. } => effect.bytes(),
        }
    }

    /// Whether this event is a step that stopped execution by `(return)`.
    #[must_use]
    pub const fn is_return_step(&self) -> bool {
        match self {
            Self::Initial { .. } => false,
            Self::Step { effect, .. } => effect.is_return(),
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
        step: usize,
        /// Structured view of the applied rule.
        rule: RuleView<'program>,
        /// Structured result of the rewrite step.
        effect: BorrowedTraceEffect<'program, 'run>,
    },
}

impl<'program> BorrowedTraceEvent<'program, '_> {
    /// Byte length carried by this event.
    #[must_use]
    pub fn len(self) -> usize {
        match self {
            Self::Initial { state } => state.len(),
            Self::Step { effect, .. } => effect.len(),
        }
    }

    /// Whether the carried bytes are empty.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// Returns whether this event carries exactly the expected bytes.
    #[must_use]
    pub fn eq_bytes(self, expected: &[u8]) -> bool {
        match self {
            Self::Initial { state } => state.eq_bytes(expected),
            Self::Step { effect, .. } => effect.eq_bytes(expected),
        }
    }

    /// Whether this event is a step that stopped execution by `(return)`.
    #[must_use]
    pub const fn is_return_step(self) -> bool {
        match self {
            Self::Initial { .. } => false,
            Self::Step { effect, .. } => effect.is_return(),
        }
    }

    /// Materializes this borrowed event as a trace snapshot event.
    ///
    /// # Errors
    ///
    /// Returns `RunError::Limit` if the event bytes exceed
    /// `RunLimits::max_trace_snapshot_len`. Returns `RunError::Allocation` if
    /// snapshot allocation fails.
    pub fn to_snapshot(self, limits: RunLimits) -> Result<TraceSnapshotEvent<'program>, RunError> {
        ensure_trace_len(self.len(), limits)?;
        match self {
            Self::Initial { state } => Ok(TraceSnapshotEvent::Initial {
                state: state.to_vec_with_context(AllocationContext::TraceSnapshot)?,
            }),
            Self::Step { step, rule, effect } => Ok(TraceSnapshotEvent::Step {
                step,
                rule,
                effect: effect.to_snapshot(limits)?,
            }),
        }
    }
}

fn ensure_trace_len(len: usize, limits: RunLimits) -> Result<(), RunError> {
    if len > limits.max_trace_snapshot_len() {
        return Err(LimitError::trace_snapshot(limits.max_trace_snapshot_len(), len).into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::test_support::{TestFailure, TestResult, expect_event};
    use crate::{
        BorrowedTraceEvent, Program, RuleActionView, RunLimits, TraceSnapshotEffect,
        TraceSnapshotEvent, TracedRunError,
    };
    use std::vec::Vec;

    #[test]
    fn borrowed_trace_events_are_emitted_without_snapshots() -> TestResult {
        let program = Program::parse_str("a=b\nb=(return)ok")?;
        let mut seen = Vec::new();

        let result = program.run_with_borrowed_trace(b"a", RunLimits::new(10_000), |event| {
            seen.push((
                event.len(),
                event.eq_bytes(match event {
                    BorrowedTraceEvent::Initial { .. } => b"a".as_ref(),
                    BorrowedTraceEvent::Step { step: 1, .. } => b"b".as_ref(),
                    BorrowedTraceEvent::Step { .. } => b"ok".as_ref(),
                }),
            ));
        })?;

        assert_eq!(result.output(), b"ok");
        assert_eq!(seen.as_slice(), &[(1, true), (1, true), (2, true)]);
        Ok(())
    }

    #[test]
    fn trace_snapshot_events_are_emitted_without_core_stderr() -> TestResult {
        let program = Program::parse_str("a=b\nb=(return)ok")?;
        let mut events = Vec::new();
        let result = program.run_with_trace_snapshots(b"a", RunLimits::new(10_000), |event| {
            events.push(event);
        })?;

        assert_eq!(result.output(), b"ok");
        assert!(result.returned());
        assert_eq!(events.len(), 3);

        let initial = expect_event(&events, 0)?;
        let first_step = expect_event(&events, 1)?;
        let second_step = expect_event(&events, 2)?;

        assert!(matches!(initial, TraceSnapshotEvent::Initial { .. }));
        assert_eq!(initial.bytes(), b"a");
        assert_eq!(first_step.bytes(), b"b");
        assert_eq!(second_step.bytes(), b"ok");
        assert!(!first_step.is_return_step());
        assert!(second_step.is_return_step());

        match first_step {
            TraceSnapshotEvent::Step {
                rule,
                effect: TraceSnapshotEffect::Continue { state },
                ..
            } => {
                assert_eq!(state.as_slice(), b"b");
                assert_eq!(rule.position().zero_based(), 0);
                assert_eq!(rule.line_number(), 1);
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
        let result = program.try_run_with_trace_snapshots(b"a", RunLimits::new(10_000), |_event| {
            Err::<(), _>("trace sink full")
        });

        assert_eq!(result, Err(TracedRunError::Trace("trace sink full")));
        Ok(())
    }

    #[test]
    fn traced_final_event_matches_run_result() -> TestResult {
        let program = Program::parse_str("a=b\nb=(return)c")?;
        let mut events = Vec::new();

        let result = program.run_with_trace_snapshots(b"a", RunLimits::new(10), |event| {
            events.push(event);
        })?;

        let last = events
            .last()
            .ok_or(TestFailure::Message("expected final trace event"))?;
        assert_eq!(last.bytes(), result.output());
        assert_eq!(events.len(), result.steps() + 1);
        assert!(last.is_return_step());
        Ok(())
    }
}
