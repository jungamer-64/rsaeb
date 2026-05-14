use crate::test_support::{
    TestResult, ensure, ensure_eq, ensure_matches, expect_error_position, expect_parse_error,
    result_bytes, run_program, run_source, source_line_number,
};
use crate::{
    LeftModifierKind, ParseErrorKind, ParseErrorLocation, PayloadKind, Program, RuleCount,
    RunLimits, StepLimit,
};

#[test]
fn code_spaces_are_ignored_in_rules() -> TestResult {
    ensure_eq!(run_source("a b=bb", "abc")?, "bbc")?;
    ensure_eq!(run_source("a = b", "a")?, "b")?;
    ensure_eq!(run_source("( once ) a = ( end ) b", "ca")?, "cb")?;
    Ok(())
}

#[test]
fn crlf_source_is_accepted_as_code_whitespace() -> TestResult {
    ensure_eq!(run_source("a=b\r\nb=c\r\n", "a")?, "c")?;
    Ok(())
}

#[test]
fn tab_whitespace_is_ignored_in_code() -> TestResult {
    ensure_eq!(run_source("a\tb = c\tc", "ab")?, "cc")?;
    Ok(())
}

#[test]
fn hash_starts_a_comment() -> TestResult {
    ensure_eq!(run_source("a=b#c", "a")?, "b")?;
    ensure_eq!(run_source("#a=b", "a")?, "a")?;
    ensure_eq!(run_source("a=b#コメント内の非ASCIIは許可", "a")?, "b")?;
    Ok(())
}

#[test]
fn empty_compact_lines_do_not_become_rules() -> TestResult {
    let program = Program::parse(crate::ProgramSource::from_str(" \t\r\n# comment\n"))?;
    ensure_eq!(program.rule_count(), RuleCount::new(0))?;
    Ok(())
}

#[test]
fn comments_may_contain_non_utf8_bytes_because_the_core_parser_is_byte_oriented() -> TestResult {
    let source = b"a=b#\xff\xfe\n";
    let program = Program::parse(crate::ProgramSource::from_bytes(source))?;
    let result = run_program(
        &program,
        b"a",
        RunLimits::new(
            StepLimit::new(10_000),
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        ),
    )?;
    ensure_eq!(result_bytes(&result), b"b".as_slice())?;
    Ok(())
}

#[test]
fn code_body_rejects_non_ascii_outside_comments() -> TestResult {
    ensure(
        Program::parse(crate::ProgramSource::from_str("a=あ")).is_err(),
        "expected parse error",
    )?;
    ensure(
        Program::parse(crate::ProgramSource::from_str("あ=b# comment")).is_err(),
        "expected parse error",
    )?;
    ensure(
        Program::parse(crate::ProgramSource::from_str("a=b#あ")).is_ok(),
        "expected comment text to parse",
    )?;

    let error = expect_parse_error("a=あ")?;
    ensure_eq!(error.line().get(), 1)?;
    expect_error_position(&error, 1, 3)?;
    ensure_matches(
        matches!(error.kind(), ParseErrorKind::NonAsciiInCode { .. }),
        "expected non-ASCII parse error",
    )?;
    Ok(())
}

#[test]
fn code_body_rejects_non_printable_ascii_outside_comments() -> TestResult {
    let error = expect_parse_error("a=\0")?;
    ensure_eq!(error.line().get(), 1)?;
    expect_error_position(&error, 1, 3)?;
    ensure_matches(
        matches!(error.kind(), ParseErrorKind::NonPrintableAsciiInCode { .. }),
        "expected non-printable parse error",
    )?;

    ensure(
        Program::parse(crate::ProgramSource::from_str("a=b#\0")).is_ok(),
        "expected comment control byte to parse",
    )?;
    Ok(())
}

#[test]
fn second_equals_is_a_parse_error_unless_it_is_in_a_comment() -> TestResult {
    let error = expect_parse_error("a=b=c")?;
    expect_error_position(&error, 1, 4)?;
    ensure_matches(
        matches!(error.kind(), ParseErrorKind::MultipleEquals),
        "expected multiple equals parse error",
    )?;

    let error = expect_parse_error("a=b =c")?;
    expect_error_position(&error, 1, 5)?;
    ensure_matches(
        matches!(error.kind(), ParseErrorKind::MultipleEquals),
        "expected multiple equals parse error",
    )?;

    ensure(
        Program::parse(crate::ProgramSource::from_str("a=b#=c")).is_ok(),
        "expected equals in comment to parse",
    )?;
    Ok(())
}

