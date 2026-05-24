use crate::error::{TraceSnapshotError, TraceSnapshotRunError, TracedRunError};
use crate::input::RunSeed;
use crate::limits::TraceSnapshotByteLimit;
use crate::trace::{BorrowedTraceEvent, TraceSnapshotEvent};

use super::Program;
use super::result::RunResult;

/// Trace callback failure split used while borrowed events become snapshots.
enum SnapshotTraceCallbackError<E> {
    /// Snapshot materialization failed before the user callback ran.
    Snapshot(TraceSnapshotError),
    /// User callback rejected a materialized snapshot event.
    Trace(E),
}

impl Program {
    /// Runs this program and emits owned trace snapshot events.
    ///
    /// This API materializes bounded `Vec<u8>` snapshots for the initial state
    /// and every committed step. Use [`Program::run_with_borrowed_trace`] when
    /// the trace sink only needs to inspect each event during the callback.
    /// Snapshot limits are evaluated per event; a too-large event is reported
    /// as a snapshot failure before the user callback receives a truncated
    /// value.
    ///
    /// # Errors
    ///
    /// Returns `TraceSnapshotRunError::Run` for runtime failures.
    /// Returns `TraceSnapshotRunError::Snapshot` when snapshot materialization
    /// exceeds `snapshot_byte_limit` or allocation fails. Returns
    /// `TraceSnapshotRunError::Trace` when the user-provided trace callback
    /// returns an error.
    pub fn run_with_trace_snapshots<'program, F, E>(
        &'program self,
        seed: RunSeed,
        snapshot_byte_limit: TraceSnapshotByteLimit,
        mut trace: F,
    ) -> Result<RunResult, TraceSnapshotRunError<E>>
    where
        F: FnMut(TraceSnapshotEvent<'program>) -> Result<(), E>,
    {
        let result = self.run_with_borrowed_trace(seed, |event| {
            let snapshot = event
                .to_snapshot(snapshot_byte_limit)
                .map_err(SnapshotTraceCallbackError::Snapshot)?;
            trace(snapshot).map_err(SnapshotTraceCallbackError::Trace)
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

    /// Runs this program and emits borrowed trace events.
    ///
    /// The callback borrows event bytes only for the duration of each call. A
    /// sink that wants to keep bytes must copy them explicitly or use
    /// [`Program::run_with_trace_snapshots`]. Runtime failures and callback
    /// failures stay separate in [`TracedRunError`], so callback control flow
    /// cannot be mistaken for interpreter failure.
    ///
    /// # Errors
    ///
    /// Returns `TracedRunError::Run` for ordinary runtime failures. Returns
    /// `TracedRunError::Trace` when the user-provided trace callback returns an
    /// error.
    pub fn run_with_borrowed_trace<'program, F, E>(
        &'program self,
        seed: RunSeed,
        trace: F,
    ) -> Result<RunResult, TracedRunError<E>>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), E>,
    {
        crate::execution::run_with_borrowed_trace(self, seed, trace)
    }
}
