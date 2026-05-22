use crate::error::{TraceSnapshotError, TraceSnapshotRunError, TracedRunError};
use crate::execution::RunSession;
use crate::runtime::RuntimeInput;
use crate::trace::{BorrowedTraceEvent, TraceSnapshotEvent};

use super::Program;
use super::limits::{RunLimits, TraceSnapshotLimits};
use super::result::RunResult;

enum SnapshotTraceCallbackError<E> {
    Snapshot(TraceSnapshotError),
    Trace(E),
}

impl Program {
    /// Runs this program and emits owned trace snapshot events.
    ///
    /// This API materializes bounded `Vec<u8>` snapshots for the initial state
    /// and every committed step. Use [`Program::run_with_borrowed_trace`] when
    /// the trace sink only needs to inspect each event during the callback.
    ///
    /// # Errors
    ///
    /// Returns `TraceSnapshotRunError::Run` for runtime failures.
    /// Returns `TraceSnapshotRunError::Snapshot` when snapshot materialization
    /// exceeds `limits.snapshot_byte_limit()` or allocation fails. Returns
    /// `TraceSnapshotRunError::Trace` when the user-provided trace callback
    /// returns an error.
    pub fn run_with_trace_snapshots<'program, F, E>(
        &'program self,
        input: RuntimeInput,
        limits: TraceSnapshotLimits,
        mut trace: F,
    ) -> Result<RunResult, TraceSnapshotRunError<E>>
    where
        F: FnMut(TraceSnapshotEvent<'program>) -> Result<(), E>,
    {
        let result = self.run_with_borrowed_trace(input, limits.run_limits(), |event| {
            let snapshot = event
                .to_snapshot(limits.snapshot_byte_limit())
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
    /// [`Program::run_with_trace_snapshots`].
    ///
    /// # Errors
    ///
    /// Returns `TracedRunError::Run` for ordinary runtime failures. Returns
    /// `TracedRunError::Trace` when the user-provided trace callback returns an
    /// error.
    pub fn run_with_borrowed_trace<'program, F, E>(
        &'program self,
        input: RuntimeInput,
        limits: RunLimits,
        trace: F,
    ) -> Result<RunResult, TracedRunError<E>>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), E>,
    {
        RunSession::new(self, input, limits)
            .map_err(TracedRunError::Run)?
            .run_with_borrowed_trace(trace)
    }
}
