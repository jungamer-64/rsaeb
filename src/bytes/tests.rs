use super::*;
use crate::error::{ParseError, ParseErrorKind, PayloadKind};
use crate::source::SourceLineNumber;
use crate::test_support::{
    TestFailure, TestResult, ensure, ensure_eq, ensure_matches, expect_error_position,
    source_column, source_line_number,
};

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
    match Payload::parse(input, line_number, payload_kind) {
        Ok(_) => Err(TestFailure::message("invalid payload bytes were accepted")),
        Err(error) => Ok(error),
    }
}

/// # Errors
///
/// Returns `TestFailure` if reserved syntax bytes are not rejected with
/// structured payload errors.
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

/// # Errors
///
/// Returns `TestFailure` if payload validation accepts invalid executable
/// bytes or reports an unexpected error location.
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

/// # Errors
///
/// Returns `TestFailure` if a parsed payload exposes bytes inconsistent
/// with the validated domain value.
#[test]
fn payload_exposes_validated_bytes_without_leaking_the_internal_domain_type() -> TestResult {
    let compact = [
        CompactByte::new(b'a', source_column(1)?),
        CompactByte::new(b'b', source_column(2)?),
    ];
    let payload = Payload::parse(&compact, source_line_number(1)?, PayloadKind::LeftSideData)
        .map_err(TestFailure::from)?;

    ensure(payload.eq_bytes(b"ab"), "expected payload bytes")?;
    ensure_eq!(
        payload.first_byte().map(program::ProgramByte::get),
        Some(b'a')
    )?;
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if runtime input byte classification or
/// materialization drifts from the ASCII domain split.
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

/// # Errors
///
/// Returns `TestFailure` if runtime input validation depends on reconstructing
/// executable program bytes.
#[test]
fn runtime_input_validation_has_no_reconstruction_invariant() -> TestResult {
    let parsed = RuntimeByte::validate_input(b'a', 0).map_err(TestFailure::from)?;
    ensure_matches(
        matches!(parsed, RuntimeByte::ProgramConstructible(byte) if byte.get() == b'a'),
        "expected ASCII runtime input to validate normally",
    )
}
