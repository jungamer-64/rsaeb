use alloc::vec::Vec;
use core::error::Error;
use core::fmt;

use crate::allocation::AllocationError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AebError {
    /// Source program parse error.
    Parse(ParseError),
    /// Runtime execution error.
    Run(RunError),
}

impl fmt::Display for AebError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(error) => error.fmt(f),
            Self::Run(error) => error.fmt(f),
        }
    }
}

impl Error for AebError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Parse(error) => Some(error),
            Self::Run(error) => Some(error),
        }
    }
}

impl From<ParseError> for AebError {
    fn from(value: ParseError) -> Self {
        Self::Parse(value)
    }
}

impl From<RunError> for AebError {
    fn from(value: RunError) -> Self {
        Self::Run(value)
    }
}

/// Source program parse error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    line: usize,
    column: Option<usize>,
    kind: ParseErrorKind,
}

impl ParseError {
    pub(crate) fn new(line: usize, column: Option<usize>, kind: ParseErrorKind) -> Self {
        Self { line, column, kind }
    }

    /// One-based source line number.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }

    /// One-based source column, when the error has a single byte position.
    #[must_use]
    pub const fn column(&self) -> Option<usize> {
        self.column
    }

    /// Structured parse error reason.
    #[must_use]
    pub const fn kind(&self) -> &ParseErrorKind {
        &self.kind
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "parse error at line {}", self.line)?;

        if let Some(column) = self.column {
            write!(f, ", column {column}")?;
        }

        write!(f, ": {}", self.kind)
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
            | ParseErrorKind::UnsupportedLeftModifierOrder
            | ParseErrorKind::UnsupportedRightActionSyntax => None,
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
    UnsupportedLeftModifierOrder,
    /// Right-side actions were nested or otherwise used outside the supported grammar.
    UnsupportedRightActionSyntax,
}

impl fmt::Display for ParseErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Allocation(error) => error.fmt(f),
            Self::NonAsciiInCode { byte } => write!(f, "non-ASCII byte 0x{byte:02x} in code"),
            Self::NonPrintableAsciiInCode { byte } => {
                write!(f, "non-printable ASCII byte 0x{byte:02x} in code")
            }
            Self::MissingEquals => write!(f, "missing '='"),
            Self::MultipleEquals => write!(f, "multiple '=' characters are not allowed"),
            Self::ReservedSyntaxInPayload { byte, payload_kind } => write!(
                f,
                "reserved syntax byte '{}' in {payload_kind}",
                printable_ascii(*byte),
            ),
            Self::UnsupportedLeftModifierOrder => {
                write!(f, "duplicated or unsupported left-side modifier order")
            }
            Self::UnsupportedRightActionSyntax => {
                write!(f, "nested or unsupported right-side action syntax")
            }
        }
    }
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

impl fmt::Display for PayloadKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LeftSideData => write!(f, "left-side data"),
            Self::RightSideData => write!(f, "right-side data"),
            Self::RightSideMoveStartPayload => write!(f, "right-side move-to-start payload"),
            Self::RightSideMoveEndPayload => write!(f, "right-side move-to-end payload"),
            Self::RightSideReturnPayload => write!(f, "right-side return payload"),
        }
    }
}

/// Runtime execution error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    /// Runtime input is invalid.
    Input(InputError),
    /// A fallible allocation failed during runtime execution.
    Allocation(AllocationError),
    /// A rewrite length could not be represented.
    StateSize(StateSizeError),
    /// Execution exceeded the configured step limit.
    StepLimit(StepLimitError),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(error) => error.fmt(f),
            Self::Allocation(error) => error.fmt(f),
            Self::StateSize(error) => error.fmt(f),
            Self::StepLimit(error) => error.fmt(f),
        }
    }
}

impl Error for RunError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Input(error) => Some(error),
            Self::Allocation(error) => Some(error),
            Self::StateSize(error) => Some(error),
            Self::StepLimit(error) => Some(error),
        }
    }
}

