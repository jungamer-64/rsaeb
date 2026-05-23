//! Borrowed and snapshot trace event types.
//!
//! Borrowed tracing observes runtime state during the callback without
//! materializing owned event snapshots. Snapshot tracing materializes bounded
//! owned bytes at the trace boundary. Both surfaces describe the same event
//! stream: the initial state followed by one event for each committed rewrite
//! step.
//!
//! Use borrowed events when a sink can decide immediately, and snapshot events
//! when a sink must retain state/output bytes after the callback returns.
//!
//! ```
//! use rsaeb::limits::{
//!     DEFAULT_MAX_INPUT_LEN, DEFAULT_PARSE_LIMITS, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN,
//!     DEFAULT_MAX_TRACE_SNAPSHOT_LEN, StepLimit,
//! };
//! use rsaeb::trace::{TraceSnapshotEffect, TraceSnapshotEvent};
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::input::RunSeed;
//! use rsaeb::limits::{ExecutionLimits, RuntimeInputLimits};
//! use rsaeb::program::Program;
//! use rsaeb::source::ProgramSource;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::parse(ProgramSource::from_text("a=b\nb=(return)ok"), DEFAULT_PARSE_LIMITS)?;
//! let input_limits = RuntimeInputLimits::new(DEFAULT_MAX_INPUT_LEN);
//! let execution_limits = ExecutionLimits::new(
//!     StepLimit::new(10),
//!     DEFAULT_MAX_STATE_LEN,
//!     DEFAULT_MAX_RETURN_LEN,
//! );
//! let input = RuntimeInput::validate(RuntimeInputSource::from_bytes(b"a"), input_limits)?;
//! let seed = RunSeed::admit(input, execution_limits)?;
//! let mut retained = Vec::new();
//!
//! program.run_with_trace_snapshots(seed, DEFAULT_MAX_TRACE_SNAPSHOT_LEN, |event| {
//!     match event {
//!         TraceSnapshotEvent::Initial { state } => retained.push(state.into_raw_bytes()),
//!         TraceSnapshotEvent::Step {
//!             effect: TraceSnapshotEffect::Continue { state },
//!             ..
//!         } => retained.push(state.into_raw_bytes()),
//!         TraceSnapshotEvent::Step {
//!             effect: TraceSnapshotEffect::Return { output },
//!             ..
//!         } => retained.push(output.into_raw_bytes()),
//!     }
//!     Ok::<(), core::convert::Infallible>(())
//! })?;
//!
//! assert_eq!(retained, [b"a".to_vec(), b"b".to_vec(), b"ok".to_vec()]);
//! # Ok(())
//! # }
//! ```

use alloc::vec::Vec;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::bytes::{RuntimeByte, RuntimeStateByteCount, TraceSnapshotByteCount};
use crate::error::TraceSnapshotError;
use crate::inspect::RuleView;
use crate::limits::{StepCount, TraceSnapshotByteLimit};
use crate::program::{ReturnOutput, ReturnOutputView, RuntimeStateSnapshot};

/// Borrowed view of runtime-state bytes.
///
/// This lets trace sinks inspect state without forcing the runtime to allocate a
/// `Vec<u8>` for every event. Internally the runtime state is not stored as raw
/// `u8`, so public byte access is an iterator/materialization boundary. The
/// view is valid only while the runtime state it borrows is held by the current
/// execution or trace callback.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RuntimeStateView<'run> {
    /// Runtime-domain bytes borrowed from the current execution state.
    bytes: &'run [RuntimeByte],
}

impl<'run> RuntimeStateView<'run> {
    /// Borrows runtime-state bytes at an execution or trace boundary.
    pub(crate) const fn new(bytes: &'run [RuntimeByte]) -> Self {
        Self { bytes }
    }

    /// Whether the state is empty.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.bytes.is_empty()
    }

    /// Returns materialized runtime bytes.
    pub(crate) fn materialized_bytes(self) -> impl Iterator<Item = u8> + 'run {
        self.bytes.iter().copied().map(RuntimeByte::materialize)
    }

    /// Runtime state length in bytes.
    #[must_use]
    pub const fn byte_count(self) -> RuntimeStateByteCount {
        RuntimeStateByteCount::new(self.bytes.len())
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
        try_reserve_total_exact(
            &mut output,
            RequestedCapacity::from_runtime_state_count(self.byte_count()),
            context,
        )?;
        for byte in self.materialized_bytes() {
            try_push(&mut output, byte, context)?;
        }
        Ok(output)
    }

    /// Materializes this borrowed runtime state into a typed owned snapshot.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the output buffer cannot be allocated.
    pub fn materialize(self) -> Result<RuntimeStateSnapshot, AllocationError> {
        Ok(RuntimeStateSnapshot::from_materialized(
            self.to_vec_with_context(AllocationContext::RuntimeStateView)?,
        ))
    }
}

