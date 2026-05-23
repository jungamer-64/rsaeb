use core::error::Error;

use crate::allocation::AllocationError;
use crate::bytes::{
    NonAsciiInputByte, PayloadByteCount, ReturnOutputByteCount, RuntimeInputByteCount,
    RuntimeStateByteCount,
};
use crate::limits::{
    ReturnByteLimit, RuntimeInputByteLimit, RuntimeStateByteLimit, StepCount, StepLimit,
};

/// Runtime execution error.
///
/// This error is returned after parsing and runtime input validation have
/// already succeeded. It covers allocation failures inside execution,
/// unrepresentable rewrite sizes, configured runtime budget failures, and
/// broken parser/runtime invariants that should be unreachable through the
/// public API.
#[derive(Debug, PartialEq, Eq)]
pub enum RunError {
    /// A fallible allocation failed during runtime execution.
    Allocation(AllocationError),
    /// A rewrite length could not be represented.
    StateSize(StateSizeError),
    /// A configured runtime budget would be exceeded.
    Limit(LimitError),
    /// Parsed program metadata and runtime-owned state no longer agree.
    InternalInvariant(InternalInvariantError),
}

impl Error for RunError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Allocation(error) => Some(error),
            Self::StateSize(error) => Some(error),
            Self::Limit(error) => Some(error),
            Self::InternalInvariant(error) => Some(error),
        }
    }
}

impl From<AllocationError> for RunError {
    fn from(value: AllocationError) -> Self {
        Self::Allocation(value)
    }
}

impl From<StateSizeError> for RunError {
    fn from(value: StateSizeError) -> Self {
        Self::StateSize(value)
    }
}

impl From<LimitError> for RunError {
    fn from(value: LimitError) -> Self {
        Self::Limit(value)
    }
}

impl From<InternalInvariantError> for RunError {
    fn from(value: InternalInvariantError) -> Self {
        Self::InternalInvariant(value)
    }
}

/// Runtime input validation boundary error.
///
/// This error is produced before execution starts, while raw host bytes are
/// being classified as [`input::RuntimeInput`](crate::input::RuntimeInput).
/// It is intentionally separate from [`RunError`] so callers can report invalid
/// input without treating it as a runtime failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeInputError {
    /// Runtime input contained a non-ASCII byte.
    NonAscii {
        /// One-based input column.
        column: InputColumn,
        /// Rejected byte.
        byte: NonAsciiInputByte,
    },
    /// A one-based input column could not be represented.
    ColumnOverflow,
    /// Runtime input exceeded its input-byte construction budget.
    InputLimit {
        /// Configured maximum runtime input length.
        limit: RuntimeInputByteLimit,
        /// Runtime input length that would have been classified.
        attempted_len: RuntimeInputByteCount,
    },
    /// Storing validated runtime input failed.
    Allocation(AllocationError),
}

impl RuntimeInputError {
    pub(crate) const fn non_ascii(column: InputColumn, byte: NonAsciiInputByte) -> Self {
        Self::NonAscii { column, byte }
    }

    pub(crate) const fn column_overflow() -> Self {
        Self::ColumnOverflow
    }

    pub(crate) const fn input_limit(
        limit: RuntimeInputByteLimit,
        attempted_len: RuntimeInputByteCount,
    ) -> Self {
        Self::InputLimit {
            limit,
            attempted_len,
        }
    }

}

impl Error for RuntimeInputError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Allocation(error) => Some(error),
            Self::NonAscii { .. } | Self::ColumnOverflow | Self::InputLimit { .. } => None,
        }
    }
}

impl From<AllocationError> for RuntimeInputError {
    fn from(value: AllocationError) -> Self {
        Self::Allocation(value)
    }
}

/// Run admission boundary error.
///
/// This error is produced after runtime input validation and before execution
/// starts, while validated input is admitted as the initial runtime state under
/// execution limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunAdmissionError {
    /// Runtime input exceeded the initial runtime-state budget for this run.
    InitialStateLimit {
        /// Configured maximum runtime state length.
        limit: RuntimeStateByteLimit,
        /// Runtime state length that would have been materialized.
        attempted_len: RuntimeStateByteCount,
    },
}

impl RunAdmissionError {
    pub(crate) const fn initial_state_limit(
        limit: RuntimeStateByteLimit,
        attempted_len: RuntimeStateByteCount,
    ) -> Self {
        Self::InitialStateLimit {
            limit,
            attempted_len,
        }
    }
}

impl Error for RunAdmissionError {}

/// One-based runtime input column.
///
/// Columns count raw input bytes starting at one. They are reported only by the
/// runtime-input boundary, not by source parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InputColumn {
    one_based: usize,
}

impl InputColumn {
    pub(crate) fn from_zero_based(zero_based: usize) -> Option<Self> {
        let one_based = zero_based.checked_add(1)?;
        Some(Self { one_based })
    }

