#![cfg(test)]

use std::string::{FromUtf8Error, String};

use crate::Program;
use crate::error::{
    AebError, AllocationError, ParseError, ParseErrorLocation, RunError, RuntimeInputError,
    TraceSnapshotRunError,
};
use crate::limits::StateByteLimit;
use crate::source::{SourceColumn, SourceLineNumber, SourcePosition};
use crate::{DEFAULT_MAX_STATE_LEN, RuntimeInput, RuntimeInputLimits};

pub(crate) enum TestFailure {
    Message(String),
    Parse(ParseError),
    Input(RuntimeInputError),
    Run(RunError),
    TraceSnapshot(TraceSnapshotRunError),
    Aeb(AebError),
    Utf8(FromUtf8Error),
    Allocation(AllocationError),
}

impl TestFailure {
    pub(crate) fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

impl core::fmt::Debug for TestFailure {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Message(message) => formatter.debug_tuple("Message").field(message).finish(),
            Self::Parse(error) => formatter.debug_tuple("Parse").field(error).finish(),
            Self::Input(error) => formatter.debug_tuple("Input").field(error).finish(),
            Self::Run(error) => formatter.debug_tuple("Run").field(error).finish(),
            Self::TraceSnapshot(error) => {
                formatter.debug_tuple("TraceSnapshot").field(error).finish()
            }
            Self::Aeb(error) => formatter.debug_tuple("Aeb").field(error).finish(),
            Self::Utf8(error) => formatter.debug_tuple("Utf8").field(error).finish(),
            Self::Allocation(error) => formatter.debug_tuple("Allocation").field(error).finish(),
        }
    }
}

impl From<ParseError> for TestFailure {
    fn from(value: ParseError) -> Self {
        Self::Parse(value)
    }
}

impl From<RunError> for TestFailure {
    fn from(value: RunError) -> Self {
        Self::Run(value)
    }
}

impl From<TraceSnapshotRunError> for TestFailure {
    fn from(value: TraceSnapshotRunError) -> Self {
        Self::TraceSnapshot(value)
    }
}

impl From<AebError> for TestFailure {
    fn from(value: AebError) -> Self {
        Self::Aeb(value)
    }
}

impl From<FromUtf8Error> for TestFailure {
    fn from(value: FromUtf8Error) -> Self {
        Self::Utf8(value)
    }
}

impl From<AllocationError> for TestFailure {
    fn from(value: AllocationError) -> Self {
        Self::Allocation(value)
    }
}

impl From<RuntimeInputError> for TestFailure {
    fn from(value: RuntimeInputError) -> Self {
        Self::Input(value)
    }
}

pub(crate) type TestResult = Result<(), TestFailure>;

/// Validates runtime input with the default state byte limit.
///
/// # Errors
///
/// Returns `RuntimeInputError` if the test input violates runtime input
/// validation or allocation constraints.
pub(crate) fn runtime_input(bytes: &[u8]) -> Result<RuntimeInput, RuntimeInputError> {
    runtime_input_with_limit(bytes, DEFAULT_MAX_STATE_LEN)
}

/// Validates runtime input with a test-provided state byte limit.
///
/// # Errors
///
/// Returns `RuntimeInputError` if the test input violates runtime input
/// validation, the provided limit, or allocation constraints.
pub(crate) fn runtime_input_with_limit(
    bytes: &[u8],
    limit: StateByteLimit,
) -> Result<RuntimeInput, RuntimeInputError> {
    RuntimeInput::validate(bytes, RuntimeInputLimits::new(limit))
}

/// Converts a boolean assertion into the shared test result type.
///
/// # Errors
///
/// Returns `TestFailure` with `message` when `condition` is false.
pub(crate) fn ensure(condition: bool, message: impl Into<String>) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(TestFailure::message(message))
    }
}

/// Converts a pattern-match assertion into the shared test result type.
///
/// # Errors
///
/// Returns `TestFailure` with `message` when `condition` is false.
pub(crate) fn ensure_matches(condition: bool, message: &'static str) -> TestResult {
    ensure(condition, message)
}

macro_rules! ensure_eq {
    ($actual:expr, $expected:expr $(,)?) => {{
        match (&$actual, &$expected) {
            (actual, expected) => {
                if *actual == *expected {
                    Ok(())
                } else {
                    Err($crate::test_support::TestFailure::message(::std::format!(
                        "values differed\nactual:   {actual:?}\nexpected: {expected:?}",
                    )))
                }
            }
        }
    }};
}

pub(crate) use ensure_eq;

/// Parses source and returns the expected parse error.
///
/// # Errors
///
/// Returns `TestFailure` if parsing succeeds.
pub(crate) fn expect_parse_error(source: &str) -> Result<ParseError, TestFailure> {
    match Program::parse(crate::ProgramSource::from_str(source)) {
        Ok(_) => Err(TestFailure::message("expected parse error")),
        Err(error) => Ok(error),
    }
}

/// Asserts that a parse error has the expected source position.
///
/// # Errors
///
/// Returns `TestFailure` if the expected position cannot be represented or the
/// parse error location differs.
pub(crate) fn expect_error_position(error: &ParseError, line: usize, column: usize) -> TestResult {
    let line = source_line_number(line)?;
    let column = source_column(column)?;
    ensure_eq!(
        error.location(),
        ParseErrorLocation::Position(SourcePosition::new(line, column)),
    )
}

/// Converts a test literal into a source line number.
///
/// # Errors
///
/// Returns `TestFailure` if `one_based` is zero.
pub(crate) fn source_line_number(one_based: usize) -> Result<SourceLineNumber, TestFailure> {
    SourceLineNumber::from_one_based(one_based)
        .ok_or(TestFailure::message("expected non-zero source line"))
}

/// Converts a test literal into a source column.
///
/// # Errors
///
/// Returns `TestFailure` if `one_based` is zero.
pub(crate) fn source_column(one_based: usize) -> Result<SourceColumn, TestFailure> {
    SourceColumn::from_one_based(one_based)
        .ok_or(TestFailure::message("expected non-zero source column"))
}
