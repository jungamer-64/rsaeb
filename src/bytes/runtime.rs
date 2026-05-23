use crate::error::{InputColumn, RuntimeInputError};

use super::program::ProgramByte;
use super::rejection::NonAsciiInputByte;

use runtime_ascii::{AsciiByte, ClassifiedAsciiByte, NonProgramAsciiByte};

mod runtime_ascii {
    use super::{InputColumn, NonAsciiInputByte, ProgramByte, RuntimeInputError};

    /// ASCII byte accepted by runtime input.
    ///
    /// Raw runtime input crosses into this domain only through validation.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) struct AsciiByte(u8);

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum ClassifiedAsciiByte {
        Program(ProgramByte),
        NonProgram(NonProgramAsciiByte),
    }

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

        pub(crate) const fn get(self) -> u8 {
            self.0
        }

        pub(super) fn from_validated(byte: u8) -> Self {
            debug_assert!(byte.is_ascii());
            Self(byte)
        }

        pub(crate) fn classify(self) -> ClassifiedAsciiByte {
            if let Some(byte) = ProgramByte::from_valid_raw(self.get()) {
                ClassifiedAsciiByte::Program(byte)
            } else {
                ClassifiedAsciiByte::NonProgram(NonProgramAsciiByte(self))
            }
        }
    }

    impl NonProgramAsciiByte {
        pub(crate) const fn materialize(self) -> u8 {
            self.0.get()
        }
    }
}

/// A raw runtime-input byte after input-boundary validation.
///
/// This witness is the only path from host `u8` input into the runtime byte
/// domain. It keeps the validation result attached until the byte is classified
/// for mutable runtime state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RuntimeInputByte {
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

    pub(crate) fn from_validated_ascii(byte: u8) -> Self {
        Self {
            byte: AsciiByte::from_validated(byte),
        }
    }

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
    ProgramConstructible(ProgramByte),
    Opaque(NonProgramAsciiByte),
}

impl RuntimeByte {
    pub(crate) const fn from_program(byte: ProgramByte) -> Self {
        Self::ProgramConstructible(byte)
    }

    fn from_ascii(byte: AsciiByte) -> Self {
        match byte.classify() {
            ClassifiedAsciiByte::Program(byte) => Self::ProgramConstructible(byte),
            ClassifiedAsciiByte::NonProgram(byte) => Self::Opaque(byte),
        }
    }

    pub(crate) const fn materialize(self) -> u8 {
        match self {
            Self::ProgramConstructible(byte) => byte.get(),
            Self::Opaque(byte) => byte.materialize(),
        }
    }

    pub(crate) const fn program_byte(self) -> Option<ProgramByte> {
        match self {
            Self::ProgramConstructible(byte) => Some(byte),
            Self::Opaque(_) => None,
        }
    }
}
