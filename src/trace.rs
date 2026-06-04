//! Borrowed and snapshot trace event types.
//!
//! Borrowed tracing observes runtime state during the callback without
//! materializing owned event snapshots. Snapshot tracing materializes bounded
//! owned bytes at the trace boundary. Both surfaces describe the same event
//! stream: the initial state followed by one event for each committed execution
//! step.
//!
//! Use borrowed events when a sink can decide immediately, and snapshot events
//! when a sink must retain state/output bytes after the callback returns.
//! Snapshot materialization is its own failure domain and its byte limit is
//! checked per event.
//!
//! ```
//! use rsaeb::trace::{SnapshotTrace, TraceSnapshotEffect, TraceSnapshotEvent};
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{
//!     DefaultParsePolicy, DefaultRuntimeInputPolicy, StaticExecutionPolicy,
//!     StaticTraceSnapshotPolicy,
//! };
//! use rsaeb::program::ExecutableProgram;
//!
//! type TenSteps = StaticExecutionPolicy<10, 16_777_216, 16_777_216>;
//! type SnapshotBytes = StaticTraceSnapshotPolicy<16_777_216>;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let executable = ExecutableProgram::<DefaultParsePolicy>::parse_text("a=b\nb=(return)ok")?;
//! let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
//! let admitted = input.admit::<TenSteps>()?;
//! let mut retained = Vec::new();
//!
//! executable.trace(admitted, SnapshotTrace::<SnapshotBytes, _>::new(|event| {
//!         match event {
//!             TraceSnapshotEvent::Initial { state } => retained.push(state.into_raw_bytes()),
//!             TraceSnapshotEvent::Step {
//!                 effect: TraceSnapshotEffect::Continue { state },
//!                 ..
//!             } => retained.push(state.into_raw_bytes()),
//!             TraceSnapshotEvent::Step {
//!                 effect: TraceSnapshotEffect::Return { output },
//!                 ..
//!             } => retained.push(output.into_raw_bytes()),
//!         }
//!         Ok::<(), core::convert::Infallible>(())
//!     }))?;
//!
//! if retained != [b"a".to_vec(), b"b".to_vec(), b"ok".to_vec()] {
//!     return Err("unexpected trace snapshots".into());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ```
//! use core::convert::Infallible;
//! use rsaeb::error::{TraceSnapshotError, TraceSnapshotRunError};
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{
//!     DefaultParsePolicy, DefaultRuntimeInputPolicy, StaticExecutionPolicy,
//!     StaticTraceSnapshotPolicy,
//! };
//! use rsaeb::program::ExecutableProgram;
//! use rsaeb::trace::SnapshotTrace;
//!
//! type TenSteps = StaticExecutionPolicy<10, 16_777_216, 16_777_216>;
//! type EmptySnapshot = StaticTraceSnapshotPolicy<0>;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let executable = ExecutableProgram::<DefaultParsePolicy>::parse_text("a=b")?;
//! let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
//! let admitted = input.admit::<TenSteps>()?;
//!
//! let result = executable.trace(
//!     admitted,
//!     SnapshotTrace::<EmptySnapshot, _>::new(|_event| Ok::<(), Infallible>(())),
//! );
//!
//! if !matches!(
//!     result,
//!     Err(TraceSnapshotRunError::Snapshot(TraceSnapshotError::Limit {
//!         attempted_len,
//!         ..
//!     })) if attempted_len.get() == 1
//! ) {
//!     return Err("unexpected trace snapshot limit error".into());
//! }
//! # Ok(())
//! # }
//! ```

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::bytes::{RuntimeByte, RuntimeStateByteCount, TraceSnapshotByteCount};
use core::marker::PhantomData;

use crate::error::{TraceSnapshotError, TraceSnapshotRunError, TracedRunError};
use crate::input::AdmittedRun;
use crate::inspect::RuleView;
use crate::limits::{StepCount, TraceSnapshotByteLimit};
use crate::policy::{ExecutionPolicy, ParsePolicy, TraceSnapshotPolicy};
use crate::program::limits::TraceSnapshotBytePermit;
use crate::program::{
    ExecutableProgramRef, ReturnOutput, ReturnOutputView, RunResult, RuntimeStateSnapshot,
};
use alloc::vec::Vec;

/// Sealed implementation detail for trace request types.
mod request_sealed {
    /// Private supertrait that keeps trace requests closed over crate-defined wrappers.
    pub trait Sealed {}
}

/// Borrowed trace request carrying a user callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BorrowedTrace<F> {
    /// Callback receiving borrowed trace events.
    callback: F,
}

/// Snapshot trace request carrying a user callback and snapshot policy type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotTrace<T: TraceSnapshotPolicy, F> {
    /// Callback receiving materialized snapshot events.
    callback: F,
    /// Trace snapshot policy selected by this request.
    policy: PhantomData<fn() -> T>,
}

