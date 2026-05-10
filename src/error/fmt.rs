use core::fmt;

use crate::allocation::{AllocationContext, AllocationError, AllocationErrorKind};

use super::{
    AebError, InputColumn, InputError, LeftModifierKind, LimitError, ParseError, ParseErrorKind,
    ParseErrorLocation, PayloadKind, RightActionKind, RunError, StateLimitContext, StateSizeError,
    TracedRunError,
};

impl fmt::Display for AllocationContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProgramRules => f.write_str("program rule table"),
            Self::CompactCodeLine => f.write_str("compact code line"),
            Self::CanonicalSource => f.write_str("canonical source bytes"),
            Self::Payload => f.write_str("program payload"),
            Self::RuntimeInput => f.write_str("runtime input state"),
            Self::OnceRuleState => f.write_str("once rule state"),
            Self::RuntimeState => f.write_str("runtime rewrite state"),
            Self::RuntimeStateView => f.write_str("runtime state view"),
            Self::FinalOutput => f.write_str("final output"),
            Self::ReturnOutput => f.write_str("return output"),
            Self::TraceSnapshot => f.write_str("trace snapshot"),
        }
    }
}

impl fmt::Display for AllocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind() {
            AllocationErrorKind::CapacityOverflow => {
                write!(
                    f,
                    "allocation capacity overflow while building {}",
                    self.context(),
                )
            }
            AllocationErrorKind::ReserveFailed { requested_capacity } => {
                write!(
                    f,
                    "allocation failure while building {}; requested capacity: {}",
                    self.context(),
                    requested_capacity,
                )
            }
        }
    }
}

impl fmt::Display for AebError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(error) => error.fmt(f),
            Self::Run(error) => error.fmt(f),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.location() {
            ParseErrorLocation::Line(line) => write!(f, "parse error at line {}", line.get())?,
            ParseErrorLocation::Position(position) => write!(
                f,
                "parse error at line {}, column {}",
                position.line().get(),
                position.column().get()
            )?,
        }

        write!(f, ": {}", self.kind())
    }
}

impl fmt::Display for ParseErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Allocation(error) => error.fmt(f),
            Self::NonAsciiInCode { byte } => {
                write!(f, "non-ASCII byte 0x{:02x} in code", byte.get())
            }
            Self::NonPrintableAsciiInCode { byte } => {
                write!(f, "non-printable ASCII byte 0x{:02x} in code", byte.get())
            }
            Self::MissingEquals => f.write_str("missing '='"),
            Self::MultipleEquals => f.write_str("multiple '=' characters are not allowed"),
            Self::ReservedSyntaxInPayload { byte, payload_kind } => write!(
                f,
                "reserved syntax byte '{}' in {payload_kind}",
                printable_ascii(byte.get()),
            ),
            Self::UnsupportedLeftModifierOrder { modifier } => write!(
                f,
                "duplicated or unsupported left-side modifier order at {modifier}"
            ),
            Self::UnsupportedRightActionSyntax { action } => {
                write!(
                    f,
                    "nested or unsupported right-side action syntax at {action}"
                )
            }
        }
    }
}

impl fmt::Display for LeftModifierKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Once => f.write_str("(once)"),
            Self::Start => f.write_str("(start)"),
            Self::End => f.write_str("(end)"),
        }
    }
}

impl fmt::Display for RightActionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start => f.write_str("(start)"),
            Self::End => f.write_str("(end)"),
            Self::Return => f.write_str("(return)"),
        }
    }
}

impl fmt::Display for PayloadKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LeftSideData => f.write_str("left-side data"),
            Self::RightSideData => f.write_str("right-side data"),
            Self::RightSideMoveStartPayload => f.write_str("right-side move-to-start payload"),
            Self::RightSideMoveEndPayload => f.write_str("right-side move-to-end payload"),
            Self::RightSideReturnPayload => f.write_str("right-side return payload"),
        }
    }
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(error) => error.fmt(f),
            Self::Allocation(error) => error.fmt(f),
            Self::StateSize(error) => error.fmt(f),
            Self::Limit(error) => error.fmt(f),
        }
    }
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

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "input error: non-ASCII byte 0x{:02x} at column {}",
            self.byte().get(),
            self.column(),
        )
    }
}

impl fmt::Display for InputColumn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(f)
    }
}

