extern crate alloc;

use alloc::format;
use alloc::string::{FromUtf8Error, String};

use rsaeb::error::{
    AllocationError, ParseError, RunAdmissionError, RunError, RuntimeInputError,
    TraceSnapshotRunError,
};
use rsaeb::input::{RunSeed, RuntimeInput, RuntimeInputSource};
use rsaeb::limits::{
    DEFAULT_MAX_INPUT_LEN, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_STEPS,
    DEFAULT_PARSE_LIMITS, ExecutionLimits, ReturnByteLimit, RuntimeInputByteLimit,
    RuntimeInputLimits, RuntimeStateByteLimit, StepLimit,
};
use rsaeb::program::Program;
use rsaeb::source::ProgramSource;

pub enum TestFailure {
    Message(String),
    Parse(ParseError),
    Input(RuntimeInputError),
    Admission(RunAdmissionError),
    Run(RunError),
    TraceSnapshot(String),
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
            Self::Admission(error) => formatter.debug_tuple("Admission").field(error).finish(),
            Self::Run(error) => formatter.debug_tuple("Run").field(error).finish(),
            Self::TraceSnapshot(error) => {
                formatter.debug_tuple("TraceSnapshot").field(error).finish()
            }
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

impl From<RunAdmissionError> for TestFailure {
    fn from(value: RunAdmissionError) -> Self {
        Self::Admission(value)
    }
}

impl From<RunError> for TestFailure {
    fn from(value: RunError) -> Self {
        Self::Run(value)
    }
}

impl<E> From<TraceSnapshotRunError<E>> for TestFailure
where
    E: core::fmt::Debug,
{
    fn from(value: TraceSnapshotRunError<E>) -> Self {
        Self::TraceSnapshot(format!("{value:?}"))
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

// Shared integration-test helper; each public test target imports this module
// but only some targets construct run seeds.
#[allow(
    dead_code,
    reason = "shared integration-test policy type is imported by targets with different coverage"
)]
#[derive(Clone, Copy)]
pub struct TestRunPolicy {
    input: RuntimeInputLimits,
    execution: ExecutionLimits,
}

// Shared integration-test helper; individual test binaries use different
// constructor/accessor subsets.
#[allow(
    dead_code,
    reason = "shared integration-test policy methods are selected per public API target"
)]
impl TestRunPolicy {
    #[must_use]
    pub const fn new(
        max_input_len: RuntimeInputByteLimit,
        max_steps: StepLimit,
        max_state_len: RuntimeStateByteLimit,
        max_return_len: ReturnByteLimit,
    ) -> Self {
        Self {
            input: RuntimeInputLimits::new(max_input_len),
            execution: ExecutionLimits::new(max_steps, max_state_len, max_return_len),
        }
    }

    #[must_use]
    pub const fn default() -> Self {
        Self::new(
            DEFAULT_MAX_INPUT_LEN,
            DEFAULT_MAX_STEPS,
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        )
    }

    #[must_use]
    pub const fn input(self) -> RuntimeInputLimits {
        self.input
    }

    #[must_use]
    pub const fn execution(self) -> ExecutionLimits {
        self.execution
    }
}

/// Validates and admits test input into a run seed.
///
/// # Errors
///
/// Returns `TestFailure` if validation or run admission fails.
// Shared integration-test helper; kept here so public tests do not add
// production-only seed construction APIs.
#[allow(
    dead_code,
    reason = "shared integration-test seed helper is unused in parse-only public API targets"
)]
pub fn run_seed(bytes: &[u8], policy: TestRunPolicy) -> Result<RunSeed, TestFailure> {
    let input = RuntimeInput::validate(RuntimeInputSource::from_bytes(bytes), policy.input())?;
    Ok(RunSeed::admit(input, policy.execution())?)
}

/// Parses source text with the default parser limits.
///
/// # Errors
///
/// Returns `ParseError` if the source violates parser syntax, resource, or
/// allocation constraints.
pub fn parse_program(source: &str) -> Result<Program, ParseError> {
    Program::parse(ProgramSource::from_text(source), DEFAULT_PARSE_LIMITS)
}

/// Converts a pattern-match assertion into the shared test result type.
///
/// # Errors
///
/// Returns `TestFailure` with `message` when `condition` is false.
pub fn ensure_matches(condition: bool, message: &'static str) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(TestFailure::message(message))
    }
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