/// Trace request accepted by executable-program trace entrypoints.
///
/// Implementations exist only for crate-defined request wrappers, so callers
/// choose borrowed or snapshot tracing by type instead of by a runtime selector.
pub trait TraceRequest<'program, P: ParsePolicy, E: ExecutionPolicy>:
    request_sealed::Sealed
{
    /// Error type produced by this request.
    type Error;

    /// Runs the trace request.
    ///
    /// # Errors
    ///
    /// Returns this request's error type if runtime execution, snapshot
    /// materialization, or the user callback fails.
    fn trace(
        self,
        executable: ExecutableProgramRef<'program, P>,
        admitted: AdmittedRun<E>,
    ) -> Result<RunResult, Self::Error>;
}

/// Trace callback failure split used while borrowed events become snapshots.
enum SnapshotTraceCallbackError<E> {
    /// Snapshot materialization failed before the user callback ran.
    Snapshot(TraceSnapshotError),
    /// User callback rejected a materialized snapshot event.
    Trace(E),
}

impl<F> BorrowedTrace<F> {
    /// Builds a borrowed trace request from a callback.
    #[must_use]
    pub fn new<'program, TraceError>(callback: F) -> Self
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), TraceError>,
    {
        Self { callback }
    }
}

impl<T: TraceSnapshotPolicy, F> SnapshotTrace<T, F> {
    /// Builds a snapshot trace request from a callback.
    #[must_use]
    pub fn new<'program, TraceError>(callback: F) -> Self
    where
        F: FnMut(TraceSnapshotEvent<'program>) -> Result<(), TraceError>,
    {
        Self {
            callback,
            policy: PhantomData,
        }
    }
}

impl<F> request_sealed::Sealed for BorrowedTrace<F> {}

impl<T: TraceSnapshotPolicy, F> request_sealed::Sealed for SnapshotTrace<T, F> {}

impl<'program, P, E, F, TraceError> TraceRequest<'program, P, E> for BorrowedTrace<F>
where
    P: ParsePolicy,
    E: ExecutionPolicy,
    F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), TraceError>,
{
    type Error = TracedRunError<TraceError>;

    fn trace(
        self,
        executable: ExecutableProgramRef<'program, P>,
        admitted: AdmittedRun<E>,
    ) -> Result<RunResult, Self::Error> {
        crate::execution::trace_events(executable, admitted, self.callback)
    }
}

impl<'program, P, E, T, F, TraceError> TraceRequest<'program, P, E> for SnapshotTrace<T, F>
where
    P: ParsePolicy,
    E: ExecutionPolicy,
    T: TraceSnapshotPolicy,
    F: FnMut(TraceSnapshotEvent<'program>) -> Result<(), TraceError>,
{
    type Error = TraceSnapshotRunError<TraceError>;

    fn trace(
        self,
        executable: ExecutableProgramRef<'program, P>,
        admitted: AdmittedRun<E>,
    ) -> Result<RunResult, Self::Error> {
        let Self {
            mut callback,
            policy: _policy,
        } = self;

        let result = crate::execution::trace_events(executable, admitted, |event| {
            let snapshot = event
                .to_snapshot::<T>()
                .map_err(SnapshotTraceCallbackError::Snapshot)?;
            callback(snapshot).map_err(SnapshotTraceCallbackError::Trace)
        });

        match result {
            Ok(result) => Ok(result),
            Err(TracedRunError::Run(error)) => Err(TraceSnapshotRunError::Run(error)),
            Err(TracedRunError::Trace(SnapshotTraceCallbackError::Snapshot(error))) => {
                Err(TraceSnapshotRunError::Snapshot(error))
            }
            Err(TracedRunError::Trace(SnapshotTraceCallbackError::Trace(error))) => {
                Err(TraceSnapshotRunError::Trace(error))
            }
        }
    }
}

/// Borrowed view of runtime-state bytes.
///
/// This lets trace sinks inspect state without forcing the runtime to allocate a
/// `Vec<u8>` for every event. Internally the runtime state is not stored as raw
/// `u8`, so public byte access is an iterator/materialization boundary. The
/// view is valid only while the runtime state it borrows is held by the current
/// execution or trace callback. Retaining bytes after that boundary requires
/// [`RuntimeStateView::materialize`] or snapshot tracing.
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

    /// Materializes this runtime-state view after its trace snapshot limit was admitted.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the output buffer cannot be allocated.
    pub(crate) fn to_vec_with_trace_permit(
        self,
        context: AllocationContext,
        permit: TraceSnapshotBytePermit,
    ) -> Result<Vec<u8>, AllocationError> {
        let mut output = Vec::new();
        try_reserve_total_exact(
            &mut output,
            RequestedCapacity::new(permit.byte_count().get()),
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
        RuntimeStateSnapshot::from_runtime_state_view(self)
    }
}

impl core::fmt::Debug for RuntimeStateView<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list()
            .entries((*self).materialized_bytes())
            .finish()
    }
}

