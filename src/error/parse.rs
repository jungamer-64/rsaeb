use core::error::Error;

use crate::allocation::AllocationError;
use crate::bytes::{NonAsciiCodeByte, NonPrintableCodeByte, PayloadByteCount, ReservedSyntaxByte};
use crate::inspect::RuleCount;
use crate::limits::{
    CodeLineByteCount, CodeLineByteLimit, PayloadByteLimit, RuleLimit, SourceByteCount,
    SourceByteLimit,
};
use crate::source::{SourceLineNumber, SourcePosition};

/// Source program parse error.
///
/// Parse errors always carry a typed source location and a structured reason.
/// The parser accepts arbitrary bytes after a line-comment marker, but
/// executable code before the comment marker must fit the supported A=B syntax.
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
            ParseErrorKind::InternalInvariant(error) => Some(error),
            ParseErrorKind::Limit(error) => Some(error),
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
///
/// Some failures apply to the whole executable line, while byte-specific
/// failures point at a one-based source position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseErrorLocation {
    /// The whole source line is the relevant location.
    Line(SourceLineNumber),
    /// A specific source byte position is the relevant location.
    Position(SourcePosition),
}

/// Structured parse error reason.
///
/// These variants describe parser-domain failures only. Runtime input and
/// runtime execution errors are reported through separate types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseErrorKind {
    /// A fallible allocation failed while parsing source.
    Allocation(AllocationError),
    /// Parser invariants were violated while deriving typed syntax ranges.
    InternalInvariant(ParseInvariantError),
    /// A configured parser resource limit would be exceeded.
    Limit(ParseLimitError),
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

/// Parser invariant violation that should be unrepresentable from accepted
/// source bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseInvariantError {
    /// A previously validated rule-side range no longer resolved inside its
    /// compact source line.
    InvalidRuleSideRange,
}

impl ParseInvariantError {
    pub(crate) const fn invalid_rule_side_range() -> Self {
        Self::InvalidRuleSideRange
    }
}

impl Error for ParseInvariantError {}

/// Configured parser budget failure.
///
/// Parser limits are checked before accepting source, code-line, payload, or
/// rule table growth beyond the host-provided policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseLimitError {
    /// Source bytes would exceed the configured source limit.
    Source {
        /// Configured maximum source length.
        limit: SourceByteLimit,
        /// Source length that was passed to the parser.
        attempted_len: SourceByteCount,
    },
    /// One executable source line would exceed the configured code-line limit.
    CodeLine {
        /// Configured maximum code-line length.
        limit: CodeLineByteLimit,
        /// Code-line length after comment removal and before whitespace compaction.
        attempted_len: CodeLineByteCount,
    },
    /// One parsed payload would exceed the configured payload limit.
    Payload {
        /// Configured maximum payload length.
        limit: PayloadByteLimit,
        /// Payload length after syntax-token removal and whitespace compaction.
        attempted_len: PayloadByteCount,
    },
    /// Parsed executable rules would exceed the configured rule limit.
    Rules {
        /// Configured maximum executable rule count.
        limit: RuleLimit,
        /// Rule count that would be reached by accepting the rejected rule.
        attempted_count: RuleCount,
    },
}

impl ParseLimitError {
    pub(crate) const fn source(limit: SourceByteLimit, attempted_len: SourceByteCount) -> Self {
        Self::Source {
            limit,
            attempted_len,
        }
    }

    pub(crate) const fn code_line(
        limit: CodeLineByteLimit,
        attempted_len: CodeLineByteCount,
    ) -> Self {
        Self::CodeLine {
            limit,
            attempted_len,
        }
    }

    pub(crate) const fn payload(limit: PayloadByteLimit, attempted_len: PayloadByteCount) -> Self {
        Self::Payload {
            limit,
            attempted_len,
        }
    }

    pub(crate) const fn rules(limit: RuleLimit, attempted_count: RuleCount) -> Self {
        Self::Rules {
            limit,
            attempted_count,
        }
    }
}

impl Error for ParseLimitError {}

/// Program payload context used by structured parse errors.
///
/// Payload context identifies which side/action was being parsed when reserved
/// syntax appeared as data.
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
