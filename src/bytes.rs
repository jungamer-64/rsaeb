use alloc::vec::Vec;
use core::fmt;

use crate::allocation::{AllocationContext, AllocationError, try_push, try_reserve_total_exact};
use crate::error::{InputColumn, InputError, ParseError, ParseErrorKind, PayloadKind};
use crate::source::{SourceColumn, SourceLineNumber, SourcePosition};

/// Byte length of executable program payload data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PayloadByteCount {
    value: usize,
}

impl PayloadByteCount {
    /// Creates a payload byte count from a primitive length.
    #[must_use]
    pub(crate) const fn new(value: usize) -> Self {
        Self { value }
    }

    /// Returns this byte count as a primitive length.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }

    /// Returns whether this count is zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.value == 0
    }
}

impl fmt::Display for PayloadByteCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}

/// Byte length of materialized runtime state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuntimeStateByteCount {
    value: usize,
}

impl RuntimeStateByteCount {
    /// Creates a runtime-state byte count from a primitive length.
    #[must_use]
    pub(crate) const fn new(value: usize) -> Self {
        Self { value }
    }

    /// Returns this byte count as a primitive length.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }

    /// Returns whether this count is zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.value == 0
    }
}

impl fmt::Display for RuntimeStateByteCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}

/// Byte length of a `(return)` output payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReturnOutputByteCount {
    value: usize,
}

impl ReturnOutputByteCount {
    /// Creates a `(return)` output byte count from a primitive length.
    #[must_use]
    pub(crate) const fn new(value: usize) -> Self {
        Self { value }
    }

    /// Returns this byte count as a primitive length.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }

    /// Returns whether this count is zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.value == 0
    }
}

impl fmt::Display for ReturnOutputByteCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}

/// Byte length budgeted for one trace snapshot event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TraceSnapshotByteCount {
    value: usize,
}

impl TraceSnapshotByteCount {
    /// Creates a trace snapshot byte count from a primitive length.
    #[must_use]
    pub(crate) const fn new(value: usize) -> Self {
        Self { value }
    }

    /// Returns this byte count as a primitive length.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }

    /// Returns whether this count is zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.value == 0
    }
}

impl fmt::Display for TraceSnapshotByteCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}

/// Non-ASCII byte rejected from executable program code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NonAsciiCodeByte {
    byte: u8,
}

impl NonAsciiCodeByte {
    pub(crate) const fn parse(byte: u8) -> Option<Self> {
        if byte.is_ascii() {
            None
        } else {
            Some(Self { byte })
        }
    }

    /// Returns the rejected raw byte.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.byte
    }
}

/// Non-printable ASCII byte rejected from executable program code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NonPrintableCodeByte {
    byte: u8,
}

impl NonPrintableCodeByte {
    pub(crate) const fn parse(byte: u8) -> Option<Self> {
        if byte.is_ascii() && !byte.is_ascii_graphic() {
            Some(Self { byte })
        } else {
            None
        }
    }

    /// Returns the rejected raw byte.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.byte
    }
}

/// Non-ASCII byte rejected from runtime input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NonAsciiInputByte {
    byte: u8,
}

impl NonAsciiInputByte {
    pub(crate) const fn parse(byte: u8) -> Option<Self> {
        if byte.is_ascii() {
            None
        } else {
            Some(Self { byte })
        }
    }

    /// Returns the rejected raw byte.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.byte
    }
}

/// Reserved executable syntax byte rejected from program payload data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReservedSyntaxByte {
    /// The `=` rule separator byte.
    Equals,
    /// The `#` line-comment byte.
    Comment,
    /// The `(` modifier/action opening byte.
    OpenParen,
    /// The `)` modifier/action closing byte.
    CloseParen,
}

impl ReservedSyntaxByte {
    pub(crate) const fn parse(byte: u8) -> Option<Self> {
        match byte {
            b'=' => Some(Self::Equals),
            b'#' => Some(Self::Comment),
            b'(' => Some(Self::OpenParen),
            b')' => Some(Self::CloseParen),
            _ => None,
        }
    }

    /// Returns the reserved raw syntax byte.
    #[must_use]
    pub const fn get(self) -> u8 {
        match self {
            Self::Equals => b'=',
            Self::Comment => b'#',
            Self::OpenParen => b'(',
            Self::CloseParen => b')',
        }
    }
}

/// A byte that is valid executable A=B payload data.
///
/// This is deliberately narrower than runtime input. Program bytes can be
/// matched and constructed by rules. Runtime-only bytes can be preserved, but
/// cannot be matched, created, or deleted directly by ordinary rewrite payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ProgramByte(u8);

impl ProgramByte {
    pub(crate) const fn is_valid_raw(raw: u8) -> bool {
        raw.is_ascii_graphic() && ReservedSyntaxByte::parse(raw).is_none()
    }

