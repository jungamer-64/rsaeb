use alloc::vec::Vec;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::bytes::{Payload, RuntimeByte, RuntimeStateByteCount};

/// Internal rewrite scratch.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RewriteScratch {
    /// Stored bytes.
    bytes: Vec<RuntimeByte>,
}

/// Internal prepared rewrite.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PreparedRewrite {
    /// Stored bytes.
    bytes: Vec<RuntimeByte>,
}

impl PreparedRewrite {
    /// Runs the into runtime bytes operation.
    pub(crate) fn into_runtime_bytes(self) -> Vec<RuntimeByte> {
        self.bytes
    }
}

impl RewriteScratch {
    /// Constructs the value from validated parts.
    pub(crate) fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    /// Runs the take prepared operation.
    pub(crate) fn take_prepared(&mut self) -> PreparedRewrite {
        PreparedRewrite {
            bytes: core::mem::take(&mut self.bytes),
        }
    }

    /// Runs the store previous state operation.
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
            RequestedCapacity::new(capacity.get()),
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
