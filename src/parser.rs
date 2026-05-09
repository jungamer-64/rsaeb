use alloc::vec::Vec;

use crate::allocation::{AllocationContext, AllocationError, try_push, try_reserve_total_exact};
use crate::bytes::{CompactByte, Payload};
use crate::error::{LeftModifierKind, ParseError, ParseErrorKind, PayloadKind, RightActionKind};
use crate::program::{Program, RuleSet};
use crate::rule::{Action, ParsedRule, RuleAnchor, RuleRepeat};
use crate::source::{SourceColumn, SourceLineNumber};
use crate::syntax::SyntaxToken;

fn parse_allocation_error(line_number: SourceLineNumber, error: AllocationError) -> ParseError {
    ParseError::new(line_number, None, ParseErrorKind::Allocation(error))
}

fn source_line_number(zero_based_line: usize) -> Result<SourceLineNumber, ParseError> {
    SourceLineNumber::from_zero_based(zero_based_line).ok_or_else(|| {
        parse_allocation_error(
            SourceLineNumber::MAX,
            AllocationError::capacity_overflow(AllocationContext::CompactCodeLine),
        )
    })
}

fn source_column(
    zero_based_column: usize,
    line_number: SourceLineNumber,
) -> Result<SourceColumn, ParseError> {
    SourceColumn::from_zero_based(zero_based_column).ok_or_else(|| {
        parse_allocation_error(
            line_number,
            AllocationError::capacity_overflow(AllocationContext::CompactCodeLine),
        )
    })
}

struct RawSourceLine<'source> {
    line_number: SourceLineNumber,
    bytes: &'source [u8],
}

impl<'source> RawSourceLine<'source> {
    fn new(line_number: SourceLineNumber, bytes: &'source [u8]) -> Self {
        Self { line_number, bytes }
    }

    fn into_code_line(self) -> Result<CodeLine<'source>, ParseError> {
        let code_bytes = self
            .bytes
            .split(|&byte| byte == b'#')
            .next()
            .unwrap_or(self.bytes);

        if let Some((zero_based_column, byte)) = code_bytes
            .iter()
            .copied()
            .enumerate()
            .find(|(_, byte)| !byte.is_ascii())
        {
            return Err(ParseError::new(
                self.line_number,
                Some(source_column(zero_based_column, self.line_number)?),
                ParseErrorKind::NonAsciiInCode { byte },
            ));
        }

        Ok(CodeLine {
            line_number: self.line_number,
            bytes: code_bytes,
        })
    }
}

struct CodeLine<'source> {
    line_number: SourceLineNumber,
    bytes: &'source [u8],
}

