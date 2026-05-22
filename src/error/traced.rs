use core::error::Error;

use crate::allocation::AllocationError;
use crate::bytes::TraceSnapshotByteCount;
use crate::program::TraceSnapshotByteLimit;

use super::RunError;

/// Error returned by fallible borrowed tracing APIs.
///
/// Borrowed tracing itself does not materialize snapshots, so the only domains
/// are runtime execution and the user-provided trace sink.
#[derive(Debug, PartialEq, Eq)]
pub enum TracedRunError<E> {
    /// Runtime execution failed.
    Run(RunError),
    /// The user-provided trace sink failed.
    Trace(E),
}

impl<E> Error for TracedRunError<E>
where
    E: Error + 'static,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Run(error) => Some(error),
            Self::Trace(error) => Some(error),
        }
    }
}

impl<E> From<RunError> for TracedRunError<E> {
    fn from(value: RunError) -> Self {
        Self::Run(value)
    }
}

/// Error while materializing an owned trace snapshot from a borrowed trace event.
///
/// Snapshot limits are evaluated per event. A failing event is not silently
/// truncated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceSnapshotError {
    /// Snapshot bytes exceeded the caller-provided snapshot byte limit.
    Limit {
        /// Configured maximum trace snapshot byte length.
        limit: TraceSnapshotByteLimit,
        /// Snapshot length that would have been allocated.
        attempted_len: TraceSnapshotByteCount,
    },
    /// Snapshot byte materialization failed.
    Allocation(AllocationError),
}

impl Error for TraceSnapshotError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Limit { .. } => None,
            Self::Allocation(error) => Some(error),
        }
    }
}

impl From<AllocationError> for TraceSnapshotError {
    fn from(value: AllocationError) -> Self {
        Self::Allocation(value)
    }
}

/// Error returned by trace-snapshot APIs.
///
/// Runtime execution, snapshot materialization, and caller callback failures
/// remain separate domains.
#[derive(Debug, PartialEq, Eq)]
pub enum TraceSnapshotRunError<E> {
    /// Runtime execution failed.
    Run(RunError),
    /// Trace snapshot materialization failed.
    Snapshot(TraceSnapshotError),
    /// The user-provided trace sink failed.
    Trace(E),
}

impl<E> Error for TraceSnapshotRunError<E>
where
    E: Error + 'static,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Run(error) => Some(error),
            Self::Snapshot(error) => Some(error),
            Self::Trace(error) => Some(error),
        }
    }
}

impl<E> From<RunError> for TraceSnapshotRunError<E> {
    fn from(value: RunError) -> Self {
        Self::Run(value)
    }
}

impl<E> From<TraceSnapshotError> for TraceSnapshotRunError<E> {
    fn from(value: TraceSnapshotError) -> Self {
        Self::Snapshot(value)
    }
}
