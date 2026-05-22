use alloc::vec::Vec;

use crate::bytes::{ReturnOutputByteCount, RuntimeStateByteCount};

use super::limits::StepCount;

/// Structured result category for one completed run.
///
/// Stable completion and `(return)` completion are distinct outcomes rather
/// than a byte buffer plus a boolean flag.
#[derive(Debug, PartialEq, Eq)]
pub enum RunOutcome {
    /// No rule matched the final runtime state.
    Stable(RuntimeStateSnapshot),
    /// A matched rule executed the `(return)` action.
    Return(ReturnOutput),
}

/// Materialized final runtime state for a run that ended without `(return)`.
///
/// This value owns public raw bytes. It is produced only after runtime-state
/// bytes have been materialized successfully.
#[derive(Debug, PartialEq, Eq)]
pub struct RuntimeStateSnapshot {
    bytes: Vec<u8>,
}

impl RuntimeStateSnapshot {
    pub(crate) fn from_execution_state(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    pub(crate) fn from_runtime_state_view(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    pub(crate) fn from_trace_snapshot(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// Borrow the materialized runtime-state bytes.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    /// Consumes the snapshot and returns the materialized host bytes.
    #[must_use]
    pub fn into_raw_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Materialized byte length.
    #[must_use]
    pub fn byte_count(&self) -> RuntimeStateByteCount {
        RuntimeStateByteCount::new(self.bytes.len())
    }

    /// Whether this snapshot contains no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// Materialized final output from a matched `(return)` rule.
///
/// This value owns public raw bytes from the return payload.
#[derive(Debug, PartialEq, Eq)]
pub struct ReturnOutput {
    bytes: Vec<u8>,
}

impl ReturnOutput {
    pub(crate) fn from_return_payload(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    pub(crate) fn from_trace_snapshot(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// Borrow the materialized `(return)` output bytes.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    /// Consumes the return output and returns the materialized host bytes.
    #[must_use]
    pub fn into_raw_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Materialized byte length.
    #[must_use]
    pub fn byte_count(&self) -> ReturnOutputByteCount {
        ReturnOutputByteCount::new(self.bytes.len())
    }

    /// Whether this return output contains no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// Result of one program execution.
///
/// The result records the number of committed rewrite steps and the terminal
/// outcome reached by the run.
#[derive(Debug, PartialEq, Eq)]
pub struct RunResult {
    steps: StepCount,
    outcome: RunOutcome,
}

impl RunResult {
    pub(crate) fn stable(output: RuntimeStateSnapshot, steps: StepCount) -> Self {
        Self {
            steps,
            outcome: RunOutcome::Stable(output),
        }
    }

    pub(crate) fn from_return(output: ReturnOutput, steps: StepCount) -> Self {
        Self {
            steps,
            outcome: RunOutcome::Return(output),
        }
    }

    /// Structured execution outcome.
    #[must_use]
    pub const fn outcome(&self) -> &RunOutcome {
        &self.outcome
    }

    /// Consumes the result and returns the structured execution outcome.
    #[must_use]
    pub fn into_outcome(self) -> RunOutcome {
        self.outcome
    }

    /// Number of rewrite steps applied.
    #[must_use]
    pub const fn steps(&self) -> StepCount {
        self.steps
    }
}
