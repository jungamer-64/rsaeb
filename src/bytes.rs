use alloc::vec::Vec;

use crate::allocation::{AllocationContext, AllocationError, try_push, try_reserve_total_exact};
use crate::error::{InputError, ParseError, ParseErrorKind, PayloadKind};
use crate::source::{SourceColumn, SourceLineNumber};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReservedSyntaxByte {
    Equals,
    Comment,
    OpenParen,
    CloseParen,
}

impl ReservedSyntaxByte {
    const fn parse(byte: u8) -> Option<Self> {
        match byte {
            b'=' => Some(Self::Equals),
            b'#' => Some(Self::Comment),
            b'(' => Some(Self::OpenParen),
            b')' => Some(Self::CloseParen),
            _ => None,
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
    pub(crate) fn parse(
        byte: CompactByte,
        line_number: SourceLineNumber,
        payload_kind: PayloadKind,
    ) -> Result<Self, ParseError> {
        let raw = byte.as_u8();

        if !raw.is_ascii() {
            return Err(ParseError::new(
                line_number,
                Some(byte.source_column()),
                ParseErrorKind::NonAsciiInCode { byte: raw },
            ));
        }

        if !raw.is_ascii_graphic() {
            return Err(ParseError::new(
                line_number,
                Some(byte.source_column()),
                ParseErrorKind::NonPrintableAsciiInCode { byte: raw },
            ));
        }

        if ReservedSyntaxByte::parse(raw).is_some() {
            return Err(ParseError::new(
                line_number,
                Some(byte.source_column()),
                ParseErrorKind::ReservedSyntaxInPayload {
                    byte: raw,
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
    pub(crate) fn parse(byte: u8, zero_based_column: usize) -> Result<Self, InputError> {
        if byte.is_ascii() {
            Ok(Self(byte))
        } else {
            Err(InputError::new(zero_based_column + 1, byte))
        }
    }

    pub(crate) const fn get(self) -> u8 {
        self.0
    }
}

/// A byte inside the mutable runtime state.
///
/// `Editable` bytes came from user input as ordinary payload-compatible bytes or
/// from rule payloads. `Opaque` bytes came from runtime input only. Rules cannot
/// directly match opaque bytes, so program syntax bytes like `=` and `#` can
/// survive in state without becoming part of the program byte domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RuntimeByte(RuntimeByteRepr);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeByteRepr {
    Editable(ProgramByte),
    Opaque(AsciiByte),
}

impl RuntimeByte {
    pub(crate) fn parse_input(byte: u8, zero_based_column: usize) -> Result<Self, InputError> {
        let byte = AsciiByte::parse(byte, zero_based_column)?;
        let raw = byte.get();

        if raw.is_ascii_graphic() && ReservedSyntaxByte::parse(raw).is_none() {
            Ok(Self(RuntimeByteRepr::Editable(ProgramByte(raw))))
        } else {
            Ok(Self(RuntimeByteRepr::Opaque(byte)))
        }
    }

    pub(crate) const fn from_program(byte: ProgramByte) -> Self {
        Self(RuntimeByteRepr::Editable(byte))
    }

    pub(crate) const fn materialize(self) -> u8 {
        match self.0 {
            RuntimeByteRepr::Editable(byte) => byte.get(),
            RuntimeByteRepr::Opaque(byte) => byte.get(),
        }
    }

    pub(crate) const fn matches_program_byte(self, expected: ProgramByte) -> bool {
        match self.0 {
            RuntimeByteRepr::Editable(byte) => byte.get() == expected.get(),
            RuntimeByteRepr::Opaque(_) => false,
        }
    }

    #[cfg(test)]
    pub(crate) const fn is_editable(self) -> bool {
        matches!(self.0, RuntimeByteRepr::Editable(_))
    }

    #[cfg(test)]
    pub(crate) const fn is_opaque(self) -> bool {
        matches!(self.0, RuntimeByteRepr::Opaque(_))
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
        for byte in input.iter().copied() {
            ProgramByte::parse(byte, line_number, payload_kind)?;
        }

        let mut bytes = Vec::new();
        try_reserve_total_exact(&mut bytes, input.len(), AllocationContext::Payload).map_err(
            |error| ParseError::new(line_number, None, ParseErrorKind::Allocation(error)),
        )?;

        for byte in input.iter().copied() {
            let parsed = ProgramByte::parse(byte, line_number, payload_kind)?;
            try_push(&mut bytes, parsed, AllocationContext::Payload).map_err(|error| {
                ParseError::new(line_number, None, ParseErrorKind::Allocation(error))
            })?;
        }

        Ok(Self { bytes })
    }

    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
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
    use crate::{ParseError, ParseErrorKind, PayloadKind};

    fn parse_payload_error(
        input: &[CompactByte],
        line_number: SourceLineNumber,
        payload_kind: PayloadKind,
    ) -> Result<ParseError, &'static str> {
        match Payload::parse(input, line_number, payload_kind) {
            Ok(_) => Err("invalid payload bytes were accepted"),
            Err(error) => Ok(error),
        }
    }

    #[test]
    fn payload_rejects_every_reserved_syntax_byte_even_if_payload_parser_is_called_directly()
    -> Result<(), &'static str> {
        for reserved in [b'=', b'#', b'(', b')'] {
            let compact = [CompactByte::new(
                reserved,
                SourceColumn::from_one_based_unchecked(1),
            )];
            let error = parse_payload_error(
                &compact,
                SourceLineNumber::from_one_based_unchecked(1),
                PayloadKind::RightSideData,
            )?;

            assert_eq!(error.column().map(SourceColumn::get), Some(1));
            assert!(matches!(
                error.kind(),
                ParseErrorKind::ReservedSyntaxInPayload {
                    payload_kind: PayloadKind::RightSideData,
                    ..
                }
            ));
        }
        Ok(())
    }

    #[test]
    fn payload_validates_compact_bytes_at_the_domain_boundary() -> Result<(), &'static str> {
        let non_ascii = [CompactByte::new(
            0xff,
            SourceColumn::from_one_based_unchecked(1),
        )];
        let non_graphic = [CompactByte::new(
            b' ',
            SourceColumn::from_one_based_unchecked(2),
        )];

        let error = parse_payload_error(
            &non_ascii,
            SourceLineNumber::from_one_based_unchecked(1),
            PayloadKind::RightSideData,
        )?;
        assert!(matches!(
            error.kind(),
            ParseErrorKind::NonAsciiInCode { .. }
        ));

        let error = parse_payload_error(
            &non_graphic,
            SourceLineNumber::from_one_based_unchecked(1),
            PayloadKind::RightSideData,
        )?;
        assert_eq!(error.column().map(SourceColumn::get), Some(2));
        assert!(matches!(
            error.kind(),
            ParseErrorKind::NonPrintableAsciiInCode { .. }
        ));
        Ok(())
    }

    #[test]
    fn payload_exposes_validated_bytes_without_leaking_the_internal_domain_type()
    -> Result<(), &'static str> {
        let compact = [
            CompactByte::new(b'a', SourceColumn::from_one_based_unchecked(1)),
            CompactByte::new(b'b', SourceColumn::from_one_based_unchecked(2)),
        ];
        let payload = Payload::parse(
            &compact,
            SourceLineNumber::from_one_based_unchecked(1),
            PayloadKind::LeftSideData,
        )
        .map_err(|_| "expected payload to parse")?;

        assert!(payload.eq_bytes(b"ab"));
        assert_eq!(payload.first_byte().map(ProgramByte::get), Some(b'a'));
        Ok(())
    }

    #[test]
    fn runtime_input_classifies_program_constructible_and_opaque_ascii_separately()
    -> Result<(), &'static str> {
        let parsed = RuntimeByte::parse_input(b'a', 0).map_err(|_| "ASCII input should parse")?;
        assert!(parsed.is_editable());
        assert_eq!(parsed.materialize(), b'a');

        for byte in [0x00, b' ', b'=', b'#', b'(', b')'] {
            let parsed =
                RuntimeByte::parse_input(byte, 0).map_err(|_| "ASCII input should parse")?;
            assert_eq!(parsed.materialize(), byte);
            assert!(parsed.is_opaque());
        }

        Ok(())
    }
}
