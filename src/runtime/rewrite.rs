use alloc::vec::Vec;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::bytes::{Payload, RuntimeByte, RuntimeStateByteCount};

/// Reusable storage for building the next runtime state.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RewriteScratch {
    /// Scratch bytes for a candidate rewrite.
    bytes: Vec<RuntimeByte>,
}

/// Candidate rewrite bytes after allocation and limit checks have succeeded.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PreparedRewrite {
    /// Runtime bytes ready to replace the current state.
    bytes: Vec<RuntimeByte>,
}

impl PreparedRewrite {
    /// Moves prepared bytes into the committed runtime state.
    pub(crate) fn into_runtime_bytes(self) -> Vec<RuntimeByte> {
        self.bytes
    }
}

impl RewriteScratch {
    /// Starts with no retained rewrite buffer.
    pub(crate) fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    /// Moves the completed candidate rewrite out of scratch storage.
    pub(crate) fn take_prepared(&mut self) -> PreparedRewrite {
        PreparedRewrite {
            bytes: core::mem::take(&mut self.bytes),
        }
    }

    /// Reuses the previous state allocation as future scratch storage.
    pub(crate) fn store_previous_state(&mut self, bytes: Vec<RuntimeByte>) {
        self.bytes = bytes;
    }

    /// Clears scratch storage and reserves the requested rewrite capacity.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the rewrite scratch buffer cannot reserve
    /// the requested capacity.
    pub(crate) fn clear_and_reserve(
        &mut self,
        capacity: RuntimeStateByteCount,
    ) -> Result<(), AllocationError> {
        self.bytes.clear();
        try_reserve_total_exact(
            &mut self.bytes,
            RequestedCapacity::from_runtime_state_count(capacity),
            AllocationContext::RuntimeRewriteState,
        )
    }

    /// Appends existing runtime bytes into scratch storage.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the rewrite scratch buffer cannot grow.
    pub(crate) fn push_existing(
        &mut self,
        source: impl IntoIterator<Item = RuntimeByte>,
    ) -> Result<(), AllocationError> {
        for byte in source {
            try_push(
                &mut self.bytes,
                byte,
                AllocationContext::RuntimeRewriteState,
            )?;
        }

        Ok(())
    }

    /// Appends program payload bytes into scratch storage.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the rewrite scratch buffer cannot grow.
    pub(crate) fn push_payload(&mut self, payload: &Payload) -> Result<(), AllocationError> {
        self.push_existing(payload.runtime_bytes())
    }
}
