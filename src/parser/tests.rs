use crate::error::{
    LeftModifierKind, ParseErrorKind, ParseErrorLocation, PayloadKind, RightActionKind,
};
use crate::inspect::{RuleActionView, RuleAnchor, RuleCount, RuleRepeat};
use crate::program::Program;
use crate::test_support::{
    TestFailure, TestResult, ensure, ensure_eq, ensure_matches, expect_error_position,
    expect_parse_error, parse_program, parse_program_bytes, source_line_number,
};

/// Returns the parsed rule at `index`.
///
/// # Errors
///
/// Returns `TestFailure` if the program has no rule at `index`.
fn expect_rule(
    program: &Program,
    index: usize,
) -> Result<crate::inspect::RuleView<'_>, TestFailure> {
    program
        .rules()
        .nth(index)
        .ok_or(TestFailure::message("expected parsed rule"))
}

/// # Errors
///
/// Returns `TestFailure` if compacted source does not preserve the expected
/// typed rule domain.
#[test]
fn compacting_source_whitespace_and_comments_preserves_rule_domain() -> TestResult {
    let program = parse_program(
        "a b=bb\n\
         a = b # trailing comment\n\
         ( once ) ( start ) x = ( end ) y",
    )?;

    ensure_eq!(program.rule_count(), RuleCount::new(3))?;
    ensure_eq!(
        expect_rule(&program, 0)?.canonical_source()?.as_slice(),
        b"ab=bb".as_slice(),
    )?;
    ensure_eq!(
        expect_rule(&program, 1)?.canonical_source()?.as_slice(),
        b"a=b".as_slice(),
    )?;
    ensure_eq!(
        expect_rule(&program, 2)?.canonical_source()?.as_slice(),
        b"(once)(start)x=(end)y".as_slice(),
    )?;
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if empty code lines or comments become parsed rules.
#[test]
fn empty_code_lines_and_comments_do_not_become_rules() -> TestResult {
    let program = parse_program(" \t\r\n# comment\n")?;
    ensure_eq!(program.rule_count(), RuleCount::new(0))
}

/// # Errors
///
/// Returns `TestFailure` if comment bytes affect executable parsing.
#[test]
fn comments_may_contain_non_utf8_bytes_because_source_is_byte_oriented() -> TestResult {
    let program = parse_program_bytes(b"a=b#\xff\xfe\n")?;
    let rule = expect_rule(&program, 0)?;

    ensure_eq!(program.rule_count(), RuleCount::new(1))?;
    ensure_eq!(rule.canonical_source()?.as_slice(), b"a=b".as_slice())
}

/// # Errors
///
/// Returns `TestFailure` if invalid executable code bytes are accepted or
/// reported at the wrong location.
#[test]
fn code_body_rejects_non_ascii_and_non_printable_bytes_outside_comments() -> TestResult {
    let error = expect_parse_error("a=\u{80}")?;
    ensure_eq!(error.line().get(), 1)?;
    expect_error_position(&error, 1, 3)?;
    ensure_matches(
        matches!(error.kind(), ParseErrorKind::NonAsciiInCode { .. }),
        "expected non-ASCII parse error",
    )?;

    let error = expect_parse_error("a=\0")?;
    ensure_eq!(error.line().get(), 1)?;
    expect_error_position(&error, 1, 3)?;
    ensure_matches(
        matches!(error.kind(), ParseErrorKind::NonPrintableAsciiInCode { .. }),
        "expected non-printable parse error",
    )?;

    ensure(
        parse_program_bytes(b"a=b#\xff").is_ok(),
        "expected comment bytes to parse",
    )
}

/// # Errors
///
/// Returns `TestFailure` if equals-separator errors lose their original source
/// locations.
#[test]
fn equals_and_missing_equals_errors_keep_original_source_locations() -> TestResult {
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

    let error = expect_parse_error("abc")?;
    ensure_eq!(
        error.location(),
        ParseErrorLocation::Line(source_line_number(1)?),
    )?;
    ensure_matches(
        matches!(error.kind(), ParseErrorKind::MissingEquals),
        "expected missing equals parse error",
    )
}

/// # Errors
///
/// Returns `TestFailure` if reserved parentheses are accepted outside their
/// supported modifier and action slots.
#[test]
fn reserved_parentheses_are_rejected_outside_supported_modifier_slots() -> TestResult {
    for source in [
        "a=b(",
        "a=b)",
        "a=b()",
        "a=()",
        "a=b(start)",
        "a=(once)b",
        "a(once)=b",
    ] {
        ensure(parse_program(source).is_err(), "source should fail")?;
    }

    ensure(
        parse_program("(once)(start)a=(end)b").is_ok(),
        "expected valid parenthesized modifiers",
    )?;
    ensure(
        parse_program("a=(return)").is_ok(),
        "expected empty return payload",
    )
}

/// # Errors
///
/// Returns `TestFailure` if nested right-side actions are accepted or reported
/// with the wrong structured action kind.
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
                action: RightActionKind::Return,
            }
        ),
        "expected return action syntax error",
    )
}

/// # Errors
///
/// Returns `TestFailure` if payload or left-modifier parse errors lose their
/// structured kind.
#[test]
fn payload_and_left_modifier_errors_are_structured() -> TestResult {
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
    )
}

/// # Errors
///
/// Returns `TestFailure` if spaced and compact source parse to different typed
/// rule views.
#[test]
fn spaced_source_and_compact_source_parse_to_the_same_rule_view() -> TestResult {
    let compact = parse_program("(once)(start)a=(end)b")?;
    let spaced = parse_program("( once ) ( start ) a = ( end ) b # comment")?;
    let compact_rule = expect_rule(&compact, 0)?;
    let spaced_rule = expect_rule(&spaced, 0)?;

    ensure_eq!(compact.rule_count(), RuleCount::new(1))?;
    ensure_eq!(spaced.rule_count(), RuleCount::new(1))?;
    ensure_eq!(spaced_rule.repeat(), RuleRepeat::Once)?;
    ensure_eq!(spaced_rule.anchor(), RuleAnchor::Start)?;
    ensure_eq!(spaced_rule.lhs().materialize()?.as_slice(), b"a".as_slice())?;
    match spaced_rule.action() {
        RuleActionView::MoveEnd(payload) => {
            ensure_eq!(payload.materialize()?.as_slice(), b"b".as_slice())?;
        }
        RuleActionView::Replace(_) | RuleActionView::MoveStart(_) | RuleActionView::Return(_) => {
            return Err(TestFailure::message("expected move-end action"));
        }
    }
    ensure_eq!(
        compact_rule.canonical_source()?.as_slice(),
        spaced_rule.canonical_source()?.as_slice(),
    )
}
