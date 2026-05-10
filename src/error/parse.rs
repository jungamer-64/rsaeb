use core::error::Error;

use crate::allocation::AllocationError;
use crate::bytes::{NonAsciiCodeByte, NonPrintableCodeByte, ReservedSyntaxByte};
use crate::source::{SourceLineNumber, SourcePosition};

/// Source program parse error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    location: ParseErrorLocation,
    kind: ParseErrorKind,
}

impl ParseError {
    pub(crate) const fn at_line(line: SourceLineNumber, kind: ParseErrorKind) -> Self {
        Self {
            location: ParseErrorLocation::Line(line),
            kind,
        }
    }

    pub(crate) const fn at_position(position: SourcePosition, kind: ParseErrorKind) -> Self {
        Self {
            location: ParseErrorLocation::Position(position),
            kind,
        }
    }

    /// Structured source location for this parse failure.
    #[must_use]
    pub const fn location(&self) -> ParseErrorLocation {
        self.location
    }

    /// One-based source line number.
    #[must_use]
    pub const fn line(&self) -> SourceLineNumber {
        match self.location {
            ParseErrorLocation::Line(line) => line,
            ParseErrorLocation::Position(position) => position.line(),
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

/// Source location carried by a parse error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseErrorLocation {
    /// The whole source line is the relevant location.
    Line(SourceLineNumber),
    /// A specific source byte position is the relevant location.
    Position(SourcePosition),
}

/// Structured parse error reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseErrorKind {
    /// A fallible allocation failed while parsing source.
    Allocation(AllocationError),
    /// A non-ASCII byte appeared before the line comment marker.
    NonAsciiInCode {
        /// Rejected non-ASCII executable-code byte.
        byte: NonAsciiCodeByte,
    },
    /// A non-whitespace ASCII control byte appeared in executable code.
    NonPrintableAsciiInCode {
        /// Rejected non-printable executable-code byte.
        byte: NonPrintableCodeByte,
    },
    /// A non-empty code line did not contain `=`.
    MissingEquals,
    /// A compact code line contained more than one `=`.
    MultipleEquals,
    /// Reserved syntax appeared where program payload data was expected.
    ReservedSyntaxInPayload {
        /// Reserved syntax byte that was rejected as payload data.
        byte: ReservedSyntaxByte,
        /// Payload domain that received the reserved byte.
        payload_kind: PayloadKind,
    },
    /// Left-side modifiers were duplicated or ordered outside the supported grammar.
    UnsupportedLeftModifierOrder {
        /// Modifier that made the left side unsupported.
        modifier: LeftModifierKind,
    },
    /// Right-side actions were nested or otherwise used outside the supported grammar.
    UnsupportedRightActionSyntax {
        /// Action token that made the right side unsupported.
        action: RightActionKind,
    },
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
