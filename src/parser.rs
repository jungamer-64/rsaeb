use alloc::vec::Vec;

use crate::allocation::{AllocationContext, AllocationError, try_push, try_reserve_total_exact};
use crate::bytes::{CompactByte, Payload};
use crate::error::{ParseError, ParseErrorKind, PayloadKind};
use crate::program::Program;
use crate::rule::{Action, Rule, RuleAnchor, RuleRepeat};

const TOK_ONCE: &[u8] = b"(once)";
const TOK_START: &[u8] = b"(start)";
const TOK_END: &[u8] = b"(end)";
const TOK_RETURN: &[u8] = b"(return)";

fn parse_allocation_error(line_number: usize, error: AllocationError) -> ParseError {
    ParseError::new(line_number, None, ParseErrorKind::Allocation(error))
}

struct CodeLine<'source> {
    line_number: usize,
    bytes: &'source [u8],
}

impl<'source> CodeLine<'source> {
    fn parse(raw_line: &'source [u8], line_number: usize) -> Result<Self, ParseError> {
        let code_bytes = match raw_line.iter().position(|&byte| byte == b'#') {
            Some(comment_start) => &raw_line[..comment_start],
            None => raw_line,
        };

        if let Some((zero_based_column, byte)) = code_bytes
            .iter()
            .copied()
            .enumerate()
            .find(|(_, byte)| !byte.is_ascii())
        {
            return Err(ParseError::new(
                line_number,
                Some(zero_based_column + 1),
                ParseErrorKind::NonAsciiInCode { byte },
            ));
        }

        Ok(Self {
            line_number,
            bytes: code_bytes,
        })
    }

