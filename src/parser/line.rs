use alloc::vec::Vec;

use crate::allocation::{AllocationContext, AllocationError, try_push, try_reserve_total_exact};
use crate::bytes::{CompactByte, NonAsciiCodeByte, NonPrintableCodeByte};
use crate::error::{ParseError, ParseErrorKind, ParseLimitError};
use crate::program::{CodeLineByteCount, CodeLineByteLimit};
use crate::source::{SourceLineNumber, SourcePosition};

use super::location::{parse_allocation_error, source_column};
use super::rule_line::RuleSyntaxLine;

pub(super) struct RawSourceLine<'source> {
    line_number: SourceLineNumber,
    bytes: &'source [u8],
    code_line_limit: CodeLineByteLimit,
}

impl<'source> RawSourceLine<'source> {
    pub(super) fn new(
        line_number: SourceLineNumber,
        bytes: &'source [u8],
        code_line_limit: CodeLineByteLimit,
    ) -> Self {
        Self {
            line_number,
            bytes,
            code_line_limit,
        }
    }

    /// Splits comments away and validates raw executable code bytes.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` when executable code contains non-ASCII bytes or
    /// source-column arithmetic overflows.
    pub(super) fn into_code_line(self) -> Result<CodeLine<'source>, ParseError> {
        let code_bytes = self
            .bytes
            .split(|&byte| byte == b'#')
            .next()
            .unwrap_or(self.bytes);

        let attempted_len = CodeLineByteCount::new(code_bytes.len());
        if attempted_len.get() > self.code_line_limit.get() {
            return Err(ParseError::at_line(
                self.line_number,
                ParseErrorKind::Limit(ParseLimitError::code_line(
                    self.code_line_limit,
                    attempted_len,
                )),
            ));
        }

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
    /// Removes insignificant source whitespace into compact syntax bytes.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` when compacting sees non-printable executable code
    /// bytes, allocation fails, or source positions cannot be represented.
    pub(super) fn into_compact_line(self) -> Result<CompactCodeLine, ParseError> {
        let compact_len = self.compact_len()?;
        let bytes = self.compact_bytes(compact_len)?;

        Ok(CompactCodeLine {
            line_number: self.line_number,
            bytes,
        })
    }

    /// Counts compact executable bytes after whitespace removal.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` when a non-printable byte is found or the compact
    /// length cannot be represented.
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

    /// Builds compact source bytes with original source columns attached.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if allocation fails or a source column cannot be
    /// represented.
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
    /// Converts a non-empty compact line into parsed rule syntax.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the compact line cannot be split into a valid
    /// rule syntax boundary.
    pub(super) fn into_rule_syntax(self) -> Result<RuleSyntaxLine, ParseError> {
        RuleSyntaxLine::new(self.line_number, self.bytes)
    }
}