#[test]
fn missing_equals_error_uses_line_location() -> TestResult {
    let error = expect_parse_error("abc")?;

    ensure_eq!(
        error.location(),
        ParseErrorLocation::Line(source_line_number(1)?),
    )?;
    ensure_matches(
        matches!(error.kind(), ParseErrorKind::MissingEquals),
        "expected missing equals parse error",
    )?;
    Ok(())
}

#[test]
fn unsupported_parentheses_are_parse_errors() -> TestResult {
    for source in [
        "a=b(",
        "a=b)",
        "a=b()",
        "a=()",
        "a=b(start)",
        "a=(once)b",
        "a(once)=b",
    ] {
        ensure(
            Program::parse(crate::ProgramSource::from_str(source)).is_err(),
            "source should fail",
        )?;
    }

    ensure(
        Program::parse(crate::ProgramSource::from_str("(once)(start)a=(end)b")).is_ok(),
        "expected valid parenthesized modifiers",
    )?;
    ensure(
        Program::parse(crate::ProgramSource::from_str("a=(return)")).is_ok(),
        "expected empty return payload",
    )?;
    Ok(())
}

#[test]
fn comment_before_non_ascii_code_hides_it() -> TestResult {
    ensure(
        Program::parse(crate::ProgramSource::from_bytes(b"#\xff\xfe\n")).is_ok(),
        "expected non-ASCII comment to parse",
    )?;
    ensure(
        Program::parse(crate::ProgramSource::from_bytes(b"a=b#\xff\xfe\n")).is_ok(),
        "expected non-ASCII trailing comment to parse",
    )?;
    Ok(())
}

#[test]
fn rhs_action_with_empty_payload_is_allowed() -> TestResult {
    ensure_eq!(run_source("a=(start)", "ba")?, "b")?;
    ensure_eq!(run_source("a=(end)", "ba")?, "b")?;
    ensure_eq!(run_source("a=(return)", "a")?, "")?;
    Ok(())
}

#[test]
fn multiline_errors_report_line_and_original_column() -> TestResult {
    let error = expect_parse_error("a=b\nx = y = z")?;

    ensure_eq!(error.line().get(), 2)?;
    expect_error_position(&error, 2, 7)?;
    ensure_matches(
        matches!(error.kind(), ParseErrorKind::MultipleEquals),
        "expected multiple equals parse error",
    )?;
    Ok(())
}

#[test]
fn right_side_action_payload_cannot_start_with_another_action() -> TestResult {
    for source in [
        "a=(start)(end)b",
        "a=(start)(return)b",
        "a=(end)(start)b",
        "a=(return)(start)b",
    ] {
        let error = expect_parse_error(source)?;
        ensure_matches(
            matches!(
                error.kind(),
                ParseErrorKind::UnsupportedRightActionSyntax { .. }
            ),
            "expected nested right action syntax error",
        )?;
    }

    let error = expect_parse_error("a=(start)(return)b")?;
    expect_error_position(&error, 1, 10)?;
    ensure_matches(
        matches!(
            error.kind(),
            ParseErrorKind::UnsupportedRightActionSyntax {
                action: crate::RightActionKind::Return,
            }
        ),
        "expected return action syntax error",
    )?;
    Ok(())
}

#[test]
fn reserved_payload_syntax_errors_keep_original_source_column() -> TestResult {
    let error = expect_parse_error("a = b (")?;
    expect_error_position(&error, 1, 7)?;
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
    Ok(())
}

#[test]
fn invalid_left_modifier_order_is_structured() -> TestResult {
    let error = expect_parse_error("(start)(once)a=b")?;
    expect_error_position(&error, 1, 8)?;
    ensure_matches(
        matches!(
            error.kind(),
            ParseErrorKind::UnsupportedLeftModifierOrder {
                modifier: LeftModifierKind::Once,
            }
        ),
        "expected left modifier order error",
    )?;
    Ok(())
}

#[test]
fn compacted_source_and_spaced_source_are_equivalent() -> TestResult {
    let compact = Program::parse(crate::ProgramSource::from_str("(once)(start)a=(end)b"))?;
    let spaced = Program::parse(crate::ProgramSource::from_str(
        "( once ) ( start ) a = ( end ) b # comment",
    ))?;

    let compact_result = run_program(
        &compact,
        b"ac",
        RunLimits::new(
            StepLimit::new(10),
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        ),
    )?;
    let spaced_result = run_program(
        &spaced,
        b"ac",
        RunLimits::new(
            StepLimit::new(10),
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        ),
    )?;

    ensure_eq!(result_bytes(&compact_result), result_bytes(&spaced_result))?;
    Ok(())
}
