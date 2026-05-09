use alloc::vec::Vec;

use crate::allocation::{try_push, try_reserve_total_exact, AllocationContext, AllocationError};
use crate::error::{InputError, ParseError, ParseErrorKind, PayloadKind, RunError};

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

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CodeByte(u8);

impl CodeByte {
    pub(crate) fn parse_compact(
        byte: CompactByte,
        line_number: usize,
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

    pub(crate) const fn as_u8(self) -> u8 {
        self.0
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RuntimeByte(u8);

impl RuntimeByte {
    pub(crate) fn parse_input(byte: u8, zero_based_column: usize) -> Result<Self, InputError> {
        if !byte.is_ascii() {
            return Err(InputError {
                column: zero_based_column + 1,
                byte,
            });
        }

        Ok(Self(byte))
    }

    pub(crate) const fn from_code(byte: CodeByte) -> Self {
        Self(byte.as_u8())
    }

    pub(crate) const fn as_u8(self) -> u8 {
        self.0
    }
}

fn copy_code_bytes(
    source: &[CodeByte],
    context: AllocationContext,
) -> Result<Vec<u8>, AllocationError> {
    let mut output = Vec::new();
    try_reserve_total_exact(&mut output, source.len(), context)?;

    for byte in source.iter().copied() {
        output.push(byte.as_u8());
    }

    Ok(output)
}

pub(crate) fn copy_runtime_bytes(
    source: &[RuntimeByte],
    context: AllocationContext,
) -> Result<Vec<u8>, AllocationError> {
    let mut output = Vec::new();
    try_reserve_total_exact(&mut output, source.len(), context)?;

    for byte in source.iter().copied() {
        output.push(byte.as_u8());
    }

    Ok(output)
}

pub(crate) fn push_runtime_bytes(
    output: &mut Vec<RuntimeByte>,
    source: impl IntoIterator<Item = RuntimeByte>,
) -> Result<(), AllocationError> {
    for byte in source {
        try_push(output, byte, AllocationContext::RuntimeState)?;
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Payload {
    bytes: Vec<CodeByte>,
}

impl Payload {
    pub(crate) fn parse(
        input: &[CompactByte],
        line_number: usize,
        payload_kind: PayloadKind,
    ) -> Result<Self, ParseError> {
        let mut bytes = Vec::new();
        try_reserve_total_exact(&mut bytes, input.len(), AllocationContext::Payload)
            .map_err(|error| parse_allocation_error(line_number, error))?;

        for byte in input.iter().copied() {
            bytes.push(CodeByte::parse_compact(byte, line_number, payload_kind)?);
        }

        Ok(Self { bytes })
    }

    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub(crate) fn bytes(&self) -> &[CodeByte] {
        &self.bytes
    }

    pub(crate) fn runtime_bytes(&self) -> impl Iterator<Item = RuntimeByte> + '_ {
        self.bytes.iter().copied().map(RuntimeByte::from_code)
    }

    pub(crate) fn to_output(&self) -> Result<Vec<u8>, AllocationError> {
        copy_code_bytes(&self.bytes, AllocationContext::ReturnOutput)
    }
}

pub(crate) struct CompactByte {
    byte: u8,
    source_column: usize,
}

impl CompactByte {
    pub(crate) const fn new(byte: u8, source_column: usize) -> Self {
        Self {
            byte,
            source_column,
        }
    }

    pub(crate) const fn as_u8(self) -> u8 {
        self.byte
    }

    pub(crate) const fn source_column(self) -> usize {
        self.source_column
    }
}

