use crate::error::{InputColumn, RuntimeInputError, RuntimeInputInvariantError};

use super::program::ProgramByte;
use super::rejection::NonAsciiInputByte;

use runtime_ascii::{AsciiByte, ClassifiedAsciiByte, NonProgramAsciiByte};

/// ASCII validation layer for host runtime input.
mod runtime_ascii {
    use super::{InputColumn, NonAsciiInputByte, ProgramByte, RuntimeInputError};

    /// ASCII byte accepted by runtime input.
    ///
    /// Raw runtime input crosses into this domain only through validation.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) struct AsciiByte(u8);

    /// Runtime-domain classification of validated ASCII input.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum ClassifiedAsciiByte {
        /// Byte can also be constructed by program payload syntax.
        Program(ProgramByte),
        /// Byte is valid runtime input but cannot appear in executable payloads.
        NonProgram(NonProgramAsciiByte),
    }

    /// Runtime-only ASCII byte kept opaque to program payload matching.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) struct NonProgramAsciiByte(AsciiByte);

    impl AsciiByte {
        /// Validates one raw runtime input byte as ASCII.
        ///
        /// # Errors
        ///
        /// Returns `RuntimeInputError` when the byte is non-ASCII or its
        /// one-based input column cannot be represented.
        pub(crate) fn validate(
            byte: u8,
            zero_based_column: usize,
        ) -> Result<Self, RuntimeInputError> {
            if let Some(rejected) = NonAsciiInputByte::parse(byte) {
                let column = InputColumn::from_zero_based(zero_based_column)
                    .ok_or_else(RuntimeInputError::column_overflow)?;
                Err(RuntimeInputError::non_ascii(column, rejected))
            } else {
                Ok(Self(byte))
            }
        }

        /// Rebuilds an ASCII byte from a validated runtime-input witness.
        pub(crate) const fn from_validated_raw(byte: u8) -> Option<Self> {
            if byte.is_ascii() {
                Some(Self(byte))
            } else {
                None
            }
        }

        /// Returns the primitive stored value.
        pub(crate) const fn get(self) -> u8 {
            self.0
        }

        /// Separates executable payload bytes from runtime-only bytes.
        pub(crate) fn classify(self) -> ClassifiedAsciiByte {
            if let Some(byte) = ProgramByte::from_valid_raw(self.get()) {
                ClassifiedAsciiByte::Program(byte)
            } else {
                ClassifiedAsciiByte::NonProgram(NonProgramAsciiByte(self))
            }
        }
    }

    impl NonProgramAsciiByte {
        /// Returns the original ASCII byte.
        pub(crate) const fn materialize(self) -> u8 {
            self.0.get()
        }
    }
}

/// A raw runtime-input byte after input-boundary validation.
///
/// This witness is the only host-byte path into the runtime byte
/// domain. It keeps the validation result attached until the byte is classified
/// for mutable runtime state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RuntimeInputByte {
    /// ASCII witness for one validated runtime-input byte.
    byte: AsciiByte,
}

impl RuntimeInputByte {
    /// Validates one raw runtime input byte as a runtime-input byte.
    ///
    /// # Errors
    ///
    /// Returns `RuntimeInputError` when ASCII validation fails or the input
    /// column cannot be represented.
    pub(crate) fn validate(byte: u8, zero_based_column: usize) -> Result<Self, RuntimeInputError> {
        Ok(Self {
            byte: AsciiByte::validate(byte, zero_based_column)?,
        })
    }

    /// Rebuilds a runtime-input byte from a previously validated witness.
    ///
    /// # Errors
    ///
    /// Returns `RuntimeInputInvariantError` if the witness no longer satisfies
    /// the runtime-input ASCII contract.
    pub(crate) fn from_validated_ascii(byte: u8) -> Result<Self, RuntimeInputInvariantError> {
        Ok(Self {
            byte: AsciiByte::from_validated_raw(byte)
                .ok_or(RuntimeInputInvariantError::MissingValidatedAsciiByte)?,
        })
    }

    /// Classifies this input byte for storage in mutable runtime state.
    pub(crate) fn into_runtime_byte(self) -> RuntimeByte {
        RuntimeByte::from_ascii(self.byte)
    }
}

/// A byte inside the mutable runtime state.
///
/// Program-constructible bytes and runtime-only bytes are separate variants, so
/// matching executable payloads cannot accidentally treat whitespace, control
/// bytes, or reserved syntax as program payload data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeByte {
    /// Byte may participate in executable payload matching.
    ProgramConstructible(ProgramByte),
    /// Byte remains visible in state but never matches program payload syntax.
    Opaque(NonProgramAsciiByte),
}

impl RuntimeByte {
    /// Lifts a program payload byte into the runtime-state domain.
    pub(crate) const fn from_program(byte: ProgramByte) -> Self {
        Self::ProgramConstructible(byte)
    }

    /// Classifies a validated ASCII byte for runtime-state storage.
    fn from_ascii(byte: AsciiByte) -> Self {
        match byte.classify() {
            ClassifiedAsciiByte::Program(byte) => Self::ProgramConstructible(byte),
            ClassifiedAsciiByte::NonProgram(byte) => Self::Opaque(byte),
        }
    }

    /// Returns the byte value visible to callers and trace output.
    pub(crate) const fn materialize(self) -> u8 {
        match self {
            Self::ProgramConstructible(byte) => byte.get(),
            Self::Opaque(byte) => byte.materialize(),
        }
    }

    /// Returns the executable payload byte when this state byte is matchable.
    pub(crate) const fn program_byte(self) -> Option<ProgramByte> {
        match self {
            Self::ProgramConstructible(byte) => Some(byte),
            Self::Opaque(_) => None,
        }
    }
}
