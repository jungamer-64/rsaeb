use core::fmt;

use crate::allocation::{AllocationContext, AllocationError, AllocationErrorKind};

use super::{
    InputColumn, LeftModifierKind, ParseError, ParseErrorKind, ParseErrorLocation,
    ParseRepresentationError, PayloadKind, ReturnOutputLimitError, RewriteSizeError,
    RightActionKind, RuleAttemptLimitError, RuleAttemptStepError, RunAdmissionError, RunError,
    RunFinishError, RunStartError, RunStepError, RuntimeInputError, RuntimeStateLimitError,
    StepLimitError, TraceSnapshotError, TraceSnapshotRunError, TracedRunError,
};

impl fmt::Display for AllocationContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProgramCodeLine => f.write_str("program code line"),
            Self::ProgramPayload => f.write_str("program payload"),
            Self::ProgramRuleTable => f.write_str("program rule table"),
            Self::CanonicalSource => f.write_str("canonical source bytes"),
            Self::RuntimeInputValidation => f.write_str("runtime input validation"),
            Self::RuntimeRuleAvailability => f.write_str("runtime rule availability"),
            Self::RuntimeRewriteState => f.write_str("runtime rewrite state"),
            Self::PayloadView => f.write_str("payload view"),
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
            AllocationErrorKind::ReservationFailed { requested_capacity } => {
                write!(
                    f,
                    "allocation reservation failure while building {}; requested capacity: {}",
                    self.context(),
                    requested_capacity,
                )
            }
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
            Self::Representation(error) => error.fmt(f),
            Self::Limit(error) => error.fmt(f),
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

impl fmt::Display for ParseRepresentationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceLineNumber => f.write_str("source line number could not be represented"),
            Self::SourceColumn { line } => write!(
                f,
                "source column could not be represented at line {}",
                line.get(),
            ),
            Self::RulePosition => f.write_str("rule position could not be represented"),
            Self::RuleCount => f.write_str("rule count could not be represented"),
        }
    }
}

impl fmt::Display for super::ParseLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source {
                limit,
                attempted_len,
            } => write!(
                f,
                "source length {attempted_len} exceeds the configured source limit {}",
                limit.get(),
            ),
            Self::CodeLine {
                limit,
                attempted_len,
            } => write!(
                f,
                "code line length {attempted_len} exceeds the configured code-line limit {}",
                limit.get(),
            ),
            Self::Payload {
                limit,
                attempted_len,
            } => write!(
                f,
                "payload length {attempted_len} exceeds the configured payload limit {}",
                limit.get(),
            ),
            Self::Rules {
                limit,
                attempted_count,
            } => write!(
                f,
                "rule count {} exceeds the configured rule limit {}",
                attempted_count.get(),
                limit.get(),
            ),
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
            Self::Start(error) => error.fmt(f),
            Self::Finish(error) => error.fmt(f),
        }
    }
}

impl fmt::Display for RunStartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Allocation(error) => {
                write!(f, "run start failed: {error}")
            }
        }
    }
}

impl fmt::Display for RunStepError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Allocation(error) => error.fmt(f),
            Self::RewriteSize(error) => error.fmt(f),
            Self::RuntimeStateLimit(error) => error.fmt(f),
            Self::ReturnOutputLimit(error) => error.fmt(f),
            Self::StepLimit(error) => error.fmt(f),
        }
    }
}

impl fmt::Display for RuleAttemptStepError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Step(error) => error.fmt(f),
            Self::RuleAttemptLimit(error) => error.fmt(f),
        }
    }
}

impl fmt::Display for RunFinishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Step(error) => error.fmt(f),
            Self::FinalOutput(error) => {
                write!(f, "final output materialization failed: {error}")
            }
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

impl fmt::Display for TraceSnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Limit {
                limit,
                attempted_len,
            } => write!(
                f,
                "trace snapshot limit exceeded; attempted length: {attempted_len}, limit: {}",
                limit.get(),
            ),
            Self::Allocation(error) => error.fmt(f),
        }
    }
}

impl<E> fmt::Display for TraceSnapshotRunError<E>
where
    E: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Run(error) => error.fmt(f),
            Self::Snapshot(error) => error.fmt(f),
            Self::Trace(error) => write!(f, "trace callback failed: {error}"),
        }
    }
}

impl fmt::Display for RuntimeInputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonAscii { column, byte } => write!(
                f,
                "input error: non-ASCII byte 0x{:02x} at column {column}",
                byte.get(),
            ),
            Self::ColumnOverflow => write!(f, "input error: column number overflow"),
            Self::InputLimit {
                limit,
                attempted_len,
            } => write!(
                f,
                "input error: runtime input length {} exceeds the configured input limit {}",
                attempted_len.get(),
                limit.get()
            ),
            Self::Allocation(error) => error.fmt(f),
        }
    }
}

impl fmt::Display for RunAdmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InitialStateTooLarge {
                limit,
                attempted_len,
            } => write!(
                f,
                "run admission error: initial runtime state length {} exceeds the configured state limit {}",
                attempted_len.get(),
                limit.get()
            ),
        }
    }
}

impl fmt::Display for InputColumn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(f)
    }
}

impl fmt::Display for RewriteSizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "rewrite size failure: replacing {} bytes in a {} byte state with {} bytes",
            self.lhs_len(),
            self.state_len(),
            self.rhs_len(),
        )
    }
}

impl fmt::Display for RuntimeStateLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "rewrite state limit exceeded; attempted length: {}, limit: {}",
            self.attempted_len(),
            self.limit().get(),
        )
    }
}

impl fmt::Display for ReturnOutputLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "return output limit exceeded; attempted length: {}, limit: {}",
            self.attempted_len(),
            self.limit().get(),
        )
    }
}

impl fmt::Display for StepLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "step limit exceeded after {} steps; max steps: {}, state length: {} bytes",
            self.completed_steps().get(),
            self.max_steps().get(),
            self.state_len(),
        )
    }
}

impl fmt::Display for RuleAttemptLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "rule-attempt limit exceeded after {} attempts; max attempts: {}, state length: {} bytes",
            self.completed_attempts().get(),
            self.max_attempts().get(),
            self.state_len(),
        )
    }
}

/// Returns the printable ASCII character or a replacement marker.
fn printable_ascii(byte: u8) -> char {
    if byte.is_ascii() {
        char::from(byte)
    } else {
        '\u{fffd}'
    }
}
