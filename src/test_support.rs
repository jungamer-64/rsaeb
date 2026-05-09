#![cfg(test)]

use std::string::{FromUtf8Error, String};
use std::vec::Vec;

use crate::{
    AebError, AllocationError, InputError, LimitError, ParseError, Program, RunError, RunLimits,
    RunOutcome, RunResult, StepLimit, TraceSnapshotEvent,
};

pub(crate) enum TestFailure {
    Message(&'static str),
    Parse(ParseError),
    Run(RunError),
    Aeb(AebError),
    Utf8(FromUtf8Error),
    Allocation(AllocationError),
}

impl core::fmt::Debug for TestFailure {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Message(message) => formatter.debug_tuple("Message").field(message).finish(),
            Self::Parse(error) => formatter.debug_tuple("Parse").field(error).finish(),
            Self::Run(error) => formatter.debug_tuple("Run").field(error).finish(),
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

pub(crate) type TestResult = Result<(), TestFailure>;

pub(crate) fn test_limits() -> RunLimits {
    RunLimits::new(StepLimit::new(10_000))
}

pub(crate) fn run_source(source: &str, input: &str) -> Result<String, TestFailure> {
    let program = Program::parse_str(source)?;
    let result = program.run(input.as_bytes(), test_limits())?;
    Ok(String::from_utf8(into_result_bytes(result))?)
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
        RunOutcome::Stable(_) => Err(TestFailure::Message("stable output bytes differed")),
        RunOutcome::Return(_) => Err(TestFailure::Message("expected stable outcome")),
    }
}

pub(crate) fn expect_return_output<'result>(
    result: &'result RunResult,
    expected: &[u8],
) -> Result<&'result [u8], TestFailure> {
    match result.outcome() {
        RunOutcome::Return(output) if output.as_bytes() == expected => Ok(output.as_bytes()),
        RunOutcome::Return(_) => Err(TestFailure::Message("return output bytes differed")),
        RunOutcome::Stable(_) => Err(TestFailure::Message("expected return outcome")),
    }
}

pub(crate) fn expect_parse_error(source: &str) -> Result<ParseError, TestFailure> {
    match Program::parse_str(source) {
        Ok(_) => Err(TestFailure::Message("expected parse error")),
        Err(error) => Ok(error),
    }
}

pub(crate) fn expect_run_error(
    result: Result<RunResult, RunError>,
) -> Result<RunError, TestFailure> {
    match result {
        Ok(_) => Err(TestFailure::Message("expected runtime error")),
        Err(error) => Ok(error),
    }
}

pub(crate) fn expect_event<'events, 'program>(
    events: &'events [TraceSnapshotEvent<'program>],
    index: usize,
) -> Result<&'events TraceSnapshotEvent<'program>, TestFailure> {
    events
        .get(index)
        .ok_or(TestFailure::Message("expected trace event"))
}

pub(crate) fn expect_step_limit(error: RunError) -> Result<LimitError, TestFailure> {
    match error {
        RunError::Limit(error @ LimitError::Step { .. }) => Ok(error),
        RunError::Input(_)
        | RunError::Allocation(_)
        | RunError::StateSize(_)
        | RunError::Limit(_) => Err(TestFailure::Message("expected step limit error")),
    }
}

pub(crate) fn expect_state_limit(error: RunError) -> Result<LimitError, TestFailure> {
    match error {
        RunError::Limit(error @ LimitError::State { .. }) => Ok(error),
        RunError::Input(_)
        | RunError::Allocation(_)
        | RunError::StateSize(_)
        | RunError::Limit(_) => Err(TestFailure::Message("expected state limit error")),
    }
}

pub(crate) fn expect_input_error(error: RunError) -> Result<InputError, TestFailure> {
    match error {
        RunError::Input(error) => Ok(error),
        RunError::Allocation(_) | RunError::StateSize(_) | RunError::Limit(_) => {
            Err(TestFailure::Message("expected input error"))
        }
    }
}
