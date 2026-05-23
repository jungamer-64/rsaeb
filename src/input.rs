//! Runtime-input boundary types.
//!
//! Host bytes enter the interpreter as [`RuntimeInputSource`], then
//! [`RuntimeInput::validate`] checks the runtime input contract before storing
//! owned runtime-domain bytes. Execution consumes a [`RunSeed`] admitted from
//! validated input and execution limits, so input validation and execution
//! budgets cannot be conflated.

use alloc::vec::Vec;
use core::fmt;

use crate::allocation::{AllocationContext, RequestedCapacity, try_push, try_reserve_total_exact};
use crate::bytes::{RuntimeByte, RuntimeInputByte, RuntimeInputByteCount, RuntimeStateByteCount};
use crate::error::{RunAdmissionError, RuntimeInputError};
use crate::limits::{ExecutionLimits, RuntimeInputByteLimit, RuntimeInputLimits};

/// Borrowed runtime input source at the validation boundary.
///
/// Constructing this value labels host bytes as runtime input bytes. It does
/// not validate ASCII or classify bytes into the runtime domain; that ownership
/// belongs to [`RuntimeInput::validate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeInputSource<'input> {
    /// Stored bytes.
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

/// Internal validated runtime input source.
struct ValidatedRuntimeInputSource<'input> {
    /// Stored source.
    source: RuntimeInputSource<'input>,
    /// Stored byte count.
    byte_count: RuntimeInputByteCount,
}

impl<'input> ValidatedRuntimeInputSource<'input> {
    /// Validates raw host bytes before owned runtime-input allocation starts.
    ///
    /// # Errors
    ///
    /// Returns `RuntimeInputError` if the input exceeds `limit`, if any input
    /// byte is non-ASCII, or if its one-based column cannot be represented.
    fn new(
        source: RuntimeInputSource<'input>,
        limit: RuntimeInputByteLimit,
    ) -> Result<Self, RuntimeInputError> {
        let byte_count = RuntimeInputByteCount::new(source.as_bytes().len());
        if byte_count.get() > limit.get() {
            return Err(RuntimeInputError::input_limit(limit, byte_count));
        }

        for (zero_based_column, byte) in source.as_bytes().iter().copied().enumerate() {
            RuntimeInputByte::validate(byte, zero_based_column)?;
        }

        Ok(Self { source, byte_count })
    }

    /// Returns the stored bytes.
    const fn bytes(&self) -> &'input [u8] {
        self.source.as_bytes()
    }

    /// Returns the typed byte count.
    const fn byte_count(&self) -> RuntimeInputByteCount {
        self.byte_count
    }

    /// Runs the runtime input bytes operation.
    fn runtime_input_bytes(&self) -> impl Iterator<Item = RuntimeInputByte> + 'input {
        self.bytes()
            .iter()
            .copied()
            .map(RuntimeInputByte::from_validated_ascii)
    }
}

/// Runtime input admitted after validation.
///
/// Runtime input is a separate byte domain from program source. It may contain
/// ASCII whitespace, control bytes, and reserved syntax bytes, but it cannot
/// contain non-ASCII bytes. This value owns validated bytes only; execution
/// budgets are admitted later by [`RunSeed`].
#[derive(PartialEq, Eq)]
pub struct RuntimeInput {
    /// Stored bytes.
    bytes: Vec<RuntimeByte>,
}

impl fmt::Debug for RuntimeInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeInput")
            .field("bytes", &RuntimeInputDebugBytes(self))
            .finish()
    }
}

/// Internal runtime input debug bytes.
struct RuntimeInputDebugBytes<'input>(&'input RuntimeInput);

impl fmt::Debug for RuntimeInputDebugBytes<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_list()
            .entries(self.0.materialized_bytes())
            .finish()
    }
}

impl RuntimeInput {
    /// Validates a runtime input source for one run.
    ///
    /// Runtime input accepts all ASCII bytes, including bytes that would be
    /// reserved syntax in program source. Non-ASCII bytes are rejected with a
    /// structured input column before execution starts.
    ///
    /// # Errors
    ///
    /// Returns `RuntimeInputError` if the input exceeds `limits`, if any input byte
    /// is non-ASCII, if its one-based column cannot be represented, or if owned
    /// storage cannot be allocated.
    pub fn validate(
        input: RuntimeInputSource<'_>,
        limits: RuntimeInputLimits,
    ) -> Result<Self, RuntimeInputError> {
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

        Ok(Self { bytes })
    }

    /// Returns materialized runtime bytes.
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

    /// Runs the into runtime bytes operation.
    fn into_runtime_bytes(self) -> Vec<RuntimeByte> {
        self.bytes
    }
}

/// Run-start witness admitted from validated input and execution limits.
#[derive(Debug, PartialEq, Eq)]
pub struct RunSeed {
    /// Stored input.
    input: RuntimeInput,
    /// Stored limits.
    limits: ExecutionLimits,
}

impl RunSeed {
    /// Admits validated runtime input for execution under execution limits.
    ///
    /// # Errors
    ///
    /// Returns `RunAdmissionError` if the validated input would exceed the
    /// initial runtime-state budget.
    pub fn admit(input: RuntimeInput, limits: ExecutionLimits) -> Result<Self, RunAdmissionError> {
        let initial_state_len = RuntimeStateByteCount::from_runtime_input_count(input.byte_count());
        if initial_state_len.get() > limits.state_byte_limit().get() {
            return Err(RunAdmissionError::initial_state_limit(
                limits.state_byte_limit(),
                initial_state_len,
            ));
        }

        Ok(Self { input, limits })
    }

    /// Runs the into runtime parts operation.
    pub(crate) fn into_runtime_parts(self) -> (InitialStateBytes, ExecutionLimits) {
        (
            InitialStateBytes {
                bytes: self.input.into_runtime_bytes(),
            },
            self.limits,
        )
    }
}

/// Runtime input materialized into the mutable execution byte domain.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct InitialStateBytes {
    /// Stored bytes.
    bytes: Vec<RuntimeByte>,
}

impl InitialStateBytes {
    /// Runs the into runtime bytes operation.
    pub(crate) fn into_runtime_bytes(self) -> Vec<RuntimeByte> {
        self.bytes
    }
}
