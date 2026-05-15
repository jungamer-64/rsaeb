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
