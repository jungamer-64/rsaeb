#![cfg(test)]
#![expect(
    dead_code,
    reason = "shared unit-test policy helpers are compiled per test target"
)]

use alloc::string::{FromUtf8Error, String};

use crate::error::{
    AllocationError, OwnedRunStepError, ParseError, ParseErrorLocation, RuleAttemptStepError,
    RunAdmissionError, RunError, RunFinishError, RunStartError, RunStepError, RuntimeInputError,
    TraceSnapshotRunError,
};
use crate::input::{AdmittedRun, RuntimeInput, RuntimeInputSource};
use crate::policy::{
    DefaultExecutionPolicy, DefaultParsePolicy, DefaultRuntimeInputPolicy, ExecutionPolicy,
    ParsePolicy, RuntimeInputPolicy, StaticExecutionPolicy, StaticRuntimeInputPolicy,
};
use crate::program::{BorrowedExecutableProgram, OwnedExecutableProgram, Program};
use crate::source::{ProgramSource, SourceColumn, SourceLineNumber, SourcePosition};
use core::marker::PhantomData;

pub(crate) const DEFAULT_BYTE_BUDGET: usize = 16_777_216;
pub(crate) const DEFAULT_COUNT_BUDGET: usize = 1_000_000;
pub(crate) type TestInputPolicy<const INPUT_BYTES: usize> = StaticRuntimeInputPolicy<INPUT_BYTES>;
pub(crate) type TestExecutionPolicy<
    const STEPS: usize,
    const STATE_BYTES: usize,
    const RETURN_BYTES: usize,
> = StaticExecutionPolicy<STEPS, STATE_BYTES, RETURN_BYTES>;
pub(crate) type StaticTestRunPolicy<
    const INPUT_BYTES: usize,
    const STEPS: usize,
    const STATE_BYTES: usize,
    const RETURN_BYTES: usize,
> = TestRunPolicy<
    TestInputPolicy<INPUT_BYTES>,
    TestExecutionPolicy<STEPS, STATE_BYTES, RETURN_BYTES>,
>;
pub(crate) type DefaultInputRunPolicy<
    const STEPS: usize,
    const STATE_BYTES: usize,
    const RETURN_BYTES: usize,
> = TestRunPolicy<DefaultRuntimeInputPolicy, TestExecutionPolicy<STEPS, STATE_BYTES, RETURN_BYTES>>;
pub(crate) type DefaultExecutionRunPolicy<const INPUT_BYTES: usize> =
    TestRunPolicy<TestInputPolicy<INPUT_BYTES>, DefaultExecutionPolicy>;
pub(crate) type DefaultRunPolicy = TestRunPolicy<DefaultRuntimeInputPolicy, DefaultExecutionPolicy>;