    fn compact(self) -> Result<CompactCodeLine, ParseError> {
        let compact_len = self
            .bytes
            .iter()
            .filter(|byte| !byte.is_ascii_whitespace())
            .count();
        let mut bytes = Vec::new();
        try_reserve_total_exact(&mut bytes, compact_len, AllocationContext::CompactCodeLine)
            .map_err(|error| parse_allocation_error(self.line_number, error))?;

        for (zero_based_column, byte) in self.bytes.iter().copied().enumerate() {
            if byte.is_ascii_whitespace() {
                continue;
            }

            if !byte.is_ascii_graphic() {
                return Err(ParseError::new(
                    self.line_number,
                    Some(zero_based_column + 1),
                    ParseErrorKind::NonPrintableAsciiInCode { byte },
                ));
            }

            bytes.push(CompactByte::new(byte, zero_based_column + 1));
        }

        Ok(CompactCodeLine {
            line_number: self.line_number,
            bytes,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompactCodeLine {
    line_number: usize,
    bytes: Vec<CompactByte>,
}

impl CompactCodeLine {
    fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    fn compact_source(&self) -> Result<Vec<u8>, AllocationError> {
        let mut source = Vec::new();
        try_reserve_total_exact(
            &mut source,
            self.bytes.len(),
            AllocationContext::CompactSource,
        )?;

        for byte in self.bytes.iter().copied() {
            source.push(byte.as_u8());
        }

        Ok(source)
    }

    fn equals_position(&self) -> Result<usize, ParseError> {
        let Some(first_equals) = self.bytes.iter().position(|byte| byte.as_u8() == b'=') else {
            return Err(ParseError::new(
                self.line_number,
                None,
                ParseErrorKind::MissingEquals,
            ));
        };

        if let Some(second_equals) = self
            .bytes
            .iter()
            .skip(first_equals + 1)
            .find(|byte| byte.as_u8() == b'=')
            .copied()
        {
            return Err(ParseError::new(
                self.line_number,
                Some(second_equals.source_column()),
                ParseErrorKind::MultipleEquals,
            ));
        }

        Ok(first_equals)
    }

    fn split_at_equals(
        &self,
        equals_position: usize,
    ) -> Result<(&[CompactByte], &[CompactByte]), ParseError> {
        let (lhs, rhs_with_equals) = self.bytes.split_at(equals_position);

        let Some(rhs) = rhs_with_equals.get(1..) else {
            return Err(ParseError::new(
                self.line_number,
                None,
                ParseErrorKind::MissingEquals,
            ));
        };

        Ok((lhs, rhs))
    }
}

pub(crate) fn parse_program_impl(source: &[u8]) -> Result<Program, ParseError> {
    let mut rules = Vec::new();
    let rule_upper_bound = source.split(|&byte| byte == b'\n').count();
    try_reserve_total_exact(
        &mut rules,
        rule_upper_bound,
        AllocationContext::ProgramRules,
    )
    .map_err(|error| parse_allocation_error(1, error))?;

    for (zero_based_line, raw_line) in source.split(|&byte| byte == b'\n').enumerate() {
        let line_number = zero_based_line + 1;
        let compact_code = CodeLine::parse(raw_line, line_number)?.compact()?;

        if compact_code.is_empty() {
            continue;
        }

        let equals_position = compact_code.equals_position()?;
        let compact_source = compact_code
            .compact_source()
            .map_err(|error| parse_allocation_error(line_number, error))?;
        let (lhs_code, rhs_code) = compact_code.split_at_equals(equals_position)?;
        let (repeat, anchor, lhs) = parse_lhs(lhs_code, line_number)?;
        let action = parse_rhs(rhs_code, line_number)?;

        try_push(
            &mut rules,
            Rule {
                line_number,
                compact_source,
                repeat,
                anchor,
                lhs,
                action,
            },
            AllocationContext::ProgramRules,
        )
        .map_err(|error| parse_allocation_error(line_number, error))?;
    }

    Ok(Program { rules })
}

fn strip_token<'code>(input: &'code [CompactByte], token: &[u8]) -> Option<&'code [CompactByte]> {
    if input.len() < token.len() {
        return None;
    }

    let starts_with_token = input
        .iter()
        .take(token.len())
        .copied()
        .map(CompactByte::as_u8)
        .eq(token.iter().copied());

    if starts_with_token {
        input.get(token.len()..)
    } else {
        None
    }
}

fn starts_with_token(input: &[CompactByte], token: &[u8]) -> bool {
    strip_token(input, token).is_some()
}

fn parse_lhs(
    mut input: &[CompactByte],
    line_number: usize,
) -> Result<(RuleRepeat, RuleAnchor, Payload), ParseError> {
    let mut repeat = RuleRepeat::Always;

    if let Some(rest) = strip_token(input, TOK_ONCE) {
        repeat = RuleRepeat::Once;
        input = rest;
    }

    let anchor = if let Some(rest) = strip_token(input, TOK_START) {
        input = rest;
        RuleAnchor::Start
    } else if let Some(rest) = strip_token(input, TOK_END) {
        input = rest;
        RuleAnchor::End
    } else {
        RuleAnchor::Anywhere
    };

    if starts_with_token(input, TOK_ONCE)
        || starts_with_token(input, TOK_START)
        || starts_with_token(input, TOK_END)
    {
        return Err(ParseError::new(
            line_number,
            input.first().copied().map(CompactByte::source_column),
            ParseErrorKind::UnsupportedLeftModifierOrder,
        ));
    }

    let lhs = Payload::parse(input, line_number, PayloadKind::LeftSideData)?;
    Ok((repeat, anchor, lhs))
}

fn parse_rhs(input: &[CompactByte], line_number: usize) -> Result<Action, ParseError> {
    if let Some(rest) = strip_token(input, TOK_START) {
        reject_nested_rhs_action(rest, line_number)?;
        let payload = Payload::parse(rest, line_number, PayloadKind::RightSideMoveStartPayload)?;
        Ok(Action::MoveStart(payload))
    } else if let Some(rest) = strip_token(input, TOK_END) {
        reject_nested_rhs_action(rest, line_number)?;
        let payload = Payload::parse(rest, line_number, PayloadKind::RightSideMoveEndPayload)?;
        Ok(Action::MoveEnd(payload))
    } else if let Some(rest) = strip_token(input, TOK_RETURN) {
        reject_nested_rhs_action(rest, line_number)?;
        let payload = Payload::parse(rest, line_number, PayloadKind::RightSideReturnPayload)?;
        Ok(Action::Return(payload))
    } else {
        let payload = Payload::parse(input, line_number, PayloadKind::RightSideData)?;
        Ok(Action::Replace(payload))
    }
}

fn reject_nested_rhs_action(input: &[CompactByte], line_number: usize) -> Result<(), ParseError> {
    if starts_with_token(input, TOK_START)
        || starts_with_token(input, TOK_END)
        || starts_with_token(input, TOK_RETURN)
    {
        return Err(ParseError::new(
            line_number,
            input.first().copied().map(CompactByte::source_column),
            ParseErrorKind::UnsupportedRightActionSyntax,
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::test_support::{TestResult, expect_parse_error, run_source};
    use crate::{ParseErrorKind, PayloadKind, Program, RunOptions};
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
    fn comments_may_contain_non_utf8_bytes_because_the_core_parser_is_byte_oriented() -> TestResult
    {
        let source = b"a=b#\xff\xfe\n";
        let program = Program::parse(source)?;
        let result = program.run(b"a", RunOptions::new(10_000))?;
        assert_eq!(result.output(), b"b");
        Ok(())
    }

    #[test]
    fn code_body_rejects_non_ascii_outside_comments() -> TestResult {
        assert!(Program::parse("a=あ").is_err());
        assert!(Program::parse("あ=b# comment").is_err());
        assert!(Program::parse("a=b#あ").is_ok());

        let error = expect_parse_error("a=あ")?;
        assert_eq!(error.line(), 1);
        assert_eq!(error.column(), Some(3));
        assert!(matches!(
            error.kind(),
            ParseErrorKind::NonAsciiInCode { .. }
        ));
        Ok(())
    }

    #[test]
    fn code_body_rejects_non_printable_ascii_outside_comments() -> TestResult {
        let error = expect_parse_error("a=\0")?;
        assert_eq!(error.line(), 1);
        assert_eq!(error.column(), Some(3));
        assert!(matches!(
            error.kind(),
            ParseErrorKind::NonPrintableAsciiInCode { .. }
        ));

        assert!(Program::parse("a=b#\0").is_ok());
        Ok(())
    }

    #[test]
    fn second_equals_is_a_parse_error_unless_it_is_in_a_comment() -> TestResult {
        let error = expect_parse_error("a=b=c")?;
        assert_eq!(error.column(), Some(4));
        assert!(matches!(error.kind(), ParseErrorKind::MultipleEquals));

        let error = expect_parse_error("a=b =c")?;
        assert_eq!(error.column(), Some(5));
        assert!(matches!(error.kind(), ParseErrorKind::MultipleEquals));

        assert!(Program::parse("a=b#=c").is_ok());
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
                Program::parse(source).is_err(),
                "source should fail: {source}"
            );
        }

        assert!(Program::parse("(once)(start)a=(end)b").is_ok());
        assert!(Program::parse("a=(return)").is_ok());
    }

    #[test]
    fn comment_before_non_ascii_code_hides_it() {
        assert!(Program::parse(b"#\xff\xfe\n").is_ok());
        assert!(Program::parse(b"a=b#\xff\xfe\n").is_ok());
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

        assert_eq!(error.line(), 2);
        assert_eq!(error.column(), Some(7));
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
                matches!(error.kind(), ParseErrorKind::UnsupportedRightActionSyntax),
                "source should fail with nested right action syntax: {source}"
            );
        }
        Ok(())
    }

    #[test]
    fn reserved_payload_syntax_errors_keep_original_source_column() -> TestResult {
        let error = expect_parse_error("a = b (")?;
        assert_eq!(error.column(), Some(7));
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
        assert!(matches!(
            error.kind(),
            ParseErrorKind::UnsupportedLeftModifierOrder
        ));
        Ok(())
    }

    #[test]
    fn compacted_source_and_spaced_source_are_equivalent() -> TestResult {
        let compact = Program::parse("(once)(start)a=(end)b")?;
        let spaced = Program::parse("( once ) ( start ) a = ( end ) b # comment")?;

        let compact_result = compact.run(b"ac", RunOptions::new(10))?;
        let spaced_result = spaced.run(b"ac", RunOptions::new(10))?;

        assert_eq!(compact_result.output(), spaced_result.output());
        Ok(())
    }
}
