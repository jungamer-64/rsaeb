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

/// Executable payload bytes owned by a parsed rule.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Payload {
    /// Program-domain bytes accepted by payload syntax validation.
    bytes: Vec<ProgramByte>,
}

/// Borrowed compact syntax slice being validated as one payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PayloadSyntax<'code> {
    /// Compact source bytes for the candidate payload.
    bytes: &'code [CompactByte],
    /// Source line used for parse diagnostics.
    line_number: SourceLineNumber,
    /// Payload position determining the parse error domain.
    payload_kind: PayloadKind,
}

/// Payload syntax after every byte has been accepted as executable data.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedPayloadSyntax {
    /// Program-domain bytes produced from the validated witness.
    bytes: Vec<ProgramByte>,
}

impl<'code> PayloadSyntax<'code> {
    /// Labels compact syntax bytes as the payload domain expected by the parser.
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
    pub(crate) fn validate(self) -> Result<ValidatedPayloadSyntax, ParseError> {
        for byte in self.bytes.iter().copied() {
            ProgramByte::parse(byte, self.line_number, self.payload_kind)?;
        }

        let mut bytes = Vec::new();
        try_reserve_total_exact(
            &mut bytes,
            RequestedCapacity::from_payload_count(self.byte_count()),
            AllocationContext::ProgramPayload,
        )
        .map_err(|error| {
            ParseError::at_line(self.line_number, ParseErrorKind::Allocation(error))
        })?;

        for byte in self.bytes.iter().copied() {
            let parsed = ProgramByte::from_validated_payload_byte(byte).map_err(|error| {
                ParseError::at_line(self.line_number, ParseErrorKind::InternalInvariant(error))
            })?;
            try_push(&mut bytes, parsed, AllocationContext::ProgramPayload).map_err(|error| {
                ParseError::at_line(self.line_number, ParseErrorKind::Allocation(error))
            })?;
        }

        Ok(ValidatedPayloadSyntax { bytes })
    }
}

impl ValidatedPayloadSyntax {
    /// Moves executable payload bytes out of the validated witness.
    pub(crate) fn into_payload(self) -> Payload {
        Payload { bytes: self.bytes }
    }
}

impl Payload {
    /// Returns the typed byte count.
    pub(crate) fn byte_count(&self) -> PayloadByteCount {
        PayloadByteCount::new(self.bytes.len())
    }

    /// Borrows payload bytes in the executable program domain.
    pub(crate) fn program_bytes(&self) -> &[ProgramByte] {
        &self.bytes
    }

    /// Splits the payload into matcher-friendly empty and non-empty forms.
    pub(crate) fn needle(&self) -> PayloadNeedle<'_> {
        match self.bytes.split_first() {
            Some((&first, _)) => PayloadNeedle::NonEmpty(NonEmptyPayloadNeedle {
                payload: self,
                first,
            }),
            None => PayloadNeedle::Empty(EmptyPayloadNeedle { payload: self }),
        }
    }

    /// Materializes payload bytes as caller-visible raw bytes.
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

    /// Converts payload bytes into runtime-state bytes for rewrite output.
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
        try_reserve_total_exact(
            &mut output,
            RequestedCapacity::from_payload_count(self.byte_count()),
            context,
        )?;
        self.push_bytes_to(&mut output, context)?;
        Ok(output)
    }
}

/// Payload shape used by the matcher to avoid unchecked first-byte access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PayloadNeedle<'payload> {
    /// Empty payload that matches at every candidate position.
    Empty(EmptyPayloadNeedle<'payload>),
    /// Non-empty payload with its first byte separated for scanning.
    NonEmpty(NonEmptyPayloadNeedle<'payload>),
}

/// Matcher view for an empty payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EmptyPayloadNeedle<'payload> {
    /// Payload whose length is known to be zero by this view.
    payload: &'payload Payload,
}

/// Matcher view for a non-empty payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NonEmptyPayloadNeedle<'payload> {
    /// Payload whose first byte is available without indexing.
    payload: &'payload Payload,
    /// First program byte used to start candidate matching.
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

    /// First byte used by the matcher to find candidate positions.
    pub(crate) const fn first_byte(self) -> ProgramByte {
        self.first
    }

    /// Full executable payload used after a first-byte candidate is found.
    pub(crate) fn program_bytes(self) -> &'payload [ProgramByte] {
        self.payload.program_bytes()
    }
}
