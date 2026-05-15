use alloc::vec::Vec;

use crate::allocation::{AllocationContext, try_push, try_reserve_total_exact};
use crate::bytes::{RuntimeByte, RuntimeStateByteCount};
use crate::error::{InputError, LimitError, RunError, StateLimitContext};
use crate::program::RunLimits;

/// Borrowed runtime input after ASCII validation.
///
/// Runtime input is a separate byte domain from program source. It may contain
/// ASCII whitespace, control bytes, and reserved syntax bytes, but it cannot
/// contain non-ASCII bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeInput<'input> {
    bytes: &'input [u8],
}

impl<'input> RuntimeInput<'input> {
    /// Validates raw bytes as runtime input.
    ///
    /// # Errors
    ///
    /// Returns `InputError` if any input byte is non-ASCII or if its one-based
    /// column cannot be represented.
    pub fn validate(input: &'input [u8]) -> Result<Self, InputError> {
        for (zero_based_column, byte) in input.iter().copied().enumerate() {
            RuntimeByte::validate_input(byte, zero_based_column)?;
        }

        Ok(Self { bytes: input })
    }

    /// Borrow the validated input bytes.
    #[must_use]
    pub const fn as_bytes(self) -> &'input [u8] {
        self.bytes
    }

    /// Runtime input length in bytes.
    #[must_use]
    pub const fn byte_count(self) -> RuntimeStateByteCount {
        RuntimeStateByteCount::new(self.bytes.len())
    }

    /// Whether this runtime input contains no bytes.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.bytes.is_empty()
    }

    pub(super) fn runtime_bytes(self) -> impl Iterator<Item = RuntimeByte> + 'input {
        self.bytes
            .iter()
            .copied()
            .map(RuntimeByte::from_validated_input)
    }
}

/// Runtime input materialized into the mutable execution byte domain.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct InitialStateBytes {
    pub(super) bytes: Vec<RuntimeByte>,
}

impl InitialStateBytes {
    pub(super) fn materialize(
        input: RuntimeInput<'_>,
        limits: RunLimits,
    ) -> Result<Self, RunError> {
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
            input.as_bytes().len(),
            AllocationContext::RuntimeInput,
        )?;

        for byte in input.runtime_bytes() {
            try_push(&mut bytes, byte, AllocationContext::RuntimeInput)?;
        }

        Ok(Self { bytes })
    }
}
