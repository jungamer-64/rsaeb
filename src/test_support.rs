#![cfg(test)]

use std::string::{FromUtf8Error, String};

use crate::{
    AebError, InputError, ParseError, Program, RunError, RunOptions, RunResult, StepLimitError,
    TraceEvent,
};

pub(crate) enum TestFailure {
    Message(&'static str),
    Parse(ParseError),
    Run(RunError),
    Aeb(AebError),
    Utf8(FromUtf8Error),
}

impl core::fmt::Debug for TestFailure {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Message(message) => formatter.debug_tuple("Message").field(message).finish(),
            Self::Parse(error) => formatter.debug_tuple("Parse").field(error).finish(),
            Self::Run(error) => formatter.debug_tuple("Run").field(error).finish(),
            Self::Aeb(error) => formatter.debug_tuple("Aeb").field(error).finish(),
            Self::Utf8(error) => formatter.debug_tuple("Utf8").field(error).finish(),
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

pub(crate) type TestResult = Result<(), TestFailure>;

pub(crate) fn run_source(source: &str, input: &str) -> Result<String, TestFailure> {
    let program = Program::parse(source)?;
    let result = program.run(input.as_bytes(), RunOptions::new(10_000))?;
    Ok(String::from_utf8(result.into_output())?)
}

pub(crate) fn expect_parse_error(source: &str) -> Result<ParseError, TestFailure> {
    match Program::parse(source) {
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
    events: &'events [TraceEvent<'program>],
    index: usize,
) -> Result<&'events TraceEvent<'program>, TestFailure> {
    events
        .get(index)
        .ok_or(TestFailure::Message("expected trace event"))
}

pub(crate) fn expect_step_limit(error: RunError) -> Result<StepLimitError, TestFailure> {
    match error {
        RunError::StepLimit(error) => Ok(error),
        RunError::Input(_) | RunError::Allocation(_) | RunError::StateSize(_) => {
            Err(TestFailure::Message("expected step limit error"))
        }
    }
}

pub(crate) fn expect_input_error(error: RunError) -> Result<InputError, TestFailure> {
    match error {
        RunError::Input(error) => Ok(error),
        RunError::Allocation(_) | RunError::StateSize(_) | RunError::StepLimit(_) => {
            Err(TestFailure::Message("expected input error"))
        }
    }
}
