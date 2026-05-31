use alloc::vec::Vec;

use crate::allocation::{AllocationContext, RequestedCapacity, try_push, try_reserve_total_exact};
use crate::bytes::{CompactByte, ExecutableCodeByte};
use crate::error::{ParseError, ParseErrorKind, ParseLimitError};
use crate::limits::{CodeLineByteCount, CodeLineByteLimit};
use crate::source::{SourceLineNumber, SourcePosition};

use super::location::{parse_allocation_error, source_column};
use super::rule_line::RuleSyntaxLine;

/// Internal raw source line.
pub(super) struct RawSourceLine<'source> {
    /// Original one-based source line number.
    line_number: SourceLineNumber,
    /// Raw line bytes before comment splitting.
    bytes: &'source [u8],
    /// Maximum executable bytes allowed before whitespace compaction.
    code_line_limit: CodeLineByteLimit,
}

impl<'source> RawSourceLine<'source> {
    /// Labels one raw source line with its parser budget.
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
        if self.code_line_limit.admit(attempted_len).is_none() {
            return Err(ParseError::at_line(
                self.line_number,
                ParseErrorKind::Limit(ParseLimitError::code_line(
                    self.code_line_limit,
                    attempted_len,
                )),
            ));
        }

        if let Some((zero_based_column, rejected)) = code_bytes
            .iter()
            .copied()
            .enumerate()
            .find_map(|(column, byte)| {
                crate::bytes::NonAsciiCodeByte::parse(byte).map(|rejected| (column, rejected))
            })
        {
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

/// Internal code line.
pub(super) struct CodeLine<'source> {
    /// Original one-based source line number.
    line_number: SourceLineNumber,
    /// Executable bytes before whitespace compaction.
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
        let executable = self.into_executable_line()?;
        let mut bytes = Vec::new();
        try_reserve_total_exact(
            &mut bytes,
            RequestedCapacity::new(executable.bytes.len()),
            AllocationContext::ProgramCodeLine,
        )
        .map_err(|error| parse_allocation_error(executable.line_number, error))?;

        for byte in executable.bytes {
            try_push(
                &mut bytes,
                CompactByte::from_executable(byte),
                AllocationContext::ProgramCodeLine,
            )
            .map_err(|error| parse_allocation_error(executable.line_number, error))?;
        }

        Ok(CompactCodeLine {
            line_number: executable.line_number,
            bytes,
        })
    }

    /// Validates non-whitespace executable bytes before compact syntax construction.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if allocation fails or a non-whitespace executable
    /// byte is non-printable code.
    fn into_executable_line(self) -> Result<ExecutableCodeLine, ParseError> {
        let mut bytes = Vec::new();
        try_reserve_total_exact(
            &mut bytes,
            RequestedCapacity::new(self.bytes.len()),
            AllocationContext::ProgramCodeLine,
        )
        .map_err(|error| parse_allocation_error(self.line_number, error))?;

        for (zero_based_column, byte) in self.bytes.iter().copied().enumerate() {
            if byte.is_ascii_whitespace() {
                continue;
            }

            let position = SourcePosition::new(
                self.line_number,
                source_column(zero_based_column, self.line_number)?,
            );
            let byte = ExecutableCodeByte::validate(byte, position)?;
            try_push(&mut bytes, byte, AllocationContext::ProgramCodeLine)
                .map_err(|error| parse_allocation_error(self.line_number, error))?;
        }

        Ok(ExecutableCodeLine {
            line_number: self.line_number,
            bytes,
        })
    }
}

/// Executable code line after non-whitespace bytes have been validated.
#[derive(Debug, PartialEq, Eq)]
struct ExecutableCodeLine {
    /// Original one-based source line number.
    line_number: SourceLineNumber,
    /// Executable bytes retained after whitespace removal.
    bytes: Vec<ExecutableCodeByte>,
}

/// Internal compact code line.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct CompactCodeLine {
    /// Original one-based source line number.
    line_number: SourceLineNumber,
    /// Compact bytes with source-column witnesses.
    bytes: Vec<CompactByte>,
}

impl CompactCodeLine {
    /// Classifies a compact source line before rule parsing.
    pub(super) fn classify(self) -> CompactCodeLineKind {
        if self.bytes.is_empty() {
            CompactCodeLineKind::Blank
        } else {
            CompactCodeLineKind::Rule(NonEmptyCompactCodeLine {
                line_number: self.line_number,
                bytes: self.bytes,
            })
        }
    }
}

/// Domain classification after code-line compaction.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum CompactCodeLineKind {
    /// Source line has no executable rule bytes.
    Blank,
    /// Source line contains executable rule syntax.
    Rule(NonEmptyCompactCodeLine),
}

/// Internal non empty compact code line.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct NonEmptyCompactCodeLine {
    /// Original one-based source line number.
    line_number: SourceLineNumber,
    /// Compact executable bytes known to contain at least one token.
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
