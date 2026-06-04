extern crate alloc;

use alloc::format;
use alloc::string::{FromUtf8Error, String};

use rsaeb::error::{
    AllocationError, EmptyProgramParseError, ExecutableProgramParseError, ParseError,
    RuleAttemptStepError, RunAdmissionError, RunError, RunFinishError, RunStartError, RunStepError,
    RuntimeInputError, TraceSnapshotRunError,
};
use rsaeb::policy::DefaultParsePolicy;
use rsaeb::program::ExecutableProgram;

pub enum TestFailure {
    Message(String),
    Parse(ParseError),
    ExecutableParse(ExecutableProgramParseError),
    EmptyParse(EmptyProgramParseError),
    Input(RuntimeInputError),
    Admission(RunAdmissionError),
    Run(RunError),
    RunStart(RunStartError),
    RunFinish(RunFinishError),
    RunStep(RunStepError),
    RuleAttemptStep(RuleAttemptStepError),
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
            Self::ExecutableParse(error) => formatter
                .debug_tuple("ExecutableParse")
                .field(error)
                .finish(),
            Self::EmptyParse(error) => formatter.debug_tuple("EmptyParse").field(error).finish(),
            Self::Input(error) => formatter.debug_tuple("Input").field(error).finish(),
            Self::Admission(error) => formatter.debug_tuple("Admission").field(error).finish(),
            Self::Run(error) => formatter.debug_tuple("Run").field(error).finish(),
            Self::RunStart(error) => formatter.debug_tuple("RunStart").field(error).finish(),
            Self::RunFinish(error) => formatter.debug_tuple("RunFinish").field(error).finish(),
            Self::RunStep(error) => formatter.debug_tuple("RunStep").field(error).finish(),
            Self::RuleAttemptStep(error) => formatter
                .debug_tuple("RuleAttemptStep")
                .field(error)
                .finish(),
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

impl From<ExecutableProgramParseError> for TestFailure {
    fn from(value: ExecutableProgramParseError) -> Self {
        Self::ExecutableParse(value)
    }
}

impl From<EmptyProgramParseError> for TestFailure {
    fn from(value: EmptyProgramParseError) -> Self {
        Self::EmptyParse(value)
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

impl From<RunStartError> for TestFailure {
    fn from(value: RunStartError) -> Self {
        Self::RunStart(value)
    }
}

impl From<RunFinishError> for TestFailure {
    fn from(value: RunFinishError) -> Self {
        Self::RunFinish(value)
    }
}

impl From<RunStepError> for TestFailure {
    fn from(value: RunStepError) -> Self {
        Self::RunStep(value)
    }
}

impl From<RuleAttemptStepError> for TestFailure {
    fn from(value: RuleAttemptStepError) -> Self {
        Self::RuleAttemptStep(value)
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

/// Parses executable source text with the default parser limits.
///
/// # Errors
///
/// Returns `ExecutableProgramParseError` if the source violates parser syntax,
/// resource constraints, allocation constraints, or contains no executable
/// rules.
pub fn parse_program(source: &str) -> Result<ExecutableProgram, ExecutableProgramParseError> {
    ExecutableProgram::parse_text::<DefaultParsePolicy>(source)
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
