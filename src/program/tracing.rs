use core::convert::Infallible;

use crate::error::{
    FallibleTraceSnapshotRunError, RunError, TraceSnapshotError, TraceSnapshotRunError,
    TracedRunError,
};
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
    /// Runs this program and emits trace-snapshot, infallible events.
    ///
    /// This convenience API materializes `Vec<u8>` snapshots. Use
    /// `run_with_borrowed_trace` when the trace sink only needs to inspect each
    /// event during the callback.
    ///
    /// # Errors
    ///
    /// Returns `TraceSnapshotRunError::Run` for ordinary runtime failures.
    /// Returns `TraceSnapshotRunError::Snapshot` when snapshot materialization
    /// exceeds `limits.snapshot_byte_limit()` or allocation fails.
    pub fn run_with_trace_snapshots<'program, F>(
        &'program self,
        input: &RuntimeInput,
        limits: TraceSnapshotLimits,
        mut trace: F,
    ) -> Result<RunResult, TraceSnapshotRunError>
    where
        F: FnMut(TraceSnapshotEvent<'program>),
    {
        match self.try_run_with_trace_snapshots(input, limits, |event| {
            trace(event);
            Ok::<(), Infallible>(())
        }) {
            Ok(result) => Ok(result),
            Err(FallibleTraceSnapshotRunError::Run(error)) => {
                Err(TraceSnapshotRunError::Run(error))
            }
            Err(FallibleTraceSnapshotRunError::Snapshot(error)) => {
                Err(TraceSnapshotRunError::Snapshot(error))
            }
            Err(FallibleTraceSnapshotRunError::Trace(error)) => match error {},
        }
    }

    /// Runs this program and emits trace-snapshot, fallible events.
    ///
    /// # Errors
    ///
    /// Returns `FallibleTraceSnapshotRunError::Run` for runtime failures.
    /// Returns `FallibleTraceSnapshotRunError::Snapshot` for snapshot
    /// materialization failures. Returns
    /// `FallibleTraceSnapshotRunError::Trace` when the user-provided trace
    /// callback returns an error.
    pub fn try_run_with_trace_snapshots<'program, F, E>(
        &'program self,
        input: &RuntimeInput,
        limits: TraceSnapshotLimits,
        mut trace: F,
    ) -> Result<RunResult, FallibleTraceSnapshotRunError<E>>
    where
        F: FnMut(TraceSnapshotEvent<'program>) -> Result<(), E>,
    {
        let result = self.try_run_with_borrowed_trace(input, limits.run_limits(), |event| {
            let snapshot = event
                .to_snapshot(limits.snapshot_byte_limit())
                .map_err(SnapshotTraceCallbackError::Snapshot)?;
            trace(snapshot).map_err(SnapshotTraceCallbackError::Trace)
        });

        match result {
            Ok(result) => Ok(result),
            Err(TracedRunError::Run(error)) => Err(FallibleTraceSnapshotRunError::Run(error)),
            Err(TracedRunError::Trace(SnapshotTraceCallbackError::Snapshot(error))) => {
                Err(FallibleTraceSnapshotRunError::Snapshot(error))
            }
            Err(TracedRunError::Trace(SnapshotTraceCallbackError::Trace(error))) => {
                Err(FallibleTraceSnapshotRunError::Trace(error))
            }
        }
    }

    /// Runs this program and emits borrowed, infallible trace events.
    ///
    /// Borrowed trace events allocate nothing. They are valid only for the
    /// callback invocation, so a sink that wants to retain bytes must copy them
    /// explicitly.
    ///
    /// # Errors
    ///
    /// Returns `RunError` for the same runtime failures as `Program::run`.
    pub fn run_with_borrowed_trace<'program, F>(
        &'program self,
        input: &RuntimeInput,
        limits: RunLimits,
        mut trace: F,
    ) -> Result<RunResult, RunError>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>),
    {
        match self.try_run_with_borrowed_trace(input, limits, |event| {
            trace(event);
            Ok::<(), Infallible>(())
        }) {
            Ok(result) => Ok(result),
            Err(TracedRunError::Run(error)) => Err(error),
            Err(TracedRunError::Trace(error)) => match error {},
        }
    }

    /// Runs this program and emits borrowed, fallible trace events.
    ///
    /// # Errors
    ///
    /// Returns `TracedRunError::Run` for ordinary runtime failures. Returns
    /// `TracedRunError::Trace` when the user-provided trace callback returns an
    /// error.
    pub fn try_run_with_borrowed_trace<'program, F, E>(
        &'program self,
        input: &RuntimeInput,
        limits: RunLimits,
        trace: F,
    ) -> Result<RunResult, TracedRunError<E>>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), E>,
    {
        crate::runtime::RunningExecution::new(self, input, limits)
            .map_err(TracedRunError::Run)?
            .run_with_borrowed_trace(trace)
    }
}
