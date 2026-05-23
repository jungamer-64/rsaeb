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

/// Internal payload.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Payload {
    /// Stored bytes.
    bytes: Vec<ProgramByte>,
}

/// Internal payload syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PayloadSyntax<'code> {
    /// Stored bytes.
    bytes: &'code [CompactByte],
    /// Stored line number.
    line_number: SourceLineNumber,
    /// Stored payload kind.
    payload_kind: PayloadKind,
}

/// Internal validated payload syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ValidatedPayloadSyntax<'code> {
    /// Stored syntax.
    syntax: PayloadSyntax<'code>,
    /// Stored byte count.
    byte_count: PayloadByteCount,
}

impl<'code> PayloadSyntax<'code> {
    /// Constructs the value from validated parts.
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

    /// Returns the typed byte count.
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
    /// Returns the runtime state length in bytes.
    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Returns the typed byte count.
    pub(crate) fn byte_count(&self) -> PayloadByteCount {
        PayloadByteCount::new(self.bytes.len())
    }

    /// Runs the program bytes operation.
    pub(crate) fn program_bytes(&self) -> &[ProgramByte] {
        &self.bytes
    }

    /// Runs the needle operation.
    pub(crate) fn needle(&self) -> PayloadNeedle<'_> {
        match self.bytes.split_first() {
            Some((&first, _)) => PayloadNeedle::NonEmpty(NonEmptyPayloadNeedle {
                payload: self,
                first,
            }),
            None => PayloadNeedle::Empty(EmptyPayloadNeedle { payload: self }),
        }
    }

    /// Returns the stored bytes.
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

    /// Runs the runtime bytes operation.
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

/// Internal payload needle alternatives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PayloadNeedle<'payload> {
    /// Empty case.
    Empty(EmptyPayloadNeedle<'payload>),
    /// Non empty case.
    NonEmpty(NonEmptyPayloadNeedle<'payload>),
}

/// Internal empty payload needle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EmptyPayloadNeedle<'payload> {
    /// Stored payload.
    payload: &'payload Payload,
}

/// Internal non empty payload needle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NonEmptyPayloadNeedle<'payload> {
    /// Stored payload.
    payload: &'payload Payload,
    /// Stored first.
    first: ProgramByte,
}

impl EmptyPayloadNeedle<'_> {
    /// Returns the typed byte count.
    pub(crate) fn byte_count(self) -> PayloadByteCount {
        self.payload.byte_count()
    }
}

impl<'payload> NonEmptyPayloadNeedle<'payload> {
    /// Returns the typed byte count.
    pub(crate) fn byte_count(self) -> PayloadByteCount {
        self.payload.byte_count()
    }

    /// Runs the first byte operation.
    pub(crate) const fn first_byte(self) -> ProgramByte {
        self.first
    }

    /// Runs the program bytes operation.
    pub(crate) fn program_bytes(self) -> &'payload [ProgramByte] {
        self.payload.program_bytes()
    }
}
