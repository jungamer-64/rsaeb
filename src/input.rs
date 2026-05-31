//! Runtime-input boundary types.
//!
//! Host bytes enter the interpreter as [`RuntimeInputSource`], then
//! [`RuntimeInput::validate`] checks the runtime input contract before storing
//! owned runtime-domain bytes. Execution consumes a [`RunSeed`] admitted from
//! validated input under an execution policy, so input validation and execution
//! budgets cannot be conflated.
//!
//! The three public values in this module represent three different states:
//!
//! - [`RuntimeInputSource`] is a borrowed label for raw host bytes. It has not
//!   proven ASCII validity and it owns nothing.
//! - [`RuntimeInput`] owns bytes after the runtime-input contract has been
//!   checked. It still has no step, state, or return-output budget.
//! - [`RunSeed`] consumes validated input under an execution policy and proves that
//!   the initial runtime state may be created for exactly one execution.
//!
//! Admission is deliberately separate from validation. Input construction can
//! fail because the raw bytes are not acceptable runtime input; admission can
//! fail because acceptable input is too large to become the initial state under
//! this run's execution policy.
//!
//! ```
//! use rsaeb::error::RunAdmissionError;
//! use rsaeb::input::{RunSeed, RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{StaticExecutionPolicy, StaticRuntimeInputPolicy};
//!
//! type Input8 = StaticRuntimeInputPolicy<8>;
//! type State3 = StaticExecutionPolicy<10, 3, 8>;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let input = RuntimeInput::<Input8>::validate(RuntimeInputSource::from_bytes(b"abcd"))?;
//!
//! let Err(error) = RunSeed::<State3>::admit(input) else {
//!     return Err("expected run admission to reject the initial state".into());
//! };
//!
//! if !matches!(
//!     error,
//!     RunAdmissionError::InitialStateTooLarge { attempted_len, .. }
//!         if attempted_len.get() == 4
//! ) {
//!     return Err("unexpected admission error".into());
//! }
//! # Ok(())
//! # }
//! ```

use alloc::vec::Vec;
use core::{fmt, marker::PhantomData};

use crate::allocation::{AllocationContext, RequestedCapacity, try_push, try_reserve_total_exact};
use crate::bytes::{RuntimeByte, RuntimeInputByte, RuntimeInputByteCount, RuntimeStateByteCount};
use crate::error::{RunAdmissionError, RuntimeInputError};
use crate::policy::{
    DefaultExecutionPolicy, DefaultRuntimeInputPolicy, ExecutionPolicy, RuntimeInputPolicy,
};
use crate::runtime::budget::RuntimeBudgetState;

/// Borrowed runtime input source at the validation boundary.
///
/// Constructing this value labels host bytes as runtime input bytes. It does
/// not validate ASCII or classify bytes into the runtime domain; that ownership
/// belongs to [`RuntimeInput::validate`]. The source may be copied freely
/// because it is only a borrowed boundary marker; copying it never duplicates
/// validated runtime state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeInputSource<'input> {
    /// Raw host bytes awaiting runtime-input validation.
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

/// Runtime input admitted after validation.
///
/// Runtime input is a separate byte domain from program source. It may contain
/// ASCII whitespace, control bytes, and reserved syntax bytes, but it cannot
/// contain non-ASCII bytes. This value owns validated bytes only; execution
/// budgets are admitted later by [`RunSeed`]. Reusing equivalent bytes for
/// another run means validating another [`RuntimeInputSource`], not cloning a
/// previously admitted execution state.
#[derive(PartialEq, Eq)]
pub struct RuntimeInput<I: RuntimeInputPolicy = DefaultRuntimeInputPolicy> {
    /// Owned bytes classified for mutable runtime state.
    bytes: Vec<RuntimeByte>,
    /// Compile-time runtime-input policy selected for this value.
    policy: PhantomData<I>,
}

impl<I: RuntimeInputPolicy> fmt::Debug for RuntimeInput<I> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeInput")
            .field("bytes", &RuntimeInputDebugBytes(self))
            .finish()
    }
}

/// Internal runtime input debug bytes.
struct RuntimeInputDebugBytes<'input, I: RuntimeInputPolicy>(&'input RuntimeInput<I>);

