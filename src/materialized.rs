use alloc::vec::Vec;
use core::marker::PhantomData;

use crate::allocation::{AllocationContext, AllocationError};
use crate::inspect::PayloadView;
use crate::program::ReturnOutputView;
use crate::program::limits::{ReturnOutputBytePermit, TraceSnapshotBytePermit};
use crate::rule::{self, Rule};
use crate::trace::RuntimeStateView;

/// Marker for bytes materialized from runtime state.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RuntimeStateSnapshotDomain {}

/// Marker for bytes materialized from `(return)` output.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReturnOutputDomain {}

/// Marker for bytes materialized from parsed payload inspection.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PayloadInspectionDomain {}

/// Marker for bytes materialized as canonical rule source.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CanonicalRuleSourceDomain {}

/// Owned bytes tagged with the domain that produced them.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MaterializedBytes<Domain> {
    /// Materialized byte payload.
    bytes: Vec<u8>,
    /// Compile-time tag preventing byte-domain mixups.
    domain: PhantomData<fn() -> Domain>,
}

impl<Domain> MaterializedBytes<Domain> {
    /// Tags bytes after a domain-specific constructor has fixed their source.
    fn from_owned_bytes(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            domain: PhantomData,
        }
    }

    /// Borrows the materialized bytes without erasing the domain tag.
    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    /// Releases the byte payload at the public boundary.
    pub(crate) fn into_raw_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Returns the runtime state length in bytes.
    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Returns whether the byte sequence is empty.
    pub(crate) const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl MaterializedBytes<PayloadInspectionDomain> {
    /// Materializes bytes from a parsed payload inspection view.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the payload view cannot be materialized.
    pub(crate) fn from_payload_view(payload: PayloadView<'_>) -> Result<Self, AllocationError> {
        Ok(Self::from_owned_bytes(
            payload.to_vec_with_context(AllocationContext::PayloadView)?,
        ))
    }
}

impl MaterializedBytes<CanonicalRuleSourceDomain> {
    /// Materializes canonical source from one parsed rule.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if canonical source generation cannot
    /// allocate or its length cannot be represented.
    pub(crate) fn from_rule(rule: &Rule) -> Result<Self, AllocationError> {
        Ok(Self::from_owned_bytes(rule::canonical_source(rule)?))
    }
}

impl MaterializedBytes<RuntimeStateSnapshotDomain> {
    /// Materializes an explicitly requested runtime-state view.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the state view cannot be materialized.
    pub(crate) fn from_runtime_state_view(
        state: RuntimeStateView<'_>,
    ) -> Result<Self, AllocationError> {
        Ok(Self::from_owned_bytes(state.to_vec_with_context(
            AllocationContext::RuntimeStateView,
        )?))
    }

    /// Materializes the terminal stable runtime state.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the final state cannot be materialized.
    pub(crate) fn from_final_state_view(
        state: RuntimeStateView<'_>,
    ) -> Result<Self, AllocationError> {
        Ok(Self::from_owned_bytes(
            state.to_vec_with_context(AllocationContext::FinalOutput)?,
        ))
    }

    /// Materializes a runtime-state view for retained trace snapshots.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the trace snapshot cannot be materialized.
    pub(crate) fn from_trace_state_view(
        state: RuntimeStateView<'_>,
        permit: TraceSnapshotBytePermit,
    ) -> Result<Self, AllocationError> {
        Ok(Self::from_owned_bytes(state.to_vec_with_trace_permit(
            AllocationContext::TraceSnapshot,
            permit,
        )?))
    }
}

impl MaterializedBytes<ReturnOutputDomain> {
    /// Materializes a committed return-output view.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the return output cannot be materialized.
    pub(crate) fn from_return_output_view(
        output: ReturnOutputView<'_>,
    ) -> Result<Self, AllocationError> {
        Ok(Self::from_owned_bytes(
            output.to_vec_with_context(AllocationContext::ReturnOutput)?,
        ))
    }

    /// Materializes a committed return-output view after runtime limit admission.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the return output cannot be materialized.
    pub(crate) fn from_permitted_return_output_view(
        output: ReturnOutputView<'_>,
        permit: ReturnOutputBytePermit,
    ) -> Result<Self, AllocationError> {
        Ok(Self::from_owned_bytes(output.to_vec_with_return_permit(
            AllocationContext::ReturnOutput,
            permit,
        )?))
    }

    /// Materializes a return-output view for retained trace snapshots.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the trace snapshot cannot be materialized.
    pub(crate) fn from_trace_return_output_view(
        output: ReturnOutputView<'_>,
        permit: TraceSnapshotBytePermit,
    ) -> Result<Self, AllocationError> {
        Ok(Self::from_owned_bytes(output.to_vec_with_trace_permit(
            AllocationContext::TraceSnapshot,
            permit,
        )?))
    }
}