    pub(crate) const fn from_valid_raw(raw: u8) -> Option<Self> {
        if Self::is_valid_raw(raw) {
            Some(Self(raw))
        } else {
            None
        }
    }

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

    pub(crate) const fn get(self) -> u8 {
        self.0
    }
}

/// ASCII byte accepted by runtime input.
///
/// This newtype prevents crate-internal code from constructing an opaque
/// runtime byte with a non-ASCII value. Runtime input validation owns the only
/// constructor that can cross from raw `u8` into this domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AsciiByte(u8);

impl AsciiByte {
    pub(crate) fn validate(byte: u8, zero_based_column: usize) -> Result<Self, InputError> {
        if let Some(rejected) = NonAsciiInputByte::parse(byte) {
            let column = InputColumn::from_zero_based(zero_based_column)
                .ok_or_else(InputError::column_overflow)?;
            Err(InputError::non_ascii(column, rejected))
        } else {
            Ok(Self(byte))
        }
    }

    pub(crate) const fn from_validated_input(byte: u8) -> Self {
        Self(byte)
    }

    pub(crate) const fn get(self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OpaqueRuntimeByte(AsciiByte);

impl OpaqueRuntimeByte {
    const fn new(byte: AsciiByte) -> Self {
        Self(byte)
    }

    pub(crate) const fn materialize(self) -> u8 {
        self.0.get()
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
    Opaque(OpaqueRuntimeByte),
}

impl RuntimeByte {
    pub(crate) fn validate_input(byte: u8, zero_based_column: usize) -> Result<Self, InputError> {
        Ok(Self::from_ascii(AsciiByte::validate(
            byte,
            zero_based_column,
        )?))
    }

    pub(crate) fn from_validated_input(byte: u8) -> Self {
        Self::from_ascii(AsciiByte::from_validated_input(byte))
    }

    pub(crate) const fn from_program(byte: ProgramByte) -> Self {
        Self::ProgramConstructible(byte)
    }

    fn from_ascii(byte: AsciiByte) -> Self {
        if let Some(program_byte) = ProgramByte::from_valid_raw(byte.get()) {
            Self::ProgramConstructible(program_byte)
        } else {
            Self::Opaque(OpaqueRuntimeByte::new(byte))
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

pub(crate) fn push_bytes(
    output: &mut Vec<u8>,
    source: impl IntoIterator<Item = u8>,
    context: AllocationContext,
) -> Result<(), AllocationError> {
    for byte in source {
        try_push(output, byte, context)?;
    }

    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Payload {
    bytes: Vec<ProgramByte>,
}

impl Payload {
    pub(crate) fn parse(
        input: &[CompactByte],
        line_number: SourceLineNumber,
        payload_kind: PayloadKind,
    ) -> Result<Self, ParseError> {
        // Validate the whole payload before allocation so syntax errors keep
        // precedence over allocation failures.
        for byte in input.iter().copied() {
            ProgramByte::parse(byte, line_number, payload_kind)?;
        }

        let mut bytes = Vec::new();
        try_reserve_total_exact(&mut bytes, input.len(), AllocationContext::ProgramPayload)
            .map_err(|error| ParseError::at_line(line_number, ParseErrorKind::Allocation(error)))?;

        for byte in input.iter().copied() {
            let parsed = ProgramByte::parse(byte, line_number, payload_kind)?;
            try_push(&mut bytes, parsed, AllocationContext::ProgramPayload).map_err(|error| {
                ParseError::at_line(line_number, ParseErrorKind::Allocation(error))
            })?;
        }

        Ok(Self { bytes })
    }

    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn byte_count(&self) -> PayloadByteCount {
        PayloadByteCount::new(self.bytes.len())
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub(crate) fn first_byte(&self) -> Option<ProgramByte> {
        self.bytes.first().copied()
    }

    pub(crate) fn program_bytes(&self) -> &[ProgramByte] {
        &self.bytes
    }

    pub(crate) fn bytes(&self) -> impl Iterator<Item = u8> + '_ {
        self.bytes.iter().copied().map(ProgramByte::get)
    }

    pub(crate) fn eq_bytes(&self, expected: &[u8]) -> bool {
        self.len() == expected.len()
            && self
                .bytes()
                .zip(expected.iter().copied())
                .all(|(actual, expected)| actual == expected)
    }

    pub(crate) fn push_bytes_to(
        &self,
        output: &mut Vec<u8>,
        context: AllocationContext,
    ) -> Result<(), AllocationError> {
        push_bytes(output, self.bytes(), context)
    }

    pub(crate) fn runtime_bytes(&self) -> impl Iterator<Item = RuntimeByte> + '_ {
        self.bytes.iter().copied().map(RuntimeByte::from_program)
    }

    pub(crate) fn to_vec_with_context(
        &self,
        context: AllocationContext,
    ) -> Result<Vec<u8>, AllocationError> {
        let mut output = Vec::new();
        try_reserve_total_exact(&mut output, self.len(), context)?;
        self.push_bytes_to(&mut output, context)?;
        Ok(output)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CompactByte {
    byte: u8,
    source_column: SourceColumn,
}

impl CompactByte {
    pub(crate) const fn new(byte: u8, source_column: SourceColumn) -> Self {
        Self {
            byte,
            source_column,
        }
    }

    pub(crate) const fn as_u8(self) -> u8 {
        self.byte
    }

    pub(crate) const fn source_column(self) -> SourceColumn {
        self.source_column
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{ParseError, ParseErrorKind, PayloadKind};
    use crate::test_support::{
        TestFailure, TestResult, ensure, ensure_eq, ensure_matches, expect_error_position,
        source_column, source_line_number,
    };

    fn parse_payload_error(
        input: &[CompactByte],
        line_number: SourceLineNumber,
        payload_kind: PayloadKind,
    ) -> Result<ParseError, TestFailure> {
        match Payload::parse(input, line_number, payload_kind) {
            Ok(_) => Err(TestFailure::message("invalid payload bytes were accepted")),
            Err(error) => Ok(error),
        }
    }

    #[test]
    fn payload_rejects_every_reserved_syntax_byte_even_if_payload_parser_is_called_directly()
    -> TestResult {
        for reserved in [b'=', b'#', b'(', b')'] {
            let compact = [CompactByte::new(reserved, source_column(1)?)];
            let error =
                parse_payload_error(&compact, source_line_number(1)?, PayloadKind::RightSideData)?;

            expect_error_position(&error, 1, 1)?;
            ensure_matches(
                matches!(
                    error.kind(),
                    ParseErrorKind::ReservedSyntaxInPayload { byte, .. }
                        if byte.get() == reserved
                ),
                "expected concrete reserved syntax byte",
            )?;
            ensure_matches(
                matches!(
                    error.kind(),
                    ParseErrorKind::ReservedSyntaxInPayload {
                        payload_kind: PayloadKind::RightSideData,
                        ..
                    }
                ),
                "expected reserved syntax payload error",
            )?;
        }
        Ok(())
    }

    #[test]
    fn payload_validates_compact_bytes_at_the_domain_boundary() -> TestResult {
        let non_ascii = [CompactByte::new(0xff, source_column(1)?)];
        let non_graphic = [CompactByte::new(b' ', source_column(2)?)];

        let error = parse_payload_error(
            &non_ascii,
            source_line_number(1)?,
            PayloadKind::RightSideData,
        )?;
        ensure_matches(
            matches!(error.kind(), ParseErrorKind::NonAsciiInCode { .. }),
            "expected non-ASCII parse error",
        )?;

        let error = parse_payload_error(
            &non_graphic,
            source_line_number(1)?,
            PayloadKind::RightSideData,
        )?;
        expect_error_position(&error, 1, 2)?;
        ensure_matches(
            matches!(error.kind(), ParseErrorKind::NonPrintableAsciiInCode { .. }),
            "expected non-printable parse error",
        )?;
        Ok(())
    }

    #[test]
    fn payload_exposes_validated_bytes_without_leaking_the_internal_domain_type() -> TestResult {
        let compact = [
            CompactByte::new(b'a', source_column(1)?),
            CompactByte::new(b'b', source_column(2)?),
        ];
        let payload = Payload::parse(&compact, source_line_number(1)?, PayloadKind::LeftSideData)
            .map_err(TestFailure::from)?;

        ensure(payload.eq_bytes(b"ab"), "expected payload bytes")?;
        ensure_eq!(payload.first_byte().map(ProgramByte::get), Some(b'a'))?;
        Ok(())
    }

    #[test]
    fn runtime_input_classifies_program_constructible_and_opaque_ascii_separately() -> TestResult {
        let parsed = RuntimeByte::validate_input(b'a', 0).map_err(TestFailure::from)?;
        ensure_matches(
            matches!(parsed, RuntimeByte::ProgramConstructible(byte) if byte.get() == b'a'),
            "expected program-constructible input byte",
        )?;
        ensure_eq!(parsed.materialize(), b'a')?;

        for byte in [0x00, b' ', b'=', b'#', b'(', b')'] {
            let parsed = RuntimeByte::validate_input(byte, 0).map_err(TestFailure::from)?;
            ensure_eq!(parsed.materialize(), byte)?;
            ensure_matches(
                matches!(parsed, RuntimeByte::Opaque(_)),
                "expected opaque input byte",
            )?;
        }

        Ok(())
    }
}
