use core::error::Error;

use crate::allocation::AllocationError;
use crate::bytes::{
    NonAsciiInputByte, RuntimeInputByteCount, RuntimeStateByteCount,
};
use crate::limits::{RuntimeInputByteLimit, RuntimeStateByteLimit};

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
    /// Builds the non ascii value.
    pub(crate) const fn non_ascii(column: InputColumn, byte: NonAsciiInputByte) -> Self {
        Self::NonAscii { column, byte }
    }

    /// Builds the column overflow value.
    pub(crate) const fn column_overflow() -> Self {
        Self::ColumnOverflow
    }

    /// Builds the input limit value.
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
/// execution limits. It means the input bytes were valid runtime input, but the
/// execution policy rejected them as the initial state for this run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunAdmissionError {
    /// Runtime input exceeded the initial runtime-state budget for this run.
    InitialStateTooLarge {
        /// Configured maximum runtime state length.
        limit: RuntimeStateByteLimit,
        /// Runtime state length that would have been materialized.
        attempted_len: RuntimeStateByteCount,
    },
}

impl RunAdmissionError {
    /// Builds the initial state limit value.
    pub(crate) const fn initial_state_limit(
        limit: RuntimeStateByteLimit,
        attempted_len: RuntimeStateByteCount,
    ) -> Self {
        Self::InitialStateTooLarge {
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
    /// One-based runtime-input byte column.
    one_based: usize,
}

impl InputColumn {
    /// Builds an index from a zero-based offset.
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
