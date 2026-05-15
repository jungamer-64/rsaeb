use std::string::{FromUtf8Error, String};

use rsaeb::error::{
    AebError, AllocationError, FallibleTraceSnapshotRunError, ParseError, RunError,
    RuntimeInputError, TraceSnapshotRunError,
};
use rsaeb::limits::StateByteLimit;
use rsaeb::{DEFAULT_MAX_STATE_LEN, RuntimeInput, RuntimeInputLimits};

pub enum TestFailure {
    Message(String),
    Parse(ParseError),
    Input(RuntimeInputError),
    Run(RunError),
    TraceSnapshot(TraceSnapshotRunError),
    FallibleTraceSnapshot(FallibleTraceSnapshotRunError<&'static str>),
    Aeb(AebError),
    Utf8(FromUtf8Error),
    Allocation(AllocationError),
}

impl TestFailure {
    pub fn message(message: impl Into<String>) -> Self {
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
            Self::FallibleTraceSnapshot(error) => formatter
                .debug_tuple("FallibleTraceSnapshot")
                .field(error)
                .finish(),
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

impl From<RuntimeInputError> for TestFailure {
    fn from(value: RuntimeInputError) -> Self {
        Self::Input(value)
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

impl From<FallibleTraceSnapshotRunError<&'static str>> for TestFailure {
    fn from(value: FallibleTraceSnapshotRunError<&'static str>) -> Self {
        Self::FallibleTraceSnapshot(value)
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

pub type TestResult = Result<(), TestFailure>;

/// Validates runtime input with the default state byte limit.
///
/// # Errors
///
/// Returns `RuntimeInputError` if the test input violates runtime input
/// validation or allocation constraints.
pub fn runtime_input(bytes: &[u8]) -> Result<RuntimeInput, RuntimeInputError> {
    runtime_input_with_limit(bytes, DEFAULT_MAX_STATE_LEN)
}

/// Validates runtime input with a test-provided state byte limit.
///
/// # Errors
///
/// Returns `RuntimeInputError` if the test input violates runtime input
/// validation, the provided limit, or allocation constraints.
pub fn runtime_input_with_limit(
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
pub fn ensure(condition: bool, message: impl Into<String>) -> TestResult {
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
pub fn ensure_matches(condition: bool, message: &'static str) -> TestResult {
    ensure(condition, message)
}

macro_rules! ensure_eq {
    ($actual:expr, $expected:expr $(,)?) => {{
        match (&$actual, &$expected) {
            (actual, expected) => {
                if *actual == *expected {
                    Ok(())
                } else {
                    Err($crate::support::TestFailure::message(::std::format!(
                        "values differed\nactual:   {actual:?}\nexpected: {expected:?}",
                    )))
                }
            }
        }
    }};
}

pub(crate) use ensure_eq;