pub(crate) enum TestFailure {
    Message(String),
    Parse(ParseError),
    Input(RuntimeInputError),
    Admission(RunAdmissionError),
    Run(RunError),
    RunStart(RunStartError),
    RunFinish(RunFinishError),
    RunStep(RunStepError),
    OwnedRunStep(OwnedRunStepError),
    RuleAttemptStep(RuleAttemptStepError),
    TraceSnapshot(String),
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
            Self::Admission(error) => formatter.debug_tuple("Admission").field(error).finish(),
            Self::Run(error) => formatter.debug_tuple("Run").field(error).finish(),
            Self::RunStart(error) => formatter.debug_tuple("RunStart").field(error).finish(),
            Self::RunFinish(error) => formatter.debug_tuple("RunFinish").field(error).finish(),
            Self::RunStep(error) => formatter.debug_tuple("RunStep").field(error).finish(),
            Self::OwnedRunStep(error) => {
                formatter.debug_tuple("OwnedRunStep").field(error).finish()
            }
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

impl From<OwnedRunStepError> for TestFailure {
    fn from(value: OwnedRunStepError) -> Self {
        Self::OwnedRunStep(value)
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
        Self::TraceSnapshot(std::format!("{value:?}"))
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

impl From<RunAdmissionError> for TestFailure {
    fn from(value: RunAdmissionError) -> Self {
        Self::Admission(value)
    }
}

pub(crate) type TestResult = Result<(), TestFailure>;

pub(crate) struct TestRunPolicy<
    I: RuntimeInputPolicy = DefaultRuntimeInputPolicy,
    E: ExecutionPolicy = DefaultExecutionPolicy,
> {
    policy: PhantomData<(I, E)>,
}

impl<I: RuntimeInputPolicy, E: ExecutionPolicy> Clone for TestRunPolicy<I, E> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<I: RuntimeInputPolicy, E: ExecutionPolicy> Copy for TestRunPolicy<I, E> {}

impl<I: RuntimeInputPolicy, E: ExecutionPolicy> TestRunPolicy<I, E> {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self {
            policy: PhantomData,
        }
    }

    #[must_use]
    pub(crate) const fn default() -> Self {
        Self::new()
    }
}

/// Validates runtime input with the supplied input limits.
///
/// # Errors
///
/// Returns `RuntimeInputError` if the test input violates validation, allocation,
/// or initial runtime-state admission constraints.
pub(crate) fn runtime_input<I: RuntimeInputPolicy, E: ExecutionPolicy>(
    bytes: &[u8],
    _policy: TestRunPolicy<I, E>,
) -> Result<RuntimeInput<I>, RuntimeInputError> {
    RuntimeInput::<I>::validate(RuntimeInputSource::from_bytes(bytes))
}

/// Validates and admits test input into an execution witness.
///
/// # Errors
///
/// Returns `TestFailure` if validation or run admission fails.
pub(crate) fn admitted_run<I: RuntimeInputPolicy, E: ExecutionPolicy>(
    bytes: &[u8],
    policy: TestRunPolicy<I, E>,
) -> Result<AdmittedRun<E>, TestFailure> {
    Ok(runtime_input(bytes, policy)?.admit::<E>()?)
}

/// Parses source text with the default parser limits.
///
/// # Errors
///
/// Returns `ParseError` if the source violates parser syntax, resource, or
/// allocation constraints.
pub(crate) fn parse_program(source: &str) -> Result<Program<DefaultParsePolicy>, ParseError> {
    Program::parse(ProgramSource::from_text(source))
}

/// Borrows the expected executable parsed program.
///
/// # Errors
///
/// Returns `TestFailure` if the parsed program has no executable rules.
pub(crate) fn executable_program<P: ParsePolicy>(
    program: &Program<P>,
) -> Result<BorrowedExecutableProgram<'_, P>, TestFailure> {
    program
        .as_executable()
        .map_err(|_| TestFailure::message("expected executable program"))
}

/// Moves the expected executable parsed program.
///
/// # Errors
///
/// Returns `TestFailure` if the parsed program has no executable rules.
pub(crate) fn owned_executable_program<P: ParsePolicy>(
    program: Program<P>,
) -> Result<OwnedExecutableProgram<P>, TestFailure> {
    program
        .into_executable()
        .map_err(|_| TestFailure::message("expected executable program"))
}

/// Parses source bytes with the default parser limits.
///
/// # Errors
///
/// Returns `ParseError` if the source violates parser syntax, resource, or
/// allocation constraints.
pub(crate) fn parse_program_bytes(
    source: &[u8],
) -> Result<Program<DefaultParsePolicy>, ParseError> {
    Program::parse(ProgramSource::from_bytes(source))
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
    match parse_program(source) {
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
    let zero_based = one_based
        .checked_sub(1)
        .ok_or(TestFailure::message("expected non-zero source line"))?;
    SourceLineNumber::from_zero_based(zero_based)
        .ok_or(TestFailure::message("expected representable source line"))
}

/// Converts a test literal into a source column.
///
/// # Errors
///
/// Returns `TestFailure` if `one_based` is zero.
pub(crate) fn source_column(one_based: usize) -> Result<SourceColumn, TestFailure> {
    let zero_based = one_based
        .checked_sub(1)
        .ok_or(TestFailure::message("expected non-zero source column"))?;
    SourceColumn::from_zero_based(zero_based)
        .ok_or(TestFailure::message("expected representable source column"))
}