impl core::fmt::Debug for RuntimeStateView<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list()
            .entries((*self).materialized_bytes())
            .finish()
    }
}

/// Trace effect emitted by step trace events.
///
/// `State` and `Output` decide whether the effect borrows runtime bytes or owns
/// materialized snapshots. The effect semantics are otherwise single-sourced:
/// a step either continues with a runtime state or returns an output payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceEffect<State, Output> {
    /// The step produced the next runtime state and execution may continue.
    Continue {
        /// Runtime state after the rewrite step.
        state: State,
    },
    /// The step executed `(return)` and produced final output bytes.
    Return {
        /// `(return)` output bytes.
        output: Output,
    },
}

/// Borrowed trace effect emitted by borrowed tracing APIs.
///
/// Borrowed effects avoid allocation by borrowing the post-step state or parsed
/// return payload for the duration of the callback.
pub type BorrowedTraceEffect<'program, 'run> =
    TraceEffect<RuntimeStateView<'run>, ReturnOutputView<'program>>;

/// Owned trace effect emitted by trace snapshot APIs.
///
/// Continuation steps materialize the post-step runtime state. Return steps
/// materialize the final `(return)` output instead of a state.
pub type TraceSnapshotEffect = TraceEffect<RuntimeStateSnapshot, ReturnOutput>;

impl TraceEffect<RuntimeStateView<'_>, ReturnOutputView<'_>> {
    /// Byte length that would be materialized by snapshot tracing.
    #[must_use]
    pub fn byte_count(self) -> TraceSnapshotByteCount {
        match self {
            Self::Continue { state } => {
                TraceSnapshotByteCount::from_runtime_state_count(state.byte_count())
            }
            Self::Return { output } => {
                TraceSnapshotByteCount::from_return_output_count(output.byte_count())
            }
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
                state: RuntimeStateSnapshot::from_materialized(
                    state.to_vec_with_context(AllocationContext::TraceSnapshot)?,
                ),
            }),
            Self::Return { output } => Ok(TraceSnapshotEffect::Return {
                output: ReturnOutput::from_materialized(
                    output.to_vec_with_context(AllocationContext::TraceSnapshot)?,
                ),
            }),
        }
    }
}

/// Trace event emitted by tracing APIs.
///
/// `State` and `Effect` decide whether event bytes are borrowed for the
/// callback or materialized into owned snapshots. Step events always borrow the
/// structured rule view from `Program`, so they cannot outlive the parsed
/// program they describe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceEvent<'program, State, Effect> {
    /// Initial runtime state before any rewrite step.
    Initial {
        /// Initial runtime state.
        state: State,
    },
    /// One applied rule.
    Step {
        /// One-based applied step count.
        step: StepCount,
        /// Structured view of the applied rule.
        rule: RuleView<'program>,
        /// Structured result of the rewrite step.
        effect: Effect,
    },
}

/// Trace event emitted by borrowed tracing APIs.
///
/// The event borrows runtime bytes only for the duration of the callback. This
/// API does not materialize owned event snapshots; snapshot tracing is derived
/// from it by materializing snapshots under an explicit
/// [`TraceSnapshotByteLimit`].
pub type BorrowedTraceEvent<'program, 'run> =
    TraceEvent<'program, RuntimeStateView<'run>, BorrowedTraceEffect<'program, 'run>>;

/// Trace event emitted by trace snapshot APIs.
///
/// State and return-output bytes are materialized as owned `Vec<u8>` snapshots.
/// Return steps cannot be confused with ordinary continuation steps by
/// forgetting to inspect a boolean flag.
pub type TraceSnapshotEvent<'program> =
    TraceEvent<'program, RuntimeStateSnapshot, TraceSnapshotEffect>;

impl<'program> TraceEvent<'program, RuntimeStateView<'_>, BorrowedTraceEffect<'program, '_>> {
    /// Byte length that would be materialized by snapshot tracing.
    #[must_use]
    pub fn byte_count(self) -> TraceSnapshotByteCount {
        match self {
            Self::Initial { state } => {
                TraceSnapshotByteCount::from_runtime_state_count(state.byte_count())
            }
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
                ensure_trace_len(
                    TraceSnapshotByteCount::from_runtime_state_count(state.byte_count()),
                    limit,
                )?;
                Ok(TraceSnapshotEvent::Initial {
                    state: RuntimeStateSnapshot::from_materialized(
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
    if !limit.accepts(len) {
        return Err(TraceSnapshotError::Limit {
            limit,
            attempted_len: len,
        });
    }

    Ok(())
}
