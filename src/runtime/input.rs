use alloc::vec::Vec;

use crate::allocation::{AllocationContext, AllocationError, try_push, try_reserve_total_exact};
use crate::bytes::{RuntimeByte, RuntimeStateByteCount};
use crate::error::{LimitError, RunError, RuntimeInputError, StateLimitContext};
use crate::program::RunLimits;

/// Runtime input after ASCII validation and byte-domain classification.
///
/// Runtime input is a separate byte domain from program source. It may contain
/// ASCII whitespace, control bytes, and reserved syntax bytes, but it cannot
/// contain non-ASCII bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeInput {
    bytes: Vec<RuntimeByte>,
}

impl RuntimeInput {
    /// Validates raw bytes as runtime input.
    ///
    /// # Errors
    ///
    /// Returns `RuntimeInputError` if any input byte is non-ASCII, if its
    /// one-based column cannot be represented, or if owned storage cannot be
    /// allocated.
    pub fn validate(input: &[u8]) -> Result<Self, RuntimeInputError> {
        let mut bytes = Vec::new();
        try_reserve_total_exact(&mut bytes, input.len(), AllocationContext::RuntimeInput)?;

        for (zero_based_column, byte) in input.iter().copied().enumerate() {
            try_push(
                &mut bytes,
                RuntimeByte::validate_input(byte, zero_based_column)?,
                AllocationContext::RuntimeInput,
            )?;
        }

        Ok(Self { bytes })
    }

    /// Runtime input bytes as a materializing iterator.
    pub fn bytes(&self) -> impl Iterator<Item = u8> + '_ {
        self.bytes.iter().copied().map(RuntimeByte::materialize)
    }

    /// Materializes this runtime input as raw bytes.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the output buffer cannot be allocated.
    pub fn to_vec(&self) -> Result<Vec<u8>, AllocationError> {
        let mut output = Vec::new();
        try_reserve_total_exact(
            &mut output,
            self.bytes.len(),
            AllocationContext::RuntimeInput,
        )?;
        for byte in self.bytes() {
            try_push(&mut output, byte, AllocationContext::RuntimeInput)?;
        }
        Ok(output)
    }

    /// Runtime input length in bytes.
    #[must_use]
    pub fn byte_count(&self) -> RuntimeStateByteCount {
        RuntimeStateByteCount::new(self.bytes.len())
    }

    /// Whether this runtime input contains no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub(super) fn runtime_bytes(&self) -> impl Iterator<Item = RuntimeByte> + '_ {
        self.bytes.iter().copied()
    }
}

/// Runtime input materialized into the mutable execution byte domain.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct InitialStateBytes {
    pub(super) bytes: Vec<RuntimeByte>,
}

impl InitialStateBytes {
    pub(super) fn materialize(input: &RuntimeInput, limits: RunLimits) -> Result<Self, RunError> {
        let byte_count = input.byte_count();

        if byte_count.get() > limits.state_byte_limit().get() {
            return Err(LimitError::state(
                StateLimitContext::Input,
                limits.state_byte_limit(),
                byte_count,
            )
            .into());
        }

        let mut bytes = Vec::new();
        try_reserve_total_exact(
            &mut bytes,
            byte_count.get(),
            AllocationContext::RuntimeInput,
        )?;

        for byte in input.runtime_bytes() {
            try_push(&mut bytes, byte, AllocationContext::RuntimeInput)?;
        }

        Ok(Self { bytes })
    }
}
