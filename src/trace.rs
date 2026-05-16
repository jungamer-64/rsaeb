//! Borrowed and snapshot trace event types.
//!
//! Borrowed tracing observes runtime state during the callback without
//! allocating per event. Snapshot tracing materializes bounded owned bytes at
//! the trace boundary. Both surfaces describe the same event stream: the
//! initial state followed by one event for each committed rewrite step.
//!
//! Use borrowed events when a sink can decide immediately, and snapshot events
//! when a sink must retain state/output bytes after the callback returns.
//!
//! ```
//! use rsaeb::limits::{
//!     DEFAULT_MAX_INPUT_LEN, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN,
//!     DEFAULT_MAX_TRACE_SNAPSHOT_LEN, StepLimit, TraceSnapshotLimits,
//! };
//! use rsaeb::trace::{TraceSnapshotEffect, TraceSnapshotEvent};
//! use rsaeb::{Program, ProgramSource, RunLimits, RuntimeInput};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::parse(ProgramSource::from_str("a=b\nb=(return)ok"))?;
//! let input = RuntimeInput::validate(b"a", DEFAULT_MAX_INPUT_LEN)?;
//! let run_limits = RunLimits::new(
//!     StepLimit::new(10),
//!     DEFAULT_MAX_STATE_LEN,
//!     DEFAULT_MAX_RETURN_LEN,
//! );
//! let trace_limits = TraceSnapshotLimits::new(run_limits, DEFAULT_MAX_TRACE_SNAPSHOT_LEN);
//! let mut retained = Vec::new();
//!
//! program.run_with_trace_snapshots(&input, trace_limits, |event| match event {
//!     TraceSnapshotEvent::Initial { state } => retained.push(state.into_vec()),
//!     TraceSnapshotEvent::Step {
//!         effect: TraceSnapshotEffect::Continue { state },
//!         ..
//!     } => retained.push(state.into_vec()),
//!     TraceSnapshotEvent::Step {
//!         effect: TraceSnapshotEffect::Return { output },
//!         ..
//!     } => retained.push(output.into_vec()),
//! })?;
//!
//! assert_eq!(retained, [b"a".to_vec(), b"b".to_vec(), b"ok".to_vec()]);
//! # Ok(())
//! # }
//! ```

use alloc::vec::Vec;

use crate::allocation::{AllocationContext, AllocationError, try_push, try_reserve_total_exact};
use crate::bytes::{RuntimeByte, RuntimeStateByteCount, TraceSnapshotByteCount};
use crate::error::TraceSnapshotError;
use crate::inspect::{PayloadView, RuleView};
use crate::program::{ReturnOutput, RuntimeStateSnapshot, StepCount, TraceSnapshotByteLimit};

/// Borrowed view of runtime-state bytes.
///
/// This lets trace sinks inspect state without forcing the runtime to allocate a
/// `Vec<u8>` for every event. Internally the runtime state is not stored as raw
/// `u8`, so public byte access is an iterator/materialization boundary. The
/// view is valid only while the runtime state it borrows is held by the current
/// execution or trace callback.
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
    ///
    /// Iteration converts the internal runtime byte domain back into public raw
    /// bytes without allocating.
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

    /// Materializes this runtime-state view at the given allocation site.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the output buffer cannot be allocated.
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
///
/// Continuation steps materialize the post-step runtime state. Return steps
/// materialize the final `(return)` output instead of a state.
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
///
/// Borrowed effects avoid allocation by borrowing the post-step state or parsed
/// return payload for the duration of the callback.
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

    /// Materializes this borrowed trace effect into an owned snapshot effect.
    ///
    /// # Errors
    ///
    /// Returns `TraceSnapshotError` if the effect exceeds `limit` or snapshot
    /// allocation fails.
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
                    output.to_vec_with_context(AllocationContext::TraceSnapshot)?,
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
/// from it by materializing snapshots under an explicit
/// [`TraceSnapshotByteLimit`].
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
        match self {
            Self::Initial { state } => {
                ensure_trace_len(TraceSnapshotByteCount::new(state.byte_count().get()), limit)?;
                Ok(TraceSnapshotEvent::Initial {
                    state: RuntimeStateSnapshot::from_vec(
                        state.to_vec_with_context(AllocationContext::TraceSnapshot)?,
                    ),
                })
            }
            Self::Step { step, rule, effect } => Ok(TraceSnapshotEvent::Step {
                step,
                rule,
                effect: effect.to_snapshot(limit)?,
            }),
        }
    }
}

/// Checks whether a trace snapshot byte count is within its limit.
///
/// # Errors
///
/// Returns `TraceSnapshotError::Limit` if `len` exceeds `limit`.
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