impl<'source> CodeLine<'source> {
    fn into_compact_line(self) -> Result<CompactCodeLine, ParseError> {
        let mut compact_len = 0usize;

        for (zero_based_column, byte) in self.bytes.iter().copied().enumerate() {
            if byte.is_ascii_whitespace() {
                continue;
            }

            if !byte.is_ascii_graphic() {
                return Err(ParseError::new(
                    self.line_number,
                    Some(source_column(zero_based_column, self.line_number)?),
                    ParseErrorKind::NonPrintableAsciiInCode { byte },
                ));
            }

            compact_len = compact_len.checked_add(1).ok_or_else(|| {
                parse_allocation_error(
                    self.line_number,
                    AllocationError::capacity_overflow(AllocationContext::CompactCodeLine),
                )
            })?;
        }

        let mut bytes = Vec::new();
        try_reserve_total_exact(&mut bytes, compact_len, AllocationContext::CompactCodeLine)
            .map_err(|error| parse_allocation_error(self.line_number, error))?;

        for (zero_based_column, byte) in self.bytes.iter().copied().enumerate() {
            if byte.is_ascii_whitespace() {
                continue;
            }

            try_push(
                &mut bytes,
                CompactByte::new(byte, source_column(zero_based_column, self.line_number)?),
                AllocationContext::CompactCodeLine,
            )
            .map_err(|error| parse_allocation_error(self.line_number, error))?;
        }

        Ok(CompactCodeLine {
            line_number: self.line_number,
            bytes,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
struct CompactCodeLine {
    line_number: SourceLineNumber,
    bytes: Vec<CompactByte>,
}

impl CompactCodeLine {
    fn into_non_empty(self) -> Option<NonEmptyCompactCodeLine> {
        (!self.bytes.is_empty()).then_some(NonEmptyCompactCodeLine {
            line_number: self.line_number,
            bytes: self.bytes,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
struct NonEmptyCompactCodeLine {
    line_number: SourceLineNumber,
    bytes: Vec<CompactByte>,
}

impl NonEmptyCompactCodeLine {
    fn into_rule_syntax(self) -> Result<RuleSyntaxLine, ParseError> {
        let Some(first_equals) = self.bytes.iter().position(|byte| byte.as_u8() == b'=') else {
            return Err(ParseError::new(
                self.line_number,
                None,
                ParseErrorKind::MissingEquals,
            ));
        };

        let equals = EqualsPosition::new(first_equals);

        if let Some(second_equals) = self
            .bytes
            .iter()
            .skip(equals.next_index())
            .find(|byte| byte.as_u8() == b'=')
            .copied()
        {
            return Err(ParseError::new(
                self.line_number,
                Some(second_equals.source_column()),
                ParseErrorKind::MultipleEquals,
            ));
        }

        Ok(RuleSyntaxLine {
            line_number: self.line_number,
            bytes: self.bytes,
            equals,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EqualsPosition {
    index: usize,
}

impl EqualsPosition {
    const fn new(index: usize) -> Self {
        Self { index }
    }

    const fn index(self) -> usize {
        self.index
    }

    const fn next_index(self) -> usize {
        self.index + 1
    }
}

#[derive(Debug, PartialEq, Eq)]
struct RuleSyntaxLine {
    line_number: SourceLineNumber,
    bytes: Vec<CompactByte>,
    equals: EqualsPosition,
}

impl RuleSyntaxLine {
    fn parse(&self) -> Result<ParsedRule, ParseError> {
        let lhs = self.left().parse()?;
        let action = self.right().parse()?;

        Ok(ParsedRule::new(
            self.line_number,
            lhs.repeat,
            lhs.anchor,
            lhs.payload,
            action,
        ))
    }

    fn left(&self) -> LeftSyntax<'_> {
        LeftSyntax {
            line_number: self.line_number,
            bytes: &self.bytes[..self.equals.index()],
        }
    }

    fn right(&self) -> RightSyntax<'_> {
        RightSyntax {
            line_number: self.line_number,
            bytes: &self.bytes[self.equals.next_index()..],
        }
    }
}

struct ParsedLhs {
    repeat: RuleRepeat,
    anchor: RuleAnchor,
    payload: Payload,
}

#[derive(Clone, Copy)]
struct LeftSyntax<'code> {
    line_number: SourceLineNumber,
    bytes: &'code [CompactByte],
}

impl<'code> LeftSyntax<'code> {
    fn parse(self) -> Result<ParsedLhs, ParseError> {
        self.into_after_repeat().parse()
    }

    fn into_after_repeat(self) -> LeftAfterRepeat<'code> {
        if let Some(rest) = strip_token(self.bytes, SyntaxToken::Once) {
            LeftAfterRepeat {
                line_number: self.line_number,
                bytes: rest,
                repeat: RuleRepeat::Once,
            }
        } else {
            LeftAfterRepeat {
                line_number: self.line_number,
                bytes: self.bytes,
                repeat: RuleRepeat::Always,
            }
        }
    }
}

#[derive(Clone, Copy)]
struct LeftAfterRepeat<'code> {
    line_number: SourceLineNumber,
    bytes: &'code [CompactByte],
    repeat: RuleRepeat,
}

impl<'code> LeftAfterRepeat<'code> {
    fn parse(self) -> Result<ParsedLhs, ParseError> {
        self.into_payload_syntax()?.parse()
    }

    fn into_payload_syntax(self) -> Result<LeftPayloadSyntax<'code>, ParseError> {
        let (anchor, bytes) = if let Some(rest) = strip_token(self.bytes, SyntaxToken::Start) {
            (RuleAnchor::Start, rest)
        } else if let Some(rest) = strip_token(self.bytes, SyntaxToken::End) {
            (RuleAnchor::End, rest)
        } else {
            (RuleAnchor::Anywhere, self.bytes)
        };

        if let Some(modifier) = left_modifier_kind(bytes) {
            return Err(ParseError::new(
                self.line_number,
                bytes.first().copied().map(CompactByte::source_column),
                ParseErrorKind::UnsupportedLeftModifierOrder { modifier },
            ));
        }

        Ok(LeftPayloadSyntax {
            line_number: self.line_number,
            bytes,
            repeat: self.repeat,
            anchor,
        })
    }
}

#[derive(Clone, Copy)]
struct LeftPayloadSyntax<'code> {
    line_number: SourceLineNumber,
    bytes: &'code [CompactByte],
    repeat: RuleRepeat,
    anchor: RuleAnchor,
}

impl LeftPayloadSyntax<'_> {
    fn parse(self) -> Result<ParsedLhs, ParseError> {
        let payload = Payload::parse(self.bytes, self.line_number, PayloadKind::LeftSideData)?;
        Ok(ParsedLhs {
            repeat: self.repeat,
            anchor: self.anchor,
            payload,
        })
    }
}

#[derive(Clone, Copy)]
struct RightSyntax<'code> {
    line_number: SourceLineNumber,
    bytes: &'code [CompactByte],
}

impl<'code> RightSyntax<'code> {
    fn parse(self) -> Result<Action, ParseError> {
        self.into_payload_syntax().parse()
    }

    fn into_payload_syntax(self) -> RightPayloadSyntax<'code> {
        if let Some(rest) = strip_token(self.bytes, SyntaxToken::Start) {
            RightPayloadSyntax {
                line_number: self.line_number,
                bytes: rest,
                action: RightActionSyntax::MoveStart,
            }
        } else if let Some(rest) = strip_token(self.bytes, SyntaxToken::End) {
            RightPayloadSyntax {
                line_number: self.line_number,
                bytes: rest,
                action: RightActionSyntax::MoveEnd,
            }
        } else if let Some(rest) = strip_token(self.bytes, SyntaxToken::Return) {
            RightPayloadSyntax {
                line_number: self.line_number,
                bytes: rest,
                action: RightActionSyntax::Return,
            }
        } else {
            RightPayloadSyntax {
                line_number: self.line_number,
                bytes: self.bytes,
                action: RightActionSyntax::Replace,
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RightActionSyntax {
    Replace,
    MoveStart,
    MoveEnd,
    Return,
}

impl RightActionSyntax {
    const fn payload_kind(self) -> PayloadKind {
        match self {
            Self::Replace => PayloadKind::RightSideData,
            Self::MoveStart => PayloadKind::RightSideMoveStartPayload,
            Self::MoveEnd => PayloadKind::RightSideMoveEndPayload,
            Self::Return => PayloadKind::RightSideReturnPayload,
        }
    }

    fn into_action(self, payload: Payload) -> Action {
        match self {
            Self::Replace => Action::Replace(payload),
            Self::MoveStart => Action::MoveStart(payload),
            Self::MoveEnd => Action::MoveEnd(payload),
            Self::Return => Action::Return(payload),
        }
    }
}

#[derive(Clone, Copy)]
struct RightPayloadSyntax<'code> {
    line_number: SourceLineNumber,
    bytes: &'code [CompactByte],
    action: RightActionSyntax,
}

impl RightPayloadSyntax<'_> {
    fn parse(self) -> Result<Action, ParseError> {
        if self.action != RightActionSyntax::Replace {
            reject_nested_rhs_action(self.bytes, self.line_number)?;
        }

        let payload = Payload::parse(self.bytes, self.line_number, self.action.payload_kind())?;
        Ok(self.action.into_action(payload))
    }
}

pub(crate) fn parse_program_impl(source: &[u8]) -> Result<Program, ParseError> {
    let mut rule_set = RuleSet::new();

    for (zero_based_line, raw_line) in source.split(|&byte| byte == b'\n').enumerate() {
        let line_number = source_line_number(zero_based_line)?;
        let compact_code = RawSourceLine::new(line_number, raw_line)
            .into_code_line()?
            .into_compact_line()?;

        let Some(non_empty_code) = compact_code.into_non_empty() else {
            continue;
        };

        let parsed_rule = non_empty_code.into_rule_syntax()?.parse()?;

        rule_set
            .push_parsed_rule(parsed_rule)
            .map_err(|error| parse_allocation_error(line_number, error))?;
    }

    Ok(Program::from_rule_set(rule_set))
}

fn strip_token(input: &[CompactByte], token: SyntaxToken) -> Option<&[CompactByte]> {
    let token_bytes = token.bytes();

    if input.len() < token_bytes.len() {
        return None;
    }

    let starts_with_token = input
        .iter()
        .take(token_bytes.len())
        .copied()
        .map(CompactByte::as_u8)
        .eq(token_bytes.iter().copied());

    if starts_with_token {
        input.get(token_bytes.len()..)
    } else {
        None
    }
}

fn starts_with_token(input: &[CompactByte], token: SyntaxToken) -> bool {
    strip_token(input, token).is_some()
}

fn left_modifier_kind(input: &[CompactByte]) -> Option<LeftModifierKind> {
    if starts_with_token(input, SyntaxToken::Once) {
        Some(LeftModifierKind::Once)
    } else if starts_with_token(input, SyntaxToken::Start) {
        Some(LeftModifierKind::Start)
    } else if starts_with_token(input, SyntaxToken::End) {
        Some(LeftModifierKind::End)
    } else {
        None
    }
}

fn right_action_kind(input: &[CompactByte]) -> Option<RightActionKind> {
    if starts_with_token(input, SyntaxToken::Start) {
        Some(RightActionKind::Start)
    } else if starts_with_token(input, SyntaxToken::End) {
        Some(RightActionKind::End)
    } else if starts_with_token(input, SyntaxToken::Return) {
        Some(RightActionKind::Return)
    } else {
        None
    }
}

fn reject_nested_rhs_action(
    input: &[CompactByte],
    line_number: SourceLineNumber,
) -> Result<(), ParseError> {
    if let Some(action) = right_action_kind(input) {
        return Err(ParseError::new(
            line_number,
            input.first().copied().map(CompactByte::source_column),
            ParseErrorKind::UnsupportedRightActionSyntax { action },
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::test_support::{TestResult, expect_parse_error, run_source};
    use crate::{LeftModifierKind, ParseErrorKind, PayloadKind, Program, RunLimits, StepLimit};

    #[test]
    fn code_spaces_are_ignored_in_rules() -> TestResult {
        assert_eq!(run_source("a b=bb", "abc")?, "bbc");
        assert_eq!(run_source("a = b", "a")?, "b");
        assert_eq!(run_source("( once ) a = ( end ) b", "ca")?, "cb");
        Ok(())
    }

    #[test]
    fn crlf_source_is_accepted_as_code_whitespace() -> TestResult {
        assert_eq!(run_source("a=b\r\nb=c\r\n", "a")?, "c");
        Ok(())
    }

    #[test]
    fn tab_whitespace_is_ignored_in_code() -> TestResult {
        assert_eq!(run_source("a\tb = c\tc", "ab")?, "cc");
        Ok(())
    }

    #[test]
    fn hash_starts_a_comment() -> TestResult {
        assert_eq!(run_source("a=b#c", "a")?, "b");
        assert_eq!(run_source("#a=b", "a")?, "a");
        assert_eq!(run_source("a=b#コメント内の非ASCIIは許可", "a")?, "b");
        Ok(())
    }

    #[test]
    fn empty_compact_lines_do_not_become_rules() -> TestResult {
        let program = Program::parse_str(" \t\r\n# comment\n")?;
        assert_eq!(program.rule_count(), 0);
        Ok(())
    }

    #[test]
    fn comments_may_contain_non_utf8_bytes_because_the_core_parser_is_byte_oriented() -> TestResult
    {
        let source = b"a=b#\xff\xfe\n";
        let program = Program::parse_bytes(source)?;
        let result = program.run(b"a", RunLimits::new(StepLimit::new(10_000)))?;
        assert_eq!(result.output(), b"b");
        Ok(())
    }

    #[test]
    fn code_body_rejects_non_ascii_outside_comments() -> TestResult {
        assert!(Program::parse_str("a=あ").is_err());
        assert!(Program::parse_str("あ=b# comment").is_err());
        assert!(Program::parse_str("a=b#あ").is_ok());

        let error = expect_parse_error("a=あ")?;
        assert_eq!(error.line().get(), 1);
        assert_eq!(error.column().map(crate::SourceColumn::get), Some(3));
        assert!(matches!(
            error.kind(),
            ParseErrorKind::NonAsciiInCode { .. }
        ));
        Ok(())
    }

    #[test]
    fn code_body_rejects_non_printable_ascii_outside_comments() -> TestResult {
        let error = expect_parse_error("a=\0")?;
        assert_eq!(error.line().get(), 1);
        assert_eq!(error.column().map(crate::SourceColumn::get), Some(3));
        assert!(matches!(
            error.kind(),
            ParseErrorKind::NonPrintableAsciiInCode { .. }
        ));

        assert!(Program::parse_str("a=b#\0").is_ok());
        Ok(())
    }

    #[test]
    fn second_equals_is_a_parse_error_unless_it_is_in_a_comment() -> TestResult {
        let error = expect_parse_error("a=b=c")?;
        assert_eq!(error.column().map(crate::SourceColumn::get), Some(4));
        assert!(matches!(error.kind(), ParseErrorKind::MultipleEquals));

        let error = expect_parse_error("a=b =c")?;
        assert_eq!(error.column().map(crate::SourceColumn::get), Some(5));
        assert!(matches!(error.kind(), ParseErrorKind::MultipleEquals));

        assert!(Program::parse_str("a=b#=c").is_ok());
        Ok(())
    }

    #[test]
    fn unsupported_parentheses_are_parse_errors() {
        for source in [
            "a=b(",
            "a=b)",
            "a=b()",
            "a=()",
            "a=b(start)",
            "a=(once)b",
            "a(once)=b",
        ] {
            assert!(
                Program::parse_str(source).is_err(),
                "source should fail: {source}"
            );
        }

        assert!(Program::parse_str("(once)(start)a=(end)b").is_ok());
        assert!(Program::parse_str("a=(return)").is_ok());
    }

    #[test]
    fn comment_before_non_ascii_code_hides_it() {
        assert!(Program::parse_bytes(b"#\xff\xfe\n").is_ok());
        assert!(Program::parse_bytes(b"a=b#\xff\xfe\n").is_ok());
    }

    #[test]
    fn rhs_action_with_empty_payload_is_allowed() -> TestResult {
        assert_eq!(run_source("a=(start)", "ba")?, "b");
        assert_eq!(run_source("a=(end)", "ba")?, "b");
        assert_eq!(run_source("a=(return)", "a")?, "");
        Ok(())
    }

    #[test]
    fn multiline_errors_report_line_and_original_column() -> TestResult {
        let error = expect_parse_error("a=b\nx = y = z")?;

        assert_eq!(error.line().get(), 2);
        assert_eq!(error.column().map(crate::SourceColumn::get), Some(7));
        assert!(matches!(error.kind(), ParseErrorKind::MultipleEquals));
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
            assert!(
                matches!(
                    error.kind(),
                    ParseErrorKind::UnsupportedRightActionSyntax { .. }
                ),
                "source should fail with nested right action syntax: {source}"
            );
        }

        let error = expect_parse_error("a=(start)(return)b")?;
        assert_eq!(error.column().map(crate::SourceColumn::get), Some(10));
        assert!(matches!(
            error.kind(),
            ParseErrorKind::UnsupportedRightActionSyntax {
                action: crate::RightActionKind::Return,
            }
        ));
        Ok(())
    }

    #[test]
    fn reserved_payload_syntax_errors_keep_original_source_column() -> TestResult {
        let error = expect_parse_error("a = b (")?;
        assert_eq!(error.column().map(crate::SourceColumn::get), Some(7));
        assert!(matches!(
            error.kind(),
            ParseErrorKind::ReservedSyntaxInPayload {
                payload_kind: PayloadKind::RightSideData,
                ..
            }
        ));
        Ok(())
    }

    #[test]
    fn invalid_left_modifier_order_is_structured() -> TestResult {
        let error = expect_parse_error("(start)(once)a=b")?;
        assert_eq!(error.column().map(crate::SourceColumn::get), Some(8));
        assert!(matches!(
            error.kind(),
            ParseErrorKind::UnsupportedLeftModifierOrder {
                modifier: LeftModifierKind::Once,
            }
        ));
        Ok(())
    }

    #[test]
    fn compacted_source_and_spaced_source_are_equivalent() -> TestResult {
        let compact = Program::parse_str("(once)(start)a=(end)b")?;
        let spaced = Program::parse_str("( once ) ( start ) a = ( end ) b # comment")?;

        let compact_result = compact.run(b"ac", RunLimits::new(StepLimit::new(10)))?;
        let spaced_result = spaced.run(b"ac", RunLimits::new(StepLimit::new(10)))?;

        assert_eq!(compact_result.output(), spaced_result.output());
        Ok(())
    }
}
