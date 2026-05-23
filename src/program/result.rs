use alloc::vec::Vec;

use crate::allocation::{AllocationContext, AllocationError};
use crate::bytes::Payload;
use crate::bytes::{ReturnOutputByteCount, RuntimeStateByteCount};
use crate::materialized::{MaterializedBytes, ReturnOutputDomain, RuntimeStateSnapshotDomain};

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
    /// Stored bytes.
    bytes: MaterializedBytes<RuntimeStateSnapshotDomain>,
}

impl RuntimeStateSnapshot {
    /// Builds the value from execution state input.
    pub(crate) fn from_execution_state(bytes: Vec<u8>) -> Self {
        Self {
            bytes: MaterializedBytes::from_vec(bytes),
        }
    }

    /// Builds the value from runtime state view input.
    pub(crate) fn from_runtime_state_view(bytes: Vec<u8>) -> Self {
        Self {
            bytes: MaterializedBytes::from_vec(bytes),
        }
    }

    /// Builds the value from trace snapshot input.
    pub(crate) fn from_trace_snapshot(bytes: Vec<u8>) -> Self {
        Self {
            bytes: MaterializedBytes::from_vec(bytes),
        }
    }

    /// Borrow the materialized runtime-state bytes.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    /// Consumes the snapshot and returns the materialized host bytes.
    #[must_use]
    pub fn into_raw_bytes(self) -> Vec<u8> {
        self.bytes.into_raw_bytes()
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
    /// Stored bytes.
    bytes: MaterializedBytes<ReturnOutputDomain>,
}

/// Borrowed `(return)` output payload produced by runtime execution.
///
/// This view is distinct from parsed payload inspection. It exists only on
/// runtime return paths and materializes into [`ReturnOutput`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ReturnOutputView<'program> {
    /// Stored payload.
    payload: &'program Payload,
}

impl ReturnOutput {
    /// Builds the value from return payload input.
    pub(crate) fn from_return_payload(bytes: Vec<u8>) -> Self {
        Self {
            bytes: MaterializedBytes::from_vec(bytes),
        }
    }

    /// Builds the value from trace snapshot input.
    pub(crate) fn from_trace_snapshot(bytes: Vec<u8>) -> Self {
        Self {
            bytes: MaterializedBytes::from_vec(bytes),
        }
    }

    /// Borrow the materialized `(return)` output bytes.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    /// Consumes the return output and returns the materialized host bytes.
    #[must_use]
    pub fn into_raw_bytes(self) -> Vec<u8> {
        self.bytes.into_raw_bytes()
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

impl<'program> ReturnOutputView<'program> {
    /// Constructs the value from validated parts.
    pub(crate) const fn new(payload: &'program Payload) -> Self {
        Self { payload }
    }

    /// Return output length in bytes.
    #[must_use]
    pub fn byte_count(self) -> ReturnOutputByteCount {
        ReturnOutputByteCount::from_payload_count(self.payload.byte_count())
    }

    /// Returns whether this borrowed return output contains no bytes.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.byte_count().is_zero()
    }

    /// Materializes this return output view at the requested allocation site.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the output buffer cannot be allocated.
    pub(crate) fn to_vec_with_context(
        self,
        context: AllocationContext,
    ) -> Result<Vec<u8>, AllocationError> {
        self.payload.to_vec_with_context(context)
    }

    /// Materializes this borrowed return output.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if return-output allocation fails.
    pub fn materialize(self) -> Result<ReturnOutput, AllocationError> {
        Ok(ReturnOutput::from_return_payload(
            self.to_vec_with_context(AllocationContext::ReturnOutput)?,
        ))
    }
}

impl core::fmt::Debug for ReturnOutputView<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_list()
            .entries(self.payload.bytes())
            .finish()
    }
}

/// Result of one program execution.
///
/// The result records the number of committed rewrite steps and the terminal
/// outcome reached by the run.
#[derive(Debug, PartialEq, Eq)]
pub struct RunResult {
    /// Stored steps.
    steps: StepCount,
    /// Stored outcome.
    outcome: RunOutcome,
}

impl RunResult {
    /// Builds the stable value.
    pub(crate) fn stable(output: RuntimeStateSnapshot, steps: StepCount) -> Self {
        Self {
            steps,
            outcome: RunOutcome::Stable(output),
        }
    }

    /// Builds the value from return input.
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
