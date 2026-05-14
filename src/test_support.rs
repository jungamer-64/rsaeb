#![cfg(test)]

use std::string::{FromUtf8Error, String};
use std::vec::Vec;

use crate::{
    AebError, AllocationError, InputError, LimitError, ParseError, ParseErrorLocation, Program,
    RunError, RunLimits, RunOutcome, RunResult, RuntimeInput, SourceColumn, SourceLineNumber,
    SourcePosition, StepLimit, TraceSnapshotEffect, TraceSnapshotEvent, TraceSnapshotRunError,
};

pub(crate) enum TestFailure {
    Message(String),
    Parse(ParseError),
    Input(InputError),
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

impl From<InputError> for TestFailure {
    fn from(value: InputError) -> Self {
        Self::Input(value)
    }
}

pub(crate) type TestResult = Result<(), TestFailure>;

pub(crate) fn ensure(condition: bool, message: impl Into<String>) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(TestFailure::message(message))
    }
}

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

pub(crate) fn test_limits() -> RunLimits {
    RunLimits::new(StepLimit::new(10_000))
}

pub(crate) fn run_source(source: &str, input: &str) -> Result<String, TestFailure> {
    let program = Program::parse_str(source)?;
    let result = run_program(&program, input.as_bytes(), test_limits())?;
    Ok(String::from_utf8(into_result_bytes(result))?)
}

pub(crate) fn runtime_input(input: &[u8]) -> Result<RuntimeInput, TestFailure> {
    RuntimeInput::parse(input).map_err(TestFailure::from)
}

pub(crate) fn run_program(
    program: &Program,
    input: &[u8],
    limits: RunLimits,
) -> Result<RunResult, TestFailure> {
    let input = runtime_input(input)?;
    program.run(input, limits).map_err(TestFailure::from)
}

pub(crate) fn result_bytes(result: &RunResult) -> &[u8] {
    match result.outcome() {
        RunOutcome::Stable(output) => output.as_bytes(),
        RunOutcome::Return(output) => output.as_bytes(),
    }
}

pub(crate) fn into_result_bytes(result: RunResult) -> Vec<u8> {
    match result.into_outcome() {
        RunOutcome::Stable(output) => output.into_vec(),
        RunOutcome::Return(output) => output.into_vec(),
    }
}

pub(crate) fn expect_stable_output<'result>(
    result: &'result RunResult,
    expected: &[u8],
) -> Result<&'result [u8], TestFailure> {
    match result.outcome() {
        RunOutcome::Stable(output) if output.as_bytes() == expected => Ok(output.as_bytes()),
        RunOutcome::Stable(_) => Err(TestFailure::message("stable output bytes differed")),
        RunOutcome::Return(_) => Err(TestFailure::message("expected stable outcome")),
    }
}

pub(crate) fn expect_return_output<'result>(
    result: &'result RunResult,
    expected: &[u8],
) -> Result<&'result [u8], TestFailure> {
    match result.outcome() {
        RunOutcome::Return(output) if output.as_bytes() == expected => Ok(output.as_bytes()),
        RunOutcome::Return(_) => Err(TestFailure::message("return output bytes differed")),
        RunOutcome::Stable(_) => Err(TestFailure::message("expected return outcome")),
    }
}

pub(crate) fn expect_parse_error(source: &str) -> Result<ParseError, TestFailure> {
    match Program::parse_str(source) {
        Ok(_) => Err(TestFailure::message("expected parse error")),
        Err(error) => Ok(error),
    }
}

pub(crate) fn expect_run_error<T>(result: Result<T, RunError>) -> Result<RunError, TestFailure> {
    match result {
        Ok(_) => Err(TestFailure::message("expected runtime error")),
        Err(error) => Ok(error),
    }
}

pub(crate) fn expect_event<'events, 'program>(
    events: &'events [TraceSnapshotEvent<'program>],
    index: usize,
) -> Result<&'events TraceSnapshotEvent<'program>, TestFailure> {
    events
        .get(index)
        .ok_or(TestFailure::message("expected trace event"))
}

pub(crate) fn expect_error_position(error: &ParseError, line: usize, column: usize) -> TestResult {
    let line = source_line_number(line)?;
    let column = source_column(column)?;
    ensure_eq!(
        error.location(),
        ParseErrorLocation::Position(SourcePosition::new(line, column)),
    )
}

pub(crate) fn trace_event_bytes<'event>(event: &'event TraceSnapshotEvent<'_>) -> &'event [u8] {
    match event {
        TraceSnapshotEvent::Initial { state } => state.as_bytes(),
        TraceSnapshotEvent::Step { effect, .. } => match effect {
            TraceSnapshotEffect::Continue { state } => state.as_bytes(),
            TraceSnapshotEffect::Return { output } => output.as_bytes(),
        },
    }
}

pub(crate) fn source_line_number(one_based: usize) -> Result<SourceLineNumber, TestFailure> {
    SourceLineNumber::from_one_based(one_based)
        .ok_or(TestFailure::message("expected non-zero source line"))
}

pub(crate) fn source_column(one_based: usize) -> Result<SourceColumn, TestFailure> {
    SourceColumn::from_one_based(one_based)
        .ok_or(TestFailure::message("expected non-zero source column"))
}

pub(crate) fn expect_step_limit(error: RunError) -> Result<LimitError, TestFailure> {
    match error {
        RunError::Limit(error @ LimitError::Step { .. }) => Ok(error),
        RunError::Allocation(_)
        | RunError::StateSize(_)
        | RunError::Limit(_)
        | RunError::Invariant(_) => Err(TestFailure::message("expected step limit error")),
    }
}

pub(crate) fn expect_state_limit(error: RunError) -> Result<LimitError, TestFailure> {
    match error {
        RunError::Limit(error @ LimitError::State { .. }) => Ok(error),
        RunError::Allocation(_)
        | RunError::StateSize(_)
        | RunError::Limit(_)
        | RunError::Invariant(_) => Err(TestFailure::message("expected state limit error")),
    }
}

pub(crate) fn expect_input_error<T>(
    result: Result<T, InputError>,
) -> Result<InputError, TestFailure> {
    match result {
        Ok(_) => Err(TestFailure::message("expected input error")),
        Err(error) => Ok(error),
    }
}