/// Borrowed trace effect emitted by borrowed tracing APIs.
///
/// Borrowed effects avoid allocation by borrowing the post-step state or parsed
/// return payload for the duration of the callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowedTraceEffect<'program, 'run> {
    /// The step produced the next runtime state and execution may continue.
    Continue {
        /// Runtime state after a non-terminal applied step.
        state: RuntimeStateView<'run>,
    },
    /// The step executed `(return)` and produced final output bytes.
    Return {
        /// `(return)` output bytes.
        output: ReturnOutputView<'program>,
    },
}

/// Owned trace effect emitted by trace snapshot APIs.
///
/// Continuation steps materialize the post-step runtime state. Return steps
/// materialize the final `(return)` output instead of a state.
#[derive(Debug, PartialEq, Eq)]
pub enum TraceSnapshotEffect {
    /// The step produced the next runtime state and execution may continue.
    Continue {
        /// Runtime state after a non-terminal applied step.
        state: RuntimeStateSnapshot,
    },
    /// The step executed `(return)` and produced final output bytes.
    Return {
        /// `(return)` output bytes.
        output: ReturnOutput,
    },
}

impl BorrowedTraceEffect<'_, '_> {
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
    fn to_snapshot<T: TraceSnapshotPolicy>(
        self,
    ) -> Result<TraceSnapshotEffect, TraceSnapshotError> {
        let permit = ensure_trace_len(self.byte_count(), T::TRACE_SNAPSHOT_BYTE_LIMIT)?;
        match self {
            Self::Continue { state } => Ok(TraceSnapshotEffect::Continue {
                state: RuntimeStateSnapshot::from_trace_state_view(state, permit)?,
            }),
            Self::Return { output } => Ok(TraceSnapshotEffect::Return {
                output: ReturnOutput::from_trace_return_output_view(output, permit)?,
            }),
        }
    }
}

/// Trace event emitted by borrowed tracing APIs.
///
/// The event borrows runtime bytes only for the duration of the callback. This
/// API does not materialize owned event snapshots; snapshot tracing is derived
/// from it by materializing snapshots under an explicit
/// [`TraceSnapshotByteLimit`]. The event also borrows parsed rule views from
/// the parsed program, so it cannot become a retained log record without an
/// explicit copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowedTraceEvent<'program, 'run> {
    /// Initial runtime state before any execution step.
    Initial {
        /// Initial runtime state.
        state: RuntimeStateView<'run>,
    },
    /// One applied rule.
    Step {
        /// One-based applied step count.
        step: StepCount,
        /// Structured view of the applied rule.
        rule: RuleView<'program>,
        /// Structured result of the execution step.
        effect: BorrowedTraceEffect<'program, 'run>,
    },
}

/// Trace event emitted by trace snapshot APIs.
///
/// State and return-output bytes are materialized as owned `Vec<u8>` snapshots.
/// Return steps cannot be confused with ordinary continuation steps by
/// forgetting to inspect a boolean flag. Parsed rule views still borrow from
/// the program so callers retain bytes, not an independent copy of rule
/// metadata.
#[derive(Debug, PartialEq, Eq)]
pub enum TraceSnapshotEvent<'program> {
    /// Initial runtime state before any execution step.
    Initial {
        /// Initial runtime state.
        state: RuntimeStateSnapshot,
    },
    /// One applied rule.
    Step {
        /// One-based applied step count.
        step: StepCount,
        /// Structured view of the applied rule.
        rule: RuleView<'program>,
        /// Structured result of the execution step.
        effect: TraceSnapshotEffect,
    },
}

impl<'program> BorrowedTraceEvent<'program, '_> {
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
    pub fn to_snapshot<T: TraceSnapshotPolicy>(
        self,
    ) -> Result<TraceSnapshotEvent<'program>, TraceSnapshotError> {
        match self {
            Self::Initial { state } => {
                let permit = ensure_trace_len(
                    TraceSnapshotByteCount::from_runtime_state_count(state.byte_count()),
                    T::TRACE_SNAPSHOT_BYTE_LIMIT,
                )?;
                Ok(TraceSnapshotEvent::Initial {
                    state: RuntimeStateSnapshot::from_trace_state_view(state, permit)?,
                })
            }
            Self::Step { step, rule, effect } => Ok(TraceSnapshotEvent::Step {
                step,
                rule,
                effect: effect.to_snapshot::<T>()?,
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
) -> Result<TraceSnapshotBytePermit, TraceSnapshotError> {
    limit.admit(len).ok_or(TraceSnapshotError::Limit {
        limit,
        attempted_len: len,
    })
}
