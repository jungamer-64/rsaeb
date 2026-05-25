use crate::error::{ParseError, ParseErrorKind};
use crate::source::{SourceColumn, SourcePosition};

use super::rejection::{NonAsciiCodeByte, NonPrintableCodeByte};

/// Source byte accepted as executable A=B code before whitespace compaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExecutableCodeByte {
    /// Executable source byte.
    byte: u8,
    /// Original source column retained for diagnostics.
    source_column: SourceColumn,
}

/// Executable source byte after whitespace removal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CompactByte {
    /// Compact executable byte.
    byte: u8,
    /// Original source column retained for diagnostics.
    source_column: SourceColumn,
}

impl ExecutableCodeByte {
    /// Validates one non-whitespace source byte as executable code.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` when the byte is non-ASCII or non-printable.
    pub(crate) fn validate(byte: u8, position: SourcePosition) -> Result<Self, ParseError> {
        if let Some(rejected) = NonAsciiCodeByte::parse(byte) {
            return Err(ParseError::at_position(
                position,
                ParseErrorKind::NonAsciiInCode { byte: rejected },
            ));
        }

        if let Some(rejected) = NonPrintableCodeByte::parse(byte) {
            return Err(ParseError::at_position(
                position,
                ParseErrorKind::NonPrintableAsciiInCode { byte: rejected },
            ));
        }

        Ok(Self {
            byte,
            source_column: position.column(),
        })
    }

    /// Returns the raw executable byte.
    pub(crate) const fn as_u8(self) -> u8 {
        self.byte
    }

    /// Original source column for parse errors involving this byte.
    pub(crate) const fn source_column(self) -> SourceColumn {
        self.source_column
    }
}

impl CompactByte {
    /// Compacts a validated executable byte after whitespace has been removed.
    pub(crate) const fn from_executable(byte: ExecutableCodeByte) -> Self {
        Self {
            byte: byte.as_u8(),
            source_column: byte.source_column(),
        }
    }

    /// Returns the as u8 view.
    pub(crate) const fn as_u8(self) -> u8 {
        self.byte
    }

    /// Original source column for parse errors involving this byte.
    pub(crate) const fn source_column(self) -> SourceColumn {
        self.source_column
    }
}
