use alloc::vec::Vec;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_reserve_total_exact,
};
use crate::bytes::Payload;
use crate::bytes::{ReturnOutputByteCount, RuntimeStateByteCount};
use crate::materialized::{MaterializedBytes, ReturnOutputDomain, RuntimeStateSnapshotDomain};

use super::limits::{ReturnOutputBytePermit, StepCount, TraceSnapshotBytePermit};

/// Structured result category for one completed run.
///
/// Stable completion and `(return)` completion are distinct outcomes rather
/// than a byte buffer plus a boolean flag. Stable bytes are the final runtime
/// state after rule search finds no match; return bytes are the payload of the
/// committed `(return)` rule.
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
/// bytes have been materialized successfully, and it is governed by runtime
/// state limits rather than return-output limits.
#[derive(Debug, PartialEq, Eq)]
pub struct RuntimeStateSnapshot {
    /// Owned bytes tagged as a stable runtime-state snapshot.
    bytes: MaterializedBytes<RuntimeStateSnapshotDomain>,
}

impl RuntimeStateSnapshot {
    /// Materializes an explicitly requested runtime-state view.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` when the runtime-state view exceeds the
    /// allocation limit for explicit state materialization.
    pub(crate) fn from_runtime_state_view(
        state: crate::trace::RuntimeStateView<'_>,
    ) -> Result<Self, AllocationError> {
        Ok(Self {
            bytes: MaterializedBytes::from_runtime_state_view(state)?,
        })
    }

    /// Materializes a terminal stable runtime state.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` when the final runtime state exceeds the
    /// allocation limit for stable run output.
    pub(crate) fn from_final_state_view(
        state: crate::trace::RuntimeStateView<'_>,
    ) -> Result<Self, AllocationError> {
        Ok(Self {
            bytes: MaterializedBytes::from_final_state_view(state)?,
        })
    }

    /// Materializes a runtime-state view for trace snapshots.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` when the traced runtime state exceeds the
    /// allocation limit for trace snapshot materialization.
    pub(crate) fn from_trace_state_view(
        state: crate::trace::RuntimeStateView<'_>,
        permit: TraceSnapshotBytePermit,
    ) -> Result<Self, AllocationError> {
        Ok(Self {
            bytes: MaterializedBytes::from_trace_state_view(state, permit)?,
        })
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
/// This value owns public raw bytes from the return payload. It is not a
/// runtime state snapshot; it comes from the parsed right-side return payload
/// after the return rule commits.
#[derive(Debug, PartialEq, Eq)]
pub struct ReturnOutput {
    /// Owned bytes tagged as `(return)` output.
    bytes: MaterializedBytes<ReturnOutputDomain>,
}

/// Borrowed `(return)` output payload produced by runtime execution.
///
/// This view is distinct from parsed payload inspection. It exists only on
/// runtime return paths and materializes into [`ReturnOutput`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ReturnOutputView<'program> {
    /// Return payload borrowed from the committed parsed rule.
    payload: &'program Payload,
}

impl ReturnOutput {
    /// Materializes committed `(return)` output bytes.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` when the committed return payload exceeds the
    /// allocation limit for return output.
    pub(crate) fn from_return_output_view(
        output: ReturnOutputView<'_>,
    ) -> Result<Self, AllocationError> {
        Ok(Self {
            bytes: MaterializedBytes::from_return_output_view(output)?,
        })
    }

    /// Materializes committed `(return)` output bytes after a runtime limit permit.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` when the committed return payload cannot be allocated.
    pub(crate) fn from_permitted_return_output_view(
        output: ReturnOutputView<'_>,
        permit: ReturnOutputBytePermit,
    ) -> Result<Self, AllocationError> {
        Ok(Self {
            bytes: MaterializedBytes::from_permitted_return_output_view(output, permit)?,
        })
    }

    /// Materializes committed `(return)` output bytes for a trace snapshot.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` when the traced return payload exceeds the
    /// allocation limit for trace snapshot materialization.
    pub(crate) fn from_trace_return_output_view(
        output: ReturnOutputView<'_>,
        permit: TraceSnapshotBytePermit,
    ) -> Result<Self, AllocationError> {
        Ok(Self {
            bytes: MaterializedBytes::from_trace_return_output_view(output, permit)?,
        })
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
    /// Borrows a parsed payload specifically as runtime return output.
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

    /// Materializes this return-output view after its runtime output limit was admitted.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the output buffer cannot be allocated.
    pub(crate) fn to_vec_with_return_permit(
        self,
        context: AllocationContext,
        permit: ReturnOutputBytePermit,
    ) -> Result<Vec<u8>, AllocationError> {
        self.to_vec_with_capacity(context, RequestedCapacity::new(permit.byte_count().get()))
    }

    /// Materializes this return-output view after its trace snapshot limit was admitted.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the output buffer cannot be allocated.
    pub(crate) fn to_vec_with_trace_permit(
        self,
        context: AllocationContext,
        permit: TraceSnapshotBytePermit,
    ) -> Result<Vec<u8>, AllocationError> {
        self.to_vec_with_capacity(context, RequestedCapacity::new(permit.byte_count().get()))
    }

    /// Materializes this return-output view with an already selected capacity.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the output buffer cannot be allocated.
    fn to_vec_with_capacity(
        self,
        context: AllocationContext,
        capacity: RequestedCapacity,
    ) -> Result<Vec<u8>, AllocationError> {
        let mut output = Vec::new();
        try_reserve_total_exact(&mut output, capacity, context)?;
        self.payload.push_bytes_to(&mut output, context)?;
        Ok(output)
    }

    /// Materializes this borrowed return output.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if return-output allocation fails.
    pub fn materialize(self) -> Result<ReturnOutput, AllocationError> {
        ReturnOutput::from_return_output_view(self)
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
/// The result records the number of committed execution steps and the terminal
/// outcome reached by the run. A failed run never produces this type; failures
/// remain in [`crate::error::RunError`] or traced-run error domains.
#[derive(Debug, PartialEq, Eq)]
pub struct RunResult {
    /// Number of committed execution steps in this run.
    steps: StepCount,
    /// Terminal execution outcome.
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

    /// Builds a result for a run ended by `(return)`.
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

    /// Number of committed execution steps.
    #[must_use]
    pub const fn steps(&self) -> StepCount {
        self.steps
    }
}
