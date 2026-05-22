use alloc::vec::Vec;
use core::fmt;

use crate::allocation::{AllocationContext, try_push, try_reserve_total_exact};
use crate::bytes::{RuntimeByte, RuntimeInputByteCount, RuntimeStateByteCount};
use crate::error::{RunError, RuntimeInputError};
use crate::program::RuntimeInputByteLimit;

use super::budget::RuntimeBudgetState;

/// Borrowed runtime input source at the validation boundary.
///
/// Constructing this value labels host bytes as runtime input bytes. It does
/// not validate ASCII or classify bytes into the runtime domain; that ownership
/// belongs to [`RuntimeInput::validate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeInputSource<'input> {
    bytes: &'input [u8],
}

impl<'input> RuntimeInputSource<'input> {
    /// Labels raw host bytes as runtime input.
    #[must_use]
    pub const fn from_bytes(bytes: &'input [u8]) -> Self {
        Self { bytes }
    }

    /// Borrows the original host bytes.
    #[must_use]
    pub const fn as_bytes(self) -> &'input [u8] {
        self.bytes
    }

    /// Returns whether the source contains no bytes.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.bytes.is_empty()
    }
}

/// Runtime input after ASCII validation and byte-domain classification.
///
/// Runtime input is a separate byte domain from program source. It may contain
/// ASCII whitespace, control bytes, and reserved syntax bytes, but it cannot
/// contain non-ASCII bytes. Validation also owns the input bytes so a
/// [`RuntimeInput`] can be reused across runs without revalidating raw host
/// input.
#[derive(PartialEq, Eq)]
pub struct RuntimeInput {
    bytes: Vec<RuntimeByte>,
}

impl fmt::Debug for RuntimeInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeInput")
            .field("bytes", &RuntimeInputBytesDebug(self))
            .finish()
    }
}

struct RuntimeInputBytesDebug<'input>(&'input RuntimeInput);

impl fmt::Debug for RuntimeInputBytesDebug<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_list()
            .entries(self.0.materialized_bytes())
            .finish()
    }
}

impl RuntimeInput {
    /// Validates a runtime input source as owned runtime input.
    ///
    /// Runtime input accepts all ASCII bytes, including bytes that would be
    /// reserved syntax in program source. Non-ASCII bytes are rejected with a
    /// structured input column before execution starts.
    ///
    /// # Errors
    ///
    /// Returns `RuntimeInputError` if the input exceeds `limits`, if any input
    /// byte is non-ASCII, if its one-based column cannot be represented, or if
    /// owned storage cannot be allocated.
    pub fn validate(
        input: RuntimeInputSource<'_>,
        limit: RuntimeInputByteLimit,
    ) -> Result<Self, RuntimeInputError> {
        let input = input.as_bytes();
        let byte_count = RuntimeInputByteCount::new(input.len());
        if byte_count.get() > limit.get() {
            return Err(RuntimeInputError::limit(limit, byte_count));
        }

        let mut bytes = Vec::new();
        try_reserve_total_exact(
            &mut bytes,
            input.len(),
            AllocationContext::RuntimeInputValidation,
        )?;

        for (zero_based_column, byte) in input.iter().copied().enumerate() {
            try_push(
                &mut bytes,
                RuntimeByte::validate_input(byte, zero_based_column)?,
                AllocationContext::RuntimeInputValidation,
            )?;
        }

        Ok(Self { bytes })
    }

    pub(crate) fn materialized_bytes(&self) -> impl Iterator<Item = u8> + '_ {
        self.bytes.iter().copied().map(RuntimeByte::materialize)
    }

    /// Runtime input length in bytes.
    #[must_use]
    pub fn byte_count(&self) -> RuntimeInputByteCount {
        RuntimeInputByteCount::new(self.bytes.len())
    }

    /// Returns whether this runtime input contains no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub(crate) fn runtime_bytes(&self) -> impl Iterator<Item = RuntimeByte> + '_ {
        self.bytes.iter().copied()
    }
}

/// Runtime input materialized into the mutable execution byte domain.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct InitialStateBytes {
    bytes: Vec<RuntimeByte>,
}

impl InitialStateBytes {
    /// Materializes validated runtime input into mutable execution state bytes.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if the input exceeds runtime state limits or the
    /// initial state buffer cannot be allocated.
    pub(crate) fn materialize(
        input: &RuntimeInput,
        budget: RuntimeBudgetState,
    ) -> Result<Self, RunError> {
        let byte_count = input.byte_count();
        let state_len = RuntimeStateByteCount::from_runtime_input_count(byte_count);

        budget.ensure_initial_state_len(state_len)?;

        let mut bytes = Vec::new();
        try_reserve_total_exact(
            &mut bytes,
            byte_count.get(),
            AllocationContext::InitialRuntimeState,
        )?;

        for byte in input.runtime_bytes() {
            try_push(&mut bytes, byte, AllocationContext::InitialRuntimeState)?;
        }

        Ok(Self { bytes })
    }

    pub(crate) fn into_runtime_bytes(self) -> Vec<RuntimeByte> {
        self.bytes
    }
}
