use crate::error::{ParseError, ParseErrorKind, ParseInvariantError, PayloadKind};
use crate::source::{SourceLineNumber, SourcePosition};

use super::compact::CompactByte;
use super::rejection::{NonAsciiCodeByte, NonPrintableCodeByte, ReservedSyntaxByte};

/// A byte that is valid executable A=B payload data.
///
/// This is deliberately narrower than runtime input. Program bytes can be
/// matched and constructed by rules. Runtime-only bytes can be preserved, but
/// cannot be matched, created, or deleted directly by ordinary rewrite payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ProgramByte(u8);

impl ProgramByte {
    /// Builds the is valid raw value.
    pub(crate) const fn is_valid_raw(raw: u8) -> bool {
        raw.is_ascii_graphic() && ReservedSyntaxByte::parse(raw).is_none()
    }

    /// Classifies raw bytes that already satisfy executable payload rules.
    pub(crate) const fn from_valid_raw(raw: u8) -> Option<Self> {
        if Self::is_valid_raw(raw) {
            Some(Self(raw))
        } else {
            None
        }
    }

    /// Rebuilds a program byte from a previously validated payload witness.
    ///
    /// # Errors
    ///
    /// Returns `ParseInvariantError` if the validated witness no longer
    /// satisfies executable payload byte rules.
    pub(crate) fn from_validated_payload_byte(
        byte: CompactByte,
    ) -> Result<Self, ParseInvariantError> {
        Self::from_valid_raw(byte.as_u8()).ok_or(ParseInvariantError::ValidatedPayloadWithoutBytes)
    }

    /// Parses a compact source byte as executable program payload data.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` when the byte is non-ASCII, non-printable, or
    /// reserved executable syntax for the selected payload boundary.
    pub(crate) fn parse(
        byte: CompactByte,
        line_number: SourceLineNumber,
        payload_kind: PayloadKind,
    ) -> Result<Self, ParseError> {
        let raw = byte.as_u8();

        if let Some(rejected) = NonAsciiCodeByte::parse(raw) {
            return Err(ParseError::at_position(
                SourcePosition::new(line_number, byte.source_column()),
                ParseErrorKind::NonAsciiInCode { byte: rejected },
            ));
        }

        if let Some(rejected) = NonPrintableCodeByte::parse(raw) {
            return Err(ParseError::at_position(
                SourcePosition::new(line_number, byte.source_column()),
                ParseErrorKind::NonPrintableAsciiInCode { byte: rejected },
            ));
        }

        if let Some(rejected) = ReservedSyntaxByte::parse(raw) {
            return Err(ParseError::at_position(
                SourcePosition::new(line_number, byte.source_column()),
                ParseErrorKind::ReservedSyntaxInPayload {
                    byte: rejected,
                    payload_kind,
                },
            ));
        }

        Ok(Self(raw))
    }

    /// Returns the primitive stored value.
    pub(crate) const fn get(self) -> u8 {
        self.0
    }
}
