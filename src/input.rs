//! Runtime-input boundary types.
//!
//! Host bytes enter the interpreter as [`RuntimeInputSource`], then
//! [`RunInput::validate`] checks the run input contract before storing owned
//! runtime-domain bytes. Execution consumes [`RunInput`], so validated input
//! cannot be reused accidentally across runs or paired with different runtime
//! limits.

use alloc::vec::Vec;
use core::fmt;

use crate::allocation::{AllocationContext, RequestedCapacity, try_push, try_reserve_total_exact};
use crate::bytes::{RuntimeByte, RuntimeInputByte, RuntimeInputByteCount, RuntimeStateByteCount};
use crate::error::{RunError, RunInputError};
use crate::limits::{RunLimits, RuntimeInputByteLimit};
use crate::runtime::budget::RuntimeBudgetState;

/// Borrowed runtime input source at the validation boundary.
///
/// Constructing this value labels host bytes as runtime input bytes. It does
/// not validate ASCII or classify bytes into the runtime domain; that ownership
/// belongs to [`RunInput::validate`].
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

struct ValidatedRuntimeInputSource<'input> {
    source: RuntimeInputSource<'input>,
    byte_count: RuntimeInputByteCount,
}

impl<'input> ValidatedRuntimeInputSource<'input> {
    /// Validates raw host bytes before owned runtime-input allocation starts.
    ///
    /// # Errors
    ///
    /// Returns `RunInputError` if the input exceeds `limit`, if any input
    /// byte is non-ASCII, or if its one-based column cannot be represented.
    fn new(
        source: RuntimeInputSource<'input>,
        limit: RuntimeInputByteLimit,
    ) -> Result<Self, RunInputError> {
        let byte_count = RuntimeInputByteCount::new(source.as_bytes().len());
        if byte_count.get() > limit.get() {
            return Err(RunInputError::input_limit(limit, byte_count));
        }

        for (zero_based_column, byte) in source.as_bytes().iter().copied().enumerate() {
            RuntimeInputByte::validate(byte, zero_based_column)?;
        }

        Ok(Self { source, byte_count })
    }

    const fn bytes(&self) -> &'input [u8] {
        self.source.as_bytes()
    }

    const fn byte_count(&self) -> RuntimeInputByteCount {
        self.byte_count
    }

    fn runtime_input_bytes(&self) -> impl Iterator<Item = RuntimeInputByte> + 'input {
        self.bytes()
            .iter()
            .copied()
            .map(RuntimeInputByte::from_validated_ascii)
    }
}

/// Runtime input admitted for one run after validation and limit binding.
///
/// Runtime input is a separate byte domain from program source. It may contain
/// ASCII whitespace, control bytes, and reserved syntax bytes, but it cannot
/// contain non-ASCII bytes. Validation owns the input bytes until a run starts;
/// execution consumes this value and moves the classified bytes directly into
/// mutable runtime state.
#[derive(PartialEq, Eq)]
pub struct RunInput {
    bytes: Vec<RuntimeByte>,
    limits: RunLimits,
}

impl fmt::Debug for RunInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RunInput")
            .field("bytes", &RuntimeInputDebugBytes(self))
            .finish()
    }
}

struct RuntimeInputDebugBytes<'input>(&'input RunInput);

impl fmt::Debug for RuntimeInputDebugBytes<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_list()
            .entries(self.0.materialized_bytes())
            .finish()
    }
}

impl RunInput {
    /// Validates a runtime input source for one run.
    ///
    /// Runtime input accepts all ASCII bytes, including bytes that would be
    /// reserved syntax in program source. Non-ASCII bytes are rejected with a
    /// structured input column before execution starts.
    ///
    /// # Errors
    ///
    /// Returns `RunInputError` if the input exceeds `limits`, if any input byte
    /// is non-ASCII, if its one-based column cannot be represented, if the
    /// admitted input exceeds the initial runtime-state limit, or if owned
    /// storage cannot be allocated.
    pub fn validate(
        input: RuntimeInputSource<'_>,
        limits: RunLimits,
    ) -> Result<Self, RunInputError> {
        let input = ValidatedRuntimeInputSource::new(input, limits.input_byte_limit())?;

        // Allocation starts only after the complete boundary validation pass;
        // the iterator below consumes that witness instead of validating each
        // byte a second time.
        let mut bytes = Vec::new();
        try_reserve_total_exact(
            &mut bytes,
            RequestedCapacity::new(input.byte_count().get()),
            AllocationContext::RuntimeInputValidation,
        )?;

        for byte in input.runtime_input_bytes() {
            try_push(
                &mut bytes,
                byte.into_runtime_byte(),
                AllocationContext::RuntimeInputValidation,
            )?;
        }

        Ok(Self { bytes, limits })
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

    pub(crate) fn into_runtime_parts(self) -> (Vec<RuntimeByte>, RunLimits) {
        (self.bytes, self.limits)
    }
}

/// Runtime input materialized into the mutable execution byte domain.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct InitialStateBytes {
    bytes: Vec<RuntimeByte>,
}

impl InitialStateBytes {
    /// Moves validated runtime input into mutable execution state bytes.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if the input exceeds runtime state limits.
    pub(crate) fn from_runtime_input(
        input: RunInput,
    ) -> Result<Self, RunError> {
        let (bytes, limits) = input.into_runtime_parts();
        let budget = RuntimeBudgetState::new(limits);
        let byte_count = input.byte_count();
        let state_len = RuntimeStateByteCount::from_runtime_input_count(byte_count);

        budget.ensure_initial_state_len(state_len)?;
        Ok(Self { bytes })
    }

    pub(crate) fn into_runtime_bytes(self) -> Vec<RuntimeByte> {
        self.bytes
    }
}