impl fmt::Display for StateSizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "state size failure: replacing {} bytes in a {} byte state with {} bytes",
            self.lhs_len(),
            self.state_len(),
            self.rhs_len(),
        )
    }
}

impl fmt::Display for StateLimitContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input => f.write_str("runtime input"),
            Self::Rewrite => f.write_str("rewrite result"),
        }
    }
}

impl fmt::Display for LimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::State {
                context,
                limit,
                attempted_len,
            } => write!(
                f,
                "state limit exceeded by {context}; attempted length: {attempted_len}, limit: {}",
                limit.get(),
            ),
            Self::Return {
                limit,
                attempted_len,
            } => write!(
                f,
                "return output limit exceeded; attempted length: {attempted_len}, limit: {}",
                limit.get(),
            ),
            Self::TraceSnapshot {
                limit,
                attempted_len,
            } => write!(
                f,
                "trace snapshot limit exceeded; attempted length: {attempted_len}, limit: {}",
                limit.get(),
            ),
            Self::Step {
                max_steps,
                completed_steps,
                state_len,
            } => write!(
                f,
                "step limit exceeded after {} steps; max steps: {}, state length: {state_len} bytes",
                completed_steps.get(),
                max_steps.get(),
            ),
        }
    }
}

fn printable_ascii(byte: u8) -> char {
    if byte.is_ascii() {
        byte as char
    } else {
        '\u{fffd}'
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use crate::test_support::{
        TestResult, ensure_eq, expect_input_error, expect_parse_error, expect_run_error,
        expect_state_limit, expect_step_limit, runtime_input,
    };
    use crate::{
        AllocationContext, AllocationError, DEFAULT_MAX_STATE_LEN, Program, ReturnByteLimit,
        RunLimits, RuntimeInput, StateByteLimit, StepLimit, TraceSnapshotByteLimit,
    };

    #[test]
    fn allocation_display_names_the_failed_context_and_capacity() -> TestResult {
        let error = AllocationError::reserve_failed(AllocationContext::TraceSnapshot, 123);

        ensure_eq(
            error.to_string(),
            "allocation failure while building trace snapshot; requested capacity: 123",
        )?;

        let error = AllocationError::reserve_failed(AllocationContext::RuntimeStateView, 456);

        ensure_eq(
            error.to_string(),
            "allocation failure while building runtime state view; requested capacity: 456",
        )?;

        let error = AllocationError::capacity_overflow(AllocationContext::CanonicalSource);

        ensure_eq(
            error.to_string(),
            "allocation capacity overflow while building canonical source bytes",
        )?;
        Ok(())
    }

    #[test]
    fn parse_error_display_includes_line_column_and_structured_reason() -> TestResult {
        let error = expect_parse_error("a=b=c")?;

        ensure_eq(
            error.to_string(),
            "parse error at line 1, column 4: multiple '=' characters are not allowed",
        )?;
        Ok(())
    }

    #[test]
    fn input_error_display_keeps_byte_and_original_column() -> TestResult {
        let error = expect_run_error(RuntimeInput::parse(&[0xff], DEFAULT_MAX_STATE_LEN))?;
        let error = expect_input_error(error)?;

        ensure_eq(
            error.to_string(),
            "input error: non-ASCII byte 0xff at column 1",
        )?;
        Ok(())
    }

    #[test]
    fn state_limit_display_names_context_attempted_length_and_limit() -> TestResult {
        let limits = RunLimits::bounded(
            StepLimit::new(10),
            StateByteLimit::new(1),
            ReturnByteLimit::new(10),
            TraceSnapshotByteLimit::new(10),
        );
        let error = expect_run_error(RuntimeInput::parse(b"aa", limits.state_byte_limit()))?;
        let error = expect_state_limit(error)?;

        ensure_eq(
            error.to_string(),
            "state limit exceeded by runtime input; attempted length: 2, limit: 1",
        )?;
        Ok(())
    }

    #[test]
    fn step_limit_display_reports_limit_and_preserved_state_len() -> TestResult {
        let program = Program::parse_str("a=b")?;
        let limits = RunLimits::new(StepLimit::new(0));
        let error = expect_run_error(program.run(runtime_input(b"a", limits)?, limits))?;
        let error = expect_step_limit(error)?;

        ensure_eq(
            error.to_string(),
            "step limit exceeded after 0 steps; max steps: 0, state length: 1 bytes",
        )?;
        Ok(())
    }
}
