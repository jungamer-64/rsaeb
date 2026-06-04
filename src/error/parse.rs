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
    /// Source location where parsing failed.
    location: ParseErrorLocation,
    /// Structured parse failure reason.
    kind: ParseErrorKind,
}

impl ParseError {
    /// Builds the at line value.
    pub(crate) const fn at_line(line: SourceLineNumber, kind: ParseErrorKind) -> Self {
        Self {
            location: ParseErrorLocation::Line(line),
            kind,
        }
    }

    /// Builds a match span at a candidate position.
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
            ParseErrorKind::Representation(error) => Some(error),
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

/// Error while parsing source that must contain executable rules.
///
/// This error keeps syntax/resource failures separate from shape mismatches:
/// source may be syntactically valid A=B input and still be rejected because the
/// caller asked for an executable program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutableProgramParseError {
    /// Source failed the ordinary parser contract.
    Parse(ParseError),
    /// Source parsed successfully but contained no executable rules.
    NoExecutableRules,
}

impl Error for ExecutableProgramParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Parse(error) => Some(error),
            Self::NoExecutableRules => None,
        }
    }
}

impl From<ParseError> for ExecutableProgramParseError {
    fn from(value: ParseError) -> Self {
        Self::Parse(value)
    }
}

/// Error while parsing source that must contain no executable rules.
///
/// This error keeps syntax/resource failures separate from shape mismatches.
/// Empty-target parsing rejects the first fully parsed executable rule without
/// parsing later source lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmptyProgramParseError {
    /// Source failed the ordinary parser contract.
    Parse(ParseError),
    /// Source contained an executable rule where an empty program was required.
    ExecutableRule {
        /// Source line of the first rejected executable rule.
        line_number: SourceLineNumber,
    },
}

impl Error for EmptyProgramParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Parse(error) => Some(error),
            Self::ExecutableRule { .. } => None,
        }
    }
}

impl From<ParseError> for EmptyProgramParseError {
    fn from(value: ParseError) -> Self {
        Self::Parse(value)
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
    /// A parser representation limit unrelated to allocation was exceeded.
    Representation(ParseRepresentationError),
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

/// Parser-domain representation failure.
///
/// These errors describe source metadata that cannot be represented. They are
/// separate from allocation failures so capacity errors do not hide
/// non-allocation representation problems.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseRepresentationError {
    /// A one-based source line number could not be represented.
    SourceLineNumber,
    /// A one-based source column could not be represented.
    SourceColumn {
        /// Source line where column conversion failed.
        line: SourceLineNumber,
    },
}

impl Error for ParseRepresentationError {}

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
    /// Builds the source value.
    pub(crate) const fn source(limit: SourceByteLimit, attempted_len: SourceByteCount) -> Self {
        Self::Source {
            limit,
            attempted_len,
        }
    }

    /// Builds the code line value.
    pub(crate) const fn code_line(
        limit: CodeLineByteLimit,
        attempted_len: CodeLineByteCount,
    ) -> Self {
        Self::CodeLine {
            limit,
            attempted_len,
        }
    }

    /// Builds the payload value.
    pub(crate) const fn payload(limit: PayloadByteLimit, attempted_len: PayloadByteCount) -> Self {
        Self::Payload {
            limit,
            attempted_len,
        }
    }

    /// Builds the rules value.
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
