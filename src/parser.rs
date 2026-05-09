use alloc::vec::Vec;

use crate::allocation::{try_push, try_reserve_total_exact, AllocationContext, AllocationError};
use crate::bytes::{CompactByte, Payload};
use crate::error::{ParseError, ParseErrorKind, PayloadKind};
use crate::program::Program;
use crate::rule::{Action, Anchor, Rule, RuleRepeat};

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
        try_reserve_total_exact(&mut source, self.bytes.len(), AllocationContext::CompactSource)?;

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
) -> Result<(RuleRepeat, Anchor, Payload), ParseError> {
    let mut repeat = RuleRepeat::Always;

    if let Some(rest) = strip_token(input, TOK_ONCE) {
        repeat = RuleRepeat::Once;
        input = rest;
    }

    let anchor = if let Some(rest) = strip_token(input, TOK_START) {
        input = rest;
        Anchor::Start
    } else if let Some(rest) = strip_token(input, TOK_END) {
        input = rest;
        Anchor::End
    } else {
        Anchor::Anywhere
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
        let payload = Payload::parse(rest, line_number, PayloadKind::RightSideMoveStartPayload)?;
        Ok(Action::MoveStart(payload))
    } else if let Some(rest) = strip_token(input, TOK_END) {
        let payload = Payload::parse(rest, line_number, PayloadKind::RightSideMoveEndPayload)?;
        Ok(Action::MoveEnd(payload))
    } else if let Some(rest) = strip_token(input, TOK_RETURN) {
        let payload = Payload::parse(rest, line_number, PayloadKind::RightSideReturnPayload)?;
        Ok(Action::Return(payload))
    } else {
        let payload = Payload::parse(input, line_number, PayloadKind::RightSideData)?;
        Ok(Action::Replace(payload))
    }
}