impl From<InputError> for RunError {
    fn from(value: InputError) -> Self {
        Self::Input(value)
    }
}

impl From<AllocationError> for RunError {
    fn from(value: AllocationError) -> Self {
        Self::Allocation(value)
    }
}

impl From<StateSizeError> for RunError {
    fn from(value: StateSizeError) -> Self {
        Self::StateSize(value)
    }
}

impl From<StepLimitError> for RunError {
    fn from(value: StepLimitError) -> Self {
        Self::StepLimit(value)
    }
}

/// Error returned by fallible tracing APIs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TracedRunError<E> {
    /// Parser/runtime execution failed.
    Run(RunError),
    /// The user-provided trace sink failed.
    Trace(E),
}

impl<E> fmt::Display for TracedRunError<E>
where
    E: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Run(error) => error.fmt(f),
            Self::Trace(error) => write!(f, "trace callback failed: {error}"),
        }
    }
}

impl<E> Error for TracedRunError<E>
where
    E: Error + 'static,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Run(error) => Some(error),
            Self::Trace(error) => Some(error),
        }
    }
}

impl<E> From<RunError> for TracedRunError<E> {
    fn from(value: RunError) -> Self {
        Self::Run(value)
    }
}

/// Runtime input validation error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputError {
    column: usize,
    byte: u8,
}

impl InputError {
    pub(crate) const fn new(column: usize, byte: u8) -> Self {
        Self { column, byte }
    }

    /// One-based input column.
    #[must_use]
    pub const fn column(&self) -> usize {
        self.column
    }

    /// Rejected byte.
    #[must_use]
    pub const fn byte(&self) -> u8 {
        self.byte
    }
}

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "input error: non-ASCII byte 0x{:02x} at column {}",
            self.byte, self.column,
        )
    }
}

impl Error for InputError {}

/// Runtime state-size failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateSizeError {
    state: usize,
    lhs: usize,
    rhs: usize,
}

impl StateSizeError {
    pub(crate) const fn new(state_len: usize, lhs_len: usize, rhs_len: usize) -> Self {
        Self {
            state: state_len,
            lhs: lhs_len,
            rhs: rhs_len,
        }
    }

    /// Runtime state length before the failing rewrite.
    #[must_use]
    pub const fn state_len(&self) -> usize {
        self.state
    }

    /// Matched left-side length that would be removed.
    #[must_use]
    pub const fn lhs_len(&self) -> usize {
        self.lhs
    }

    /// Right-side payload length that would be inserted.
    #[must_use]
    pub const fn rhs_len(&self) -> usize {
        self.rhs
    }
}

impl fmt::Display for StateSizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "state size failure: replacing {} bytes in a {} byte state with {} bytes",
            self.lhs, self.state, self.rhs,
        )
    }
}

impl Error for StateSizeError {}

/// Step-limit failure with the last runtime state preserved as bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepLimitError {
    max_steps: usize,
    state: Vec<u8>,
}

impl StepLimitError {
    pub(crate) fn new(max_steps: usize, state: Vec<u8>) -> Self {
        Self { max_steps, state }
    }

    /// Configured maximum step count.
    #[must_use]
    pub const fn max_steps(&self) -> usize {
        self.max_steps
    }

    /// Runtime state when the limit was hit.
    #[must_use]
    pub fn state(&self) -> &[u8] {
        &self.state
    }

    /// Consumes the error and returns the runtime state.
    #[must_use]
    pub fn into_state(self) -> Vec<u8> {
        self.state
    }
}

impl fmt::Display for StepLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "step limit exceeded after {} steps; state length: {} bytes",
            self.max_steps,
            self.state.len(),
        )
    }
}

impl Error for StepLimitError {}

fn printable_ascii(byte: u8) -> char {
    if byte.is_ascii() {
        byte as char
    } else {
        '\u{fffd}'
    }
}