impl<I: RuntimeInputPolicy> fmt::Debug for RuntimeInputDebugBytes<'_, I> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_list()
            .entries(self.0.materialized_bytes())
            .finish()
    }
}

impl<I: RuntimeInputPolicy> RuntimeInput<I> {
    /// Validates a runtime input source for one run.
    ///
    /// Runtime input accepts all ASCII bytes, including bytes that would be
    /// reserved syntax in program source. Non-ASCII bytes are rejected with a
    /// structured input column before execution starts. Owned storage is
    /// reserved only after the full validation pass succeeds, so
    /// the selected [`RuntimeInputPolicy`] bounds raw input classification
    /// before allocation grows runtime-domain bytes.
    ///
    /// # Errors
    ///
    /// Returns `RuntimeInputError` if the input exceeds the selected policy, if
    /// any input byte is non-ASCII, if its one-based column cannot be
    /// represented, or if owned storage cannot be allocated.
    pub fn validate(input: RuntimeInputSource<'_>) -> Result<Self, RuntimeInputError> {
        let byte_count = RuntimeInputByteCount::new(input.as_bytes().len());
        let limit = I::INPUT_BYTE_LIMIT;
        if !limit.accepts(byte_count) {
            return Err(RuntimeInputError::input_limit(limit, byte_count));
        }

        let mut bytes = Vec::new();
        try_reserve_total_exact(
            &mut bytes,
            RequestedCapacity::from_runtime_input_count(byte_count),
            AllocationContext::RuntimeInputValidation,
        )?;

        for (zero_based_column, byte) in input.as_bytes().iter().copied().enumerate() {
            try_push(
                &mut bytes,
                RuntimeInputByte::validate(byte, zero_based_column)?.into_runtime_byte(),
                AllocationContext::RuntimeInputValidation,
            )?;
        }

        Ok(Self {
            bytes,
            policy: PhantomData,
        })
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

    /// Moves validated input bytes into the execution state boundary.
    fn into_runtime_bytes(self) -> Vec<RuntimeByte> {
        self.bytes
    }
}

/// Run-start witness tying checked input to an execution policy.
///
/// A seed is the only public value accepted by execution entrypoints. It
/// carries both the initial runtime-state bytes and the already checked budget
/// state, so `Program::run`, `Program::start_run`, and `Program::into_run` do
/// not need to reinterpret raw input or detached execution policy values.
#[derive(Debug, PartialEq, Eq)]
pub struct RunSeed<E: ExecutionPolicy = DefaultExecutionPolicy> {
    /// Runtime-domain bytes admitted as the initial execution state.
    initial_state: InitialStateBytes,
    /// Execution budgets already tied to this admitted run.
    budget: RuntimeBudgetState<E>,
}

impl<E: ExecutionPolicy> RunSeed<E> {
    /// Admits validated runtime input for execution under an execution policy.
    ///
    /// This consumes the validated input. A successful seed represents one
    /// admitted execution start; it is not a reusable input buffer.
    ///
    /// # Errors
    ///
    /// Returns `RunAdmissionError` if the validated input would exceed the
    /// initial runtime-state budget.
    pub fn admit<I: RuntimeInputPolicy>(input: RuntimeInput<I>) -> Result<Self, RunAdmissionError> {
        let initial_state_len = RuntimeStateByteCount::from_runtime_input_count(input.byte_count());
        let limit = E::STATE_BYTE_LIMIT;
        if !limit.accepts(initial_state_len) {
            return Err(RunAdmissionError::initial_state_limit(
                limit,
                initial_state_len,
            ));
        }

        Ok(Self {
            initial_state: InitialStateBytes {
                bytes: input.into_runtime_bytes(),
            },
            budget: RuntimeBudgetState::new(),
        })
    }

    /// Splits the admitted run seed into runtime state bytes and budget state.
    pub(crate) fn into_runtime_parts(self) -> (InitialStateBytes, RuntimeBudgetState<E>) {
        (self.initial_state, self.budget)
    }
}

/// Runtime input materialized into the mutable execution byte domain.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct InitialStateBytes {
    /// Runtime-domain bytes used to initialize execution state.
    bytes: Vec<RuntimeByte>,
}

impl InitialStateBytes {
    /// Moves initial state bytes into the runtime core.
    pub(crate) fn into_runtime_bytes(self) -> Vec<RuntimeByte> {
        self.bytes
    }
}
