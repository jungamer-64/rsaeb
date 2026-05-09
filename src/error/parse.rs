use core::error::Error;

use crate::allocation::AllocationError;
use crate::source::{SourceColumn, SourceLineNumber, SourcePosition};

/// Source program parse error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    line: SourceLineNumber,
    column: Option<SourceColumn>,
    kind: ParseErrorKind,
}

impl ParseError {
    pub(crate) fn new(
        line: SourceLineNumber,
        column: Option<SourceColumn>,
        kind: ParseErrorKind,
    ) -> Self {
        Self { line, column, kind }
    }

    /// One-based source line number.
    #[must_use]
    pub const fn line(&self) -> SourceLineNumber {
        self.line
    }

    /// One-based source column, when the error has a single byte position.
    #[must_use]
    pub const fn column(&self) -> Option<SourceColumn> {
        self.column
    }

    /// One-based source position, when the error has a single byte position.
    #[must_use]
    pub const fn position(&self) -> Option<SourcePosition> {
        match self.column {
            Some(column) => Some(SourcePosition::new(self.line, column)),
            None => None,
        }
    }

    /// Structured parse error reason.
    #[must_use]
    pub const fn kind(&self) -> &ParseErrorKind {
        &self.kind
    }
}

impl Error for ParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.kind {
            ParseErrorKind::Allocation(error) => Some(error),
            ParseErrorKind::NonAsciiInCode { .. }
            | ParseErrorKind::NonPrintableAsciiInCode { .. }
            | ParseErrorKind::MissingEquals
            | ParseErrorKind::MultipleEquals
            | ParseErrorKind::ReservedSyntaxInPayload { .. }
            | ParseErrorKind::UnsupportedLeftModifierOrder { .. }
            | ParseErrorKind::UnsupportedRightActionSyntax { .. } => None,
        }
    }
}

/// Structured parse error reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseErrorKind {
    /// A fallible allocation failed while parsing source.
    Allocation(AllocationError),
    /// A non-ASCII byte appeared before the line comment marker.
    NonAsciiInCode { byte: u8 },
    /// A non-whitespace ASCII control byte appeared in executable code.
    NonPrintableAsciiInCode { byte: u8 },
    /// A non-empty code line did not contain `=`.
    MissingEquals,
    /// A compact code line contained more than one `=`.
    MultipleEquals,
    /// Reserved syntax appeared where program payload data was expected.
    ReservedSyntaxInPayload { byte: u8, payload_kind: PayloadKind },
    /// Left-side modifiers were duplicated or ordered outside the supported grammar.
    UnsupportedLeftModifierOrder { modifier: LeftModifierKind },
    /// Right-side actions were nested or otherwise used outside the supported grammar.
    UnsupportedRightActionSyntax { action: RightActionKind },
}

/// Program payload context used by structured parse errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadKind {
    /// Ordinary left-side match data.
    LeftSideData,
    /// Ordinary right-side replacement data.
    RightSideData,
    /// Right-side payload after `(start)`.
    RightSideMoveStartPayload,
    /// Right-side payload after `(end)`.
    RightSideMoveEndPayload,
    /// Right-side payload after `(return)`.
    RightSideReturnPayload,
}

/// Left-side modifier that caused a structured parse error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeftModifierKind {
    /// `(once)` repeat modifier.
    Once,
    /// `(start)` anchor modifier.
    Start,
    /// `(end)` anchor modifier.
    End,
}

/// Right-side action token that caused a structured parse error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RightActionKind {
    /// `(start)` move-to-start action.
    Start,
    /// `(end)` move-to-end action.
    End,
    /// `(return)` return action.
    Return,
}
