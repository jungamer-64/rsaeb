use core::fmt;

use crate::allocation::{AllocationContext, AllocationError, AllocationErrorKind};

use super::{
    AebError, FallibleTraceSnapshotRunError, InputColumn, InputError, LeftModifierKind, LimitError,
    ParseError, ParseErrorKind, ParseErrorLocation, PayloadKind, RightActionKind, RunError,
    RuntimeInvariantError, StateLimitContext, StateSizeError, TraceSnapshotError,
    TraceSnapshotRunError, TracedRunError,
};

impl fmt::Display for AllocationContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProgramCodeLine => f.write_str("program code line"),
            Self::ProgramPayload => f.write_str("program payload"),
            Self::ProgramRuleTable => f.write_str("program rule table"),
            Self::CanonicalSource => f.write_str("canonical source bytes"),
            Self::RuntimeInput => f.write_str("runtime input state"),
            Self::RuntimeOnceRuleState => f.write_str("runtime once rule state"),
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
            Self::Input(error) => error.fmt(f),
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
            Self::Allocation(error) => error.fmt(f),
            Self::StateSize(error) => error.fmt(f),
            Self::Limit(error) => error.fmt(f),
            Self::Invariant(error) => error.fmt(f),
        }
    }
}

impl fmt::Display for RuntimeInvariantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingOnceRuleState {
                once_slot,
                once_state_count,
            } => write!(
                f,
                "runtime invariant violation: once rule slot {once_slot} has no runtime state; allocated once state count: {once_state_count}",
            ),
            Self::ConsumedOnceRuleSlot { once_slot } => write!(
                f,
                "runtime invariant violation: matched once rule slot {once_slot} was already consumed",
            ),
            Self::ValidatedInputBecameNonAscii { byte } => write!(
                f,
                "runtime invariant violation: validated runtime input byte became non-ASCII: 0x{:02x}",
                byte.get(),
            ),
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

impl fmt::Display for TraceSnapshotRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Run(error) => error.fmt(f),
            Self::Snapshot(error) => error.fmt(f),
        }
    }
}

impl<E> fmt::Display for FallibleTraceSnapshotRunError<E>
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

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonAscii { column, byte } => write!(
                f,
                "input error: non-ASCII byte 0x{:02x} at column {column}",
                byte.get(),
            ),
            Self::ColumnOverflow => write!(f, "input error: column number overflow"),
        }
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
