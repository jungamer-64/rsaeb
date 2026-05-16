use core::convert::Infallible;

use crate::error::{
    FallibleTraceSnapshotRunError, RunError, TraceSnapshotError, TraceSnapshotRunError,
    TracedRunError,
};
use crate::execution::RunningExecution;
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
    /// Runs this program and emits infallible owned trace snapshot events.
    ///
    /// This convenience API materializes bounded `Vec<u8>` snapshots for the
    /// initial state and every committed step. Use
    /// [`Program::run_with_borrowed_trace`] when the trace sink only needs to
    /// inspect each event during the callback.
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

    /// Runs this program and emits fallible owned trace snapshot events.
    ///
    /// This is the snapshot API for sinks that can fail, such as serializers or
    /// host-side buffers with their own limits. Snapshot materialization errors
    /// and sink errors are reported as separate variants.
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

    /// Runs this program and emits infallible borrowed trace events.
    ///
    /// Borrowed trace events do not materialize owned snapshots. They are valid
    /// only for the callback invocation, so a sink that wants to retain bytes
    /// must copy them explicitly.
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

    /// Runs this program and emits fallible borrowed trace events.
    ///
    /// The callback borrows event bytes only for the duration of each call. A
    /// sink that wants to keep bytes must copy them explicitly or use
    /// [`Program::try_run_with_trace_snapshots`].
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
        RunningExecution::new(self, input, limits)
            .map_err(TracedRunError::Run)?
            .run_with_borrowed_trace(trace)
    }
}
