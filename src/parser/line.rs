use alloc::vec::Vec;

use crate::allocation::{AllocationContext, AllocationError, try_push, try_reserve_total_exact};
use crate::bytes::{CompactByte, NonAsciiCodeByte, NonPrintableCodeByte};
use crate::error::{ParseError, ParseErrorKind};
use crate::source::{SourceLineNumber, SourcePosition};

use super::location::{parse_allocation_error, source_column};
use super::rule_line::RuleSyntaxLine;

pub(super) struct RawSourceLine<'source> {
    line_number: SourceLineNumber,
    bytes: &'source [u8],
}

impl<'source> RawSourceLine<'source> {
    pub(super) fn new(line_number: SourceLineNumber, bytes: &'source [u8]) -> Self {
        Self { line_number, bytes }
    }

    pub(super) fn into_code_line(self) -> Result<CodeLine<'source>, ParseError> {
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
            let rejected = NonAsciiCodeByte::parse(byte).ok_or_else(|| {
                parse_allocation_error(
                    self.line_number,
                    AllocationError::capacity_overflow(AllocationContext::ProgramCodeLine),
                )
            })?;
            return Err(ParseError::at_position(
                SourcePosition::new(
                    self.line_number,
                    source_column(zero_based_column, self.line_number)?,
                ),
                ParseErrorKind::NonAsciiInCode { byte: rejected },
            ));
        }

        Ok(CodeLine {
            line_number: self.line_number,
            bytes: code_bytes,
        })
    }
}

pub(super) struct CodeLine<'source> {
    line_number: SourceLineNumber,
    bytes: &'source [u8],
}

impl CodeLine<'_> {
    pub(super) fn into_compact_line(self) -> Result<CompactCodeLine, ParseError> {
        let compact_len = self.compact_len()?;
        let bytes = self.compact_bytes(compact_len)?;

        Ok(CompactCodeLine {
            line_number: self.line_number,
            bytes,
        })
    }

    fn compact_len(&self) -> Result<usize, ParseError> {
        let mut compact_len = 0usize;
        for (zero_based_column, byte) in self.bytes.iter().copied().enumerate() {
            if byte.is_ascii_whitespace() {
                continue;
            }

            if let Some(rejected) = NonPrintableCodeByte::parse(byte) {
                return Err(ParseError::at_position(
                    SourcePosition::new(
                        self.line_number,
                        source_column(zero_based_column, self.line_number)?,
                    ),
                    ParseErrorKind::NonPrintableAsciiInCode { byte: rejected },
                ));
            }

            compact_len = compact_len.checked_add(1).ok_or_else(|| {
                parse_allocation_error(
                    self.line_number,
                    AllocationError::capacity_overflow(AllocationContext::ProgramCodeLine),
                )
            })?;
        }

        Ok(compact_len)
    }

    fn compact_bytes(&self, compact_len: usize) -> Result<Vec<CompactByte>, ParseError> {
        let mut bytes = Vec::new();
        try_reserve_total_exact(&mut bytes, compact_len, AllocationContext::ProgramCodeLine)
            .map_err(|error| parse_allocation_error(self.line_number, error))?;

        for (zero_based_column, byte) in self.bytes.iter().copied().enumerate() {
            if byte.is_ascii_whitespace() {
                continue;
            }

            try_push(
                &mut bytes,
                CompactByte::new(byte, source_column(zero_based_column, self.line_number)?),
                AllocationContext::ProgramCodeLine,
            )
            .map_err(|error| parse_allocation_error(self.line_number, error))?;
        }

        Ok(bytes)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct CompactCodeLine {
    line_number: SourceLineNumber,
    bytes: Vec<CompactByte>,
}

impl CompactCodeLine {
    pub(super) fn into_non_empty(self) -> Option<NonEmptyCompactCodeLine> {
        (!self.bytes.is_empty()).then_some(NonEmptyCompactCodeLine {
            line_number: self.line_number,
            bytes: self.bytes,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct NonEmptyCompactCodeLine {
    line_number: SourceLineNumber,
    bytes: Vec<CompactByte>,
}

impl NonEmptyCompactCodeLine {
    pub(super) fn into_rule_syntax(self) -> Result<RuleSyntaxLine, ParseError> {
        let mut left = Vec::new();
        let mut right = Vec::new();
        let mut side = RuleSyntaxSide::Left;

        for byte in self.bytes {
            if byte.as_u8() == b'=' {
                if side == RuleSyntaxSide::Right {
                    return Err(ParseError::at_position(
                        SourcePosition::new(self.line_number, byte.source_column()),
                        ParseErrorKind::MultipleEquals,
                    ));
                }

                side = RuleSyntaxSide::Right;
                continue;
            }

            let target = match side {
                RuleSyntaxSide::Left => &mut left,
                RuleSyntaxSide::Right => &mut right,
            };
            try_push(target, byte, AllocationContext::ProgramCodeLine)
                .map_err(|error| parse_allocation_error(self.line_number, error))?;
        }

        if side == RuleSyntaxSide::Left {
            return Err(ParseError::at_line(
                self.line_number,
                ParseErrorKind::MissingEquals,
            ));
        }

        Ok(RuleSyntaxLine::new(self.line_number, left, right))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuleSyntaxSide {
    Left,
    Right,
}
