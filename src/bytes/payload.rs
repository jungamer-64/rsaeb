use alloc::vec::Vec;

use crate::allocation::{AllocationContext, AllocationError, try_push, try_reserve_total_exact};
use crate::error::{ParseError, ParseErrorKind, PayloadKind};
use crate::source::SourceLineNumber;

use super::compact::CompactByte;
use super::count::PayloadByteCount;
use super::program::ProgramByte;
use super::runtime::RuntimeByte;

/// Pushes raw bytes through the explicit allocation boundary.
///
/// # Errors
///
/// Returns `AllocationError` if output capacity cannot be represented or
/// allocated.
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
    /// Parses compact bytes into typed executable payload data.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` when a payload byte is invalid for executable
    /// payload data, allocation fails, or payload allocation capacity
    /// overflows.
    pub(crate) fn parse(
        input: &[CompactByte],
        line_number: SourceLineNumber,
        payload_kind: PayloadKind,
    ) -> Result<Self, ParseError> {
        // Error ordering is part of the parser contract: payload syntax is
        // validated before any owned payload allocation, so invalid source
        // bytes cannot be hidden behind an allocation failure.
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

    pub(crate) fn program_bytes(&self) -> &[ProgramByte] {
        &self.bytes
    }

    pub(crate) fn needle(&self) -> PayloadNeedle<'_> {
        match self.bytes.split_first() {
            Some((&first, _)) => PayloadNeedle::NonEmpty(NonEmptyPayloadNeedle {
                payload: self,
                first,
            }),
            None => PayloadNeedle::Empty(EmptyPayloadNeedle { payload: self }),
        }
    }

    pub(crate) fn bytes(&self) -> impl Iterator<Item = u8> + '_ {
        self.bytes.iter().copied().map(ProgramByte::get)
    }

    /// Appends materialized payload bytes to `output`.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if output capacity cannot be represented or
    /// allocated.
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

    /// Materializes this payload as owned bytes for the given allocation site.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the output buffer cannot be allocated.
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
pub(crate) enum PayloadNeedle<'payload> {
    Empty(EmptyPayloadNeedle<'payload>),
    NonEmpty(NonEmptyPayloadNeedle<'payload>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EmptyPayloadNeedle<'payload> {
    payload: &'payload Payload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NonEmptyPayloadNeedle<'payload> {
    payload: &'payload Payload,
    first: ProgramByte,
}

impl EmptyPayloadNeedle<'_> {
    pub(crate) fn byte_count(self) -> PayloadByteCount {
        self.payload.byte_count()
    }
}

impl<'payload> NonEmptyPayloadNeedle<'payload> {
    pub(crate) fn byte_count(self) -> PayloadByteCount {
        self.payload.byte_count()
    }

    pub(crate) const fn first_byte(self) -> ProgramByte {
        self.first
    }

    pub(crate) fn program_bytes(self) -> &'payload [ProgramByte] {
        self.payload.program_bytes()
    }
}
