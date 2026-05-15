use core::error::Error;

use crate::allocation::AllocationError;
use crate::bytes::{
    NonAsciiInputByte, PayloadByteCount, ReturnOutputByteCount, RuntimeStateByteCount,
};
use crate::program::{ReturnByteLimit, StateByteLimit, StepCount, StepLimit};

/// Runtime execution error.
#[derive(Debug, PartialEq, Eq)]
pub enum RunError {
    /// A fallible allocation failed during runtime execution.
    Allocation(AllocationError),
    /// A rewrite length could not be represented.
    StateSize(StateSizeError),
    /// A configured runtime budget would be exceeded.
    Limit(LimitError),
}

impl Error for RunError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Allocation(error) => Some(error),
            Self::StateSize(error) => Some(error),
            Self::Limit(error) => Some(error),
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

/// Runtime input boundary error.
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
    /// Runtime input exceeded its construction byte budget.
    Limit {
        /// Configured maximum runtime input length.
        limit: StateByteLimit,
        /// Runtime input length that would have been classified.
        attempted_len: RuntimeStateByteCount,
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

    pub(crate) const fn limit(limit: StateByteLimit, attempted_len: RuntimeStateByteCount) -> Self {
        Self::Limit {
            limit,
            attempted_len,
        }
    }
}

impl Error for RuntimeInputError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Allocation(error) => Some(error),
            Self::NonAscii { .. } | Self::ColumnOverflow | Self::Limit { .. } => None,
        }
    }
}

impl From<AllocationError> for RuntimeInputError {
    fn from(value: AllocationError) -> Self {
        Self::Allocation(value)
    }
}

/// One-based runtime input column.
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

/// Context in which the configured state limit was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateLimitContext {
    /// Initial runtime input was larger than the configured state limit.
    Input,
    /// A rewrite would create a state larger than the configured state limit.
    Rewrite,
}

/// Configured runtime budget failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LimitError {
    /// Runtime state would exceed the configured state length limit.
    State {
        /// Whether the limit was exceeded by input or by a rewrite.
        context: StateLimitContext,
        /// Configured maximum runtime state length.
        limit: StateByteLimit,
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
        context: StateLimitContext,
        limit: StateByteLimit,
        attempted_len: RuntimeStateByteCount,
    ) -> Self {
        Self::State {
            context,
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
