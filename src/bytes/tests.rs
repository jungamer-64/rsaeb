use alloc::vec::Vec;

use super::*;
use crate::error::{ParseError, ParseErrorKind, PayloadKind};
use crate::limits::DEFAULT_MAX_PAYLOAD_LEN;
use crate::source::{SourceLineNumber, SourcePosition};
use crate::test_support::{
    TestFailure, TestResult, ensure_eq, ensure_matches, expect_error_position, source_column,
    source_line_number,
};

/// Returns one classified runtime byte after crossing the validation boundary.
///
/// # Errors
///
/// Returns `TestFailure` if the byte is rejected by runtime input validation.
fn validated_runtime_byte(byte: u8) -> Result<RuntimeByte, TestFailure> {
    RuntimeInputByte::validate(byte, 0)
        .map(RuntimeInputByte::into_runtime_byte)
        .map_err(TestFailure::from)
}

/// Validates one executable source byte and compacts it for payload parsing.
///
/// # Errors
///
/// Returns `TestFailure` if the byte is not executable code or its source
/// position cannot be represented.
fn compact_byte(byte: u8, line: usize, column: usize) -> Result<CompactByte, TestFailure> {
    let position = SourcePosition::new(source_line_number(line)?, source_column(column)?);
    let executable = ExecutableCodeByte::validate(byte, position)?;
    Ok(CompactByte::from_executable(executable))
}

/// Parses payload bytes and returns the expected parse error.
///
/// # Errors
///
/// Returns `TestFailure` if the invalid payload is accepted.
fn parse_payload_error(
    input: &[CompactByte],
    line_number: SourceLineNumber,
    payload_kind: PayloadKind,
) -> Result<ParseError, TestFailure> {
    match PayloadSyntax::check(input, line_number, payload_kind, DEFAULT_MAX_PAYLOAD_LEN)?
        .validate()
    {
        Ok(_) => Err(TestFailure::message("invalid payload bytes were accepted")),
        Err(error) => Ok(error),
    }
}

/// Parses payload bytes through the validated payload syntax boundary.
///
/// # Errors
///
/// Returns `TestFailure` if the bytes are not valid payload syntax.
fn parse_payload(
    input: &[CompactByte],
    line_number: SourceLineNumber,
    payload_kind: PayloadKind,
) -> Result<Payload, TestFailure> {
    Ok(
        PayloadSyntax::check(input, line_number, payload_kind, DEFAULT_MAX_PAYLOAD_LEN)?
            .validate()?,
    )
}

/// # Errors
///
/// Returns `TestFailure` if reserved syntax bytes are not rejected with
/// structured payload errors.
#[test]
fn payload_rejects_every_reserved_syntax_byte_even_if_payload_parser_is_called_directly()
-> TestResult {
    for reserved in [b'=', b'#', b'(', b')'] {
        let compact = [compact_byte(reserved, 1, 1)?];
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

/// # Errors
///
/// Returns `TestFailure` if executable code-byte validation accepts invalid
/// executable bytes or reports an unexpected error location.
#[test]
fn executable_code_byte_validation_precedes_payload_parsing() -> TestResult {
    let position = SourcePosition::new(source_line_number(1)?, source_column(1)?);
    let Err(error) = ExecutableCodeByte::validate(0xff, position) else {
        return Err(TestFailure::message(
            "non-ASCII executable byte should be rejected",
        ));
    };
    ensure_matches(
        matches!(error.kind(), ParseErrorKind::NonAsciiInCode { .. }),
        "expected non-ASCII parse error",
    )?;

    let position = SourcePosition::new(source_line_number(1)?, source_column(2)?);
    let Err(error) = ExecutableCodeByte::validate(b' ', position) else {
        return Err(TestFailure::message(
            "non-printable executable byte should be rejected",
        ));
    };
    expect_error_position(&error, 1, 2)?;
    ensure_matches(
        matches!(error.kind(), ParseErrorKind::NonPrintableAsciiInCode { .. }),
        "expected non-printable parse error",
    )?;
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if a parsed payload exposes bytes inconsistent
/// with the validated domain value.
#[test]
fn payload_exposes_validated_bytes_without_leaking_the_internal_domain_type() -> TestResult {
    let compact = [compact_byte(b'a', 1, 1)?, compact_byte(b'b', 1, 2)?];
    let payload = parse_payload(&compact, source_line_number(1)?, PayloadKind::LeftSideData)?;

    ensure_eq!(payload.bytes().collect::<Vec<_>>(), b"ab".to_vec())?;
    let PayloadNeedle::NonEmpty(needle) = payload.needle() else {
        return Err(TestFailure::message("expected non-empty payload needle"));
    };
    ensure_eq!(needle.first_byte().get(), b'a')?;
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if runtime input byte classification or
/// materialization drifts from the ASCII domain split.
#[test]
fn runtime_input_classifies_program_constructible_and_opaque_ascii_separately() -> TestResult {
    let parsed = validated_runtime_byte(b'a')?;
    ensure_matches(
        matches!(parsed, RuntimeByte::ProgramConstructible(byte) if byte.get() == b'a'),
        "expected program-constructible input byte",
    )?;
    ensure_eq!(parsed.materialize(), b'a')?;

    for byte in [0x00, b' ', b'=', b'#', b'(', b')'] {
        let parsed = validated_runtime_byte(byte)?;
        ensure_eq!(parsed.materialize(), byte)?;
        ensure_matches(
            matches!(parsed, RuntimeByte::Opaque(_)),
            "expected opaque input byte",
        )?;
    }

    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if runtime input validation depends on reconstructing
/// executable program bytes.
#[test]
fn runtime_input_validation_has_no_reconstruction_invariant() -> TestResult {
    let parsed = validated_runtime_byte(b'a')?;
    ensure_matches(
        matches!(parsed, RuntimeByte::ProgramConstructible(byte) if byte.get() == b'a'),
        "expected ASCII runtime input to validate normally",
    )
}
