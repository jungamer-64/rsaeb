use core::error::Error;

use crate::allocation::AllocationError;
use crate::bytes::TraceSnapshotByteCount;
use crate::program::TraceSnapshotByteLimit;

use super::RunError;

/// Error returned by fallible tracing APIs.
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

/// Error returned by infallible trace-snapshot APIs.
#[derive(Debug, PartialEq, Eq)]
pub enum TraceSnapshotRunError {
    /// Runtime execution failed.
    Run(RunError),
    /// Trace snapshot materialization failed.
    Snapshot(TraceSnapshotError),
}

impl Error for TraceSnapshotRunError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Run(error) => Some(error),
            Self::Snapshot(error) => Some(error),
        }
    }
}

impl From<RunError> for TraceSnapshotRunError {
    fn from(value: RunError) -> Self {
        Self::Run(value)
    }
}

impl From<TraceSnapshotError> for TraceSnapshotRunError {
    fn from(value: TraceSnapshotError) -> Self {
        Self::Snapshot(value)
    }
}

/// Error returned by fallible trace-snapshot APIs.
#[derive(Debug, PartialEq, Eq)]
pub enum FallibleTraceSnapshotRunError<E> {
    /// Runtime execution failed.
    Run(RunError),
    /// Trace snapshot materialization failed.
    Snapshot(TraceSnapshotError),
    /// The user-provided trace sink failed.
    Trace(E),
}

impl<E> Error for FallibleTraceSnapshotRunError<E>
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

impl<E> From<RunError> for FallibleTraceSnapshotRunError<E> {
    fn from(value: RunError) -> Self {
        Self::Run(value)
    }
}

impl<E> From<TraceSnapshotError> for FallibleTraceSnapshotRunError<E> {
    fn from(value: TraceSnapshotError) -> Self {
        Self::Snapshot(value)
    }
}