    /// One-based input column as a primitive value.
    #[must_use]
    pub const fn get(self) -> usize {
        self.one_based
    }
}

/// Runtime state-size failure caused by arithmetic overflow.
///
/// This is distinct from a configured byte limit. It means the interpreter
/// could not represent the length of the state that a rewrite would produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateSizeError {
    state: RuntimeStateByteCount,
    lhs: PayloadByteCount,
    rhs: PayloadByteCount,
}

impl StateSizeError {
    pub(crate) const fn new(
        state_len: RuntimeStateByteCount,
        lhs_len: PayloadByteCount,
        rhs_len: PayloadByteCount,
    ) -> Self {
        Self {
            state: state_len,
            lhs: lhs_len,
            rhs: rhs_len,
        }
    }

    /// Runtime state length before the failing rewrite.
    #[must_use]
    pub const fn state_len(&self) -> RuntimeStateByteCount {
        self.state
    }

    /// Matched left-side length that would be removed.
    #[must_use]
    pub const fn lhs_len(&self) -> PayloadByteCount {
        self.lhs
    }

    /// Right-side payload length that would be inserted.
    #[must_use]
    pub const fn rhs_len(&self) -> PayloadByteCount {
        self.rhs
    }
}

impl Error for StateSizeError {}

/// Runtime invariant violation that should be unrepresentable from public
/// inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InternalInvariantError {
    /// A parsed `(once)` rule referenced no runtime state slot.
    MissingOnceRuleState,
    /// Runtime attempted to commit a `(once)` rule without a fresh slot permit.
    ConsumedOnceRuleCommit,
    /// A committed rule position did not resolve inside its originating program.
    MissingCommittedRule,
    /// A committed return transition pointed at a non-return rule.
    ReturnedRuleWithoutOutput,
    /// A previously validated runtime-state match no longer resolved inside
    /// the current execution state.
    InvalidStateMatchRange,
}

impl InternalInvariantError {
    pub(crate) const fn missing_once_rule_state() -> Self {
        Self::MissingOnceRuleState
    }

    pub(crate) const fn consumed_once_rule_commit() -> Self {
        Self::ConsumedOnceRuleCommit
    }

    pub(crate) const fn missing_committed_rule() -> Self {
        Self::MissingCommittedRule
    }

    pub(crate) const fn returned_rule_without_output() -> Self {
        Self::ReturnedRuleWithoutOutput
    }

    pub(crate) const fn invalid_state_match_range() -> Self {
        Self::InvalidStateMatchRange
    }
}

impl Error for InternalInvariantError {}

/// Configured runtime budget failure.
///
/// Limits are checked before committing the operation that would exceed them,
/// so errors report the attempted length or completed step count at the
/// rejection point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LimitError {
    /// Runtime state would exceed the configured state length limit.
    State {
        /// Configured maximum runtime state length.
        limit: RuntimeStateByteLimit,
        /// State length that would have been accepted without this guard.
        attempted_len: RuntimeStateByteCount,
    },
    /// `(return)` output would exceed the configured return-output limit.
    Return {
        /// Configured maximum `(return)` output length.
        limit: ReturnByteLimit,
        /// Return payload length that would have been allocated.
        attempted_len: ReturnOutputByteCount,
    },
    /// Execution exceeded the configured step limit.
    Step {
        /// Configured maximum step count.
        max_steps: StepLimit,
        /// Number of completed rewrite steps when the next match was found.
        completed_steps: StepCount,
        /// Runtime state length when the limit was hit.
        state_len: RuntimeStateByteCount,
    },
}

impl LimitError {
    pub(crate) const fn state(
        limit: RuntimeStateByteLimit,
        attempted_len: RuntimeStateByteCount,
    ) -> Self {
        Self::State {
            limit,
            attempted_len,
        }
    }

    pub(crate) const fn return_output(
        limit: ReturnByteLimit,
        attempted_len: ReturnOutputByteCount,
    ) -> Self {
        Self::Return {
            limit,
            attempted_len,
        }
    }

    pub(crate) const fn step(
        max_steps: StepLimit,
        completed_steps: StepCount,
        state_len: RuntimeStateByteCount,
    ) -> Self {
        Self::Step {
            max_steps,
            completed_steps,
            state_len,
        }
    }
}

impl Error for LimitError {}

#[cfg(test)]
mod tests {
    use super::InputColumn;
    use crate::test_support::{TestResult, ensure_eq};

    /// # Errors
    ///
    /// Returns `TestFailure` if input-column conversion accepts an
    /// unrepresentable index or rejects zero.
    #[test]
    fn input_column_rejects_unrepresentable_zero_based_index() -> TestResult {
        ensure_eq!(InputColumn::from_zero_based(usize::MAX), None)?;
        ensure_eq!(
            InputColumn::from_zero_based(0).map(InputColumn::get),
            Some(1),
        )?;
        Ok(())
    }
}
