use alloc::vec::Vec;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PayloadSyntax<'code> {
    bytes: &'code [CompactByte],
    line_number: SourceLineNumber,
    payload_kind: PayloadKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ValidatedPayloadSyntax<'code> {
    syntax: PayloadSyntax<'code>,
    byte_count: PayloadByteCount,
}

impl<'code> PayloadSyntax<'code> {
    pub(crate) const fn new(
        bytes: &'code [CompactByte],
        line_number: SourceLineNumber,
        payload_kind: PayloadKind,
    ) -> Self {
        Self {
            bytes,
            line_number,
            payload_kind,
        }
    }

    pub(crate) const fn byte_count(self) -> PayloadByteCount {
        PayloadByteCount::new(self.bytes.len())
    }

    /// Validates compact payload bytes before owned payload allocation.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if any byte is invalid executable payload data.
    pub(crate) fn validate(self) -> Result<ValidatedPayloadSyntax<'code>, ParseError> {
        for byte in self.bytes.iter().copied() {
            ProgramByte::parse(byte, self.line_number, self.payload_kind)?;
        }

        Ok(ValidatedPayloadSyntax {
            syntax: self,
            byte_count: self.byte_count(),
        })
    }
}

impl ValidatedPayloadSyntax<'_> {
    /// Builds an owned executable payload from validated syntax.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if allocation fails while storing the payload.
    pub(crate) fn into_payload(self) -> Result<Payload, ParseError> {
        // Allocation starts only after the full syntax pass, so syntax errors
        // are not reordered behind storage failures.
        let mut bytes = Vec::new();
        try_reserve_total_exact(
            &mut bytes,
            RequestedCapacity::new(self.byte_count.get()),
            AllocationContext::ProgramPayload,
        )
        .map_err(|error| {
            ParseError::at_line(self.syntax.line_number, ParseErrorKind::Allocation(error))
        })?;

        for byte in self.syntax.bytes.iter().copied() {
            let parsed = ProgramByte::from_validated_compact(byte);
            try_push(&mut bytes, parsed, AllocationContext::ProgramPayload).map_err(|error| {
                ParseError::at_line(self.syntax.line_number, ParseErrorKind::Allocation(error))
            })?;
        }

        Ok(Payload { bytes })
    }
}

impl Payload {
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
        try_reserve_total_exact(&mut output, RequestedCapacity::new(self.len()), context)?;
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
