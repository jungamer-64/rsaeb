mod support;

use rsaeb::error::{
    AebError, InputError, LimitError, ParseErrorKind, ParseErrorLocation, PayloadKind, RunError,
    RuntimeInputBytesError, StateLimitContext,
};
use rsaeb::limits::{ReturnByteLimit, StateByteLimit, StepLimit};
use rsaeb::{
    DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, Program, ProgramSource, RunLimits, RuntimeInput,
    RuntimeInputBytes,
};
use support::{TestFailure, TestResult, ensure_eq, ensure_matches};

fn expect_run_error<T>(result: Result<T, RunError>) -> Result<RunError, TestFailure> {
    match result {
        Ok(_) => Err(TestFailure::message("expected runtime error")),
        Err(error) => Ok(error),
    }
}

fn expect_step_limit(error: RunError) -> Result<LimitError, TestFailure> {
    match error {
        RunError::Limit(error @ LimitError::Step { .. }) => Ok(error),
        RunError::Allocation(_)
        | RunError::StateSize(_)
        | RunError::Limit(_)
        | RunError::Invariant(_) => Err(TestFailure::message("expected step limit error")),
    }
}

fn expect_state_limit(error: RunError) -> Result<LimitError, TestFailure> {
    match error {
        RunError::Limit(error @ LimitError::State { .. }) => Ok(error),
        RunError::Allocation(_)
        | RunError::StateSize(_)
        | RunError::Limit(_)
        | RunError::Invariant(_) => Err(TestFailure::message("expected state limit error")),
    }
}

#[test]
fn parse_error_location_and_kind_are_structured() -> TestResult {
    let Err(error) = Program::parse(ProgramSource::from_str("a=b=c")) else {
        return Err(TestFailure::message("expected parse error"));
    };

    ensure_eq!(error.line().get(), 1)?;
    match error.location() {
        ParseErrorLocation::Position(position) => {
            ensure_eq!(position.line().get(), 1)?;
            ensure_eq!(position.column().get(), 4)?;
        }
        ParseErrorLocation::Line(_) => {
            return Err(TestFailure::message("expected positioned parse error"));
        }
    }
    ensure_matches(
        matches!(error.kind(), ParseErrorKind::MultipleEquals),
        "expected multiple-equals parse error",
    )
}

#[test]
fn payload_and_modifier_errors_keep_domain_information() -> TestResult {
    let Err(error) = Program::parse(ProgramSource::from_str("a = b (")) else {
        return Err(TestFailure::message("expected reserved syntax error"));
    };
    ensure_matches(
        matches!(
            error.kind(),
            ParseErrorKind::ReservedSyntaxInPayload {
                payload_kind: PayloadKind::RightSideData,
                ..
            }
        ),
        "expected right payload syntax error",
    )?;

    let Err(error) = Program::parse(ProgramSource::from_str("(start)(once)a=b")) else {
        return Err(TestFailure::message("expected modifier order error"));
    };
    ensure_matches(
        matches!(
            error.kind(),
            ParseErrorKind::UnsupportedLeftModifierOrder { .. }
        ),
        "expected left modifier order error",
    )
}

#[test]
fn input_error_and_top_level_aeb_error_are_structured() -> TestResult {
    let Err(error) = RuntimeInput::validate(&[0xff]) else {
        return Err(TestFailure::message("expected input error"));
    };

    ensure_matches(
        matches!(
            error,
            InputError::NonAscii { column, .. } if column.get() == 1
        ),
        "expected runtime input error",
    )?;

    let error = AebError::from(error);
    ensure_matches(
        matches!(error, AebError::Input(_)),
        "expected top-level input error",
    )?;

    let Err(error) = RuntimeInputBytes::from_slice(&[0xff]) else {
        return Err(TestFailure::message("expected owned input error"));
    };
    ensure_matches(
        matches!(
            error,
            RuntimeInputBytesError::Input(InputError::NonAscii { column, .. })
                if column.get() == 1
        ),
        "expected owned runtime input validation error",
    )
}

#[test]
fn display_errors_name_their_domain_contexts() -> TestResult {
    let Err(parse_error) = Program::parse(ProgramSource::from_str("a=b=c")) else {
        return Err(TestFailure::message("expected parse error"));
    };
    ensure_eq!(
        parse_error.to_string(),
        "parse error at line 1, column 4: multiple '=' characters are not allowed",
    )?;

    let Err(input_error) = RuntimeInput::validate(&[0xff]) else {
        return Err(TestFailure::message("expected input error"));
    };
    ensure_eq!(
        input_error.to_string(),
        "input error: non-ASCII byte 0xff at column 1",
    )?;

    let Err(input_error) = RuntimeInputBytes::from_slice(&[0xff]) else {
        return Err(TestFailure::message("expected owned input error"));
    };
    ensure_eq!(
        input_error.to_string(),
        "input error: non-ASCII byte 0xff at column 1",
    )
}

#[test]
fn limit_errors_report_step_state_and_return_domains() -> TestResult {
    let state_error = Program::parse(ProgramSource::from_str("# no executable rules"))?.run(
        RuntimeInput::validate(b"aa")?,
        RunLimits::new(
            StepLimit::new(10),
            StateByteLimit::new(1),
            ReturnByteLimit::new(10),
        ),
    );
    let state_error = expect_state_limit(expect_run_error(state_error)?)?;
    ensure_matches(
        matches!(
            state_error,
            LimitError::State {
                context: StateLimitContext::Input,
                limit,
                attempted_len,
            } if limit == StateByteLimit::new(1) && attempted_len.get() == 2
        ),
        "expected input state limit details",
    )?;
    ensure_eq!(
        state_error.to_string(),
        "state limit exceeded by runtime input; attempted length: 2, limit: 1",
    )?;

    let step_error = Program::parse(ProgramSource::from_str("a=b"))?.run(
        RuntimeInput::validate(b"a")?,
        RunLimits::new(
            StepLimit::new(0),
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        ),
    );
    let step_error = expect_step_limit(expect_run_error(step_error)?)?;
    ensure_eq!(
        step_error.to_string(),
        "step limit exceeded after 0 steps; max steps: 0, state length: 1 bytes",
    )?;

    let return_error = Program::parse(ProgramSource::from_str("a=(return)ok"))?.run(
        RuntimeInput::validate(b"a")?,
        RunLimits::new(
            StepLimit::new(1),
            DEFAULT_MAX_STATE_LEN,
            ReturnByteLimit::new(1),
        ),
    );
    ensure_matches(
        matches!(
            expect_run_error(return_error)?,
            RunError::Limit(LimitError::Return { .. })
        ),
        "expected return limit error",
    )
}
