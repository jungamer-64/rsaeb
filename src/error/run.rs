use core::error::Error;

use crate::allocation::AllocationError;

/// Runtime execution error.
#[derive(Debug, PartialEq, Eq)]
pub enum RunError {
    /// Runtime input is invalid.
    Input(InputError),
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
            Self::Input(error) => Some(error),
            Self::Allocation(error) => Some(error),
            Self::StateSize(error) => Some(error),
            Self::Limit(error) => Some(error),
        }
    }
}

impl From<InputError> for RunError {
    fn from(value: InputError) -> Self {
        Self::Input(value)
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

/// Runtime input validation error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputError {
    column: usize,
    byte: u8,
}

impl InputError {
    pub(crate) const fn new(column: usize, byte: u8) -> Self {
        Self { column, byte }
    }

    /// One-based input column.
    #[must_use]
    pub const fn column(&self) -> usize {
        self.column
    }

    /// Rejected byte.
    #[must_use]
    pub const fn byte(&self) -> u8 {
        self.byte
    }
}

impl Error for InputError {}

/// Runtime state-size failure caused by arithmetic overflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateSizeError {
    state: usize,
    lhs: usize,
    rhs: usize,
}

impl StateSizeError {
    pub(crate) const fn new(state_len: usize, lhs_len: usize, rhs_len: usize) -> Self {
        Self {
            state: state_len,
            lhs: lhs_len,
            rhs: rhs_len,
        }
    }

    /// Runtime state length before the failing rewrite.
    #[must_use]
    pub const fn state_len(&self) -> usize {
        self.state
    }

    /// Matched left-side length that would be removed.
    #[must_use]
    pub const fn lhs_len(&self) -> usize {
        self.lhs
    }

    /// Right-side payload length that would be inserted.
    #[must_use]
    pub const fn rhs_len(&self) -> usize {
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
        limit: usize,
        /// State length that would have been accepted without this guard.
        attempted_len: usize,
    },
    /// `(return)` output would exceed the configured return-output limit.
    Return {
        /// Configured maximum `(return)` output length.
        limit: usize,
        /// Return payload length that would have been allocated.
        attempted_len: usize,
    },
    /// Trace snapshot materialization would exceed the configured trace limit.
    TraceSnapshot {
        /// Configured maximum trace snapshot byte length.
        limit: usize,
        /// Trace state/output snapshot length that would have been allocated.
        attempted_len: usize,
    },
    /// Execution exceeded the configured step limit.
    Step {
        /// Configured maximum step count.
        max_steps: usize,
        /// Number of completed rewrite steps when the next match was found.
        completed_steps: usize,
        /// Runtime state length when the limit was hit.
        state_len: usize,
    },
}

impl LimitError {
    pub(crate) const fn state(
        context: StateLimitContext,
        limit: usize,
        attempted_len: usize,
    ) -> Self {
        Self::State {
            context,
            limit,
            attempted_len,
        }
    }

    pub(crate) const fn return_output(limit: usize, attempted_len: usize) -> Self {
        Self::Return {
            limit,
            attempted_len,
        }
    }

    pub(crate) const fn trace_snapshot(limit: usize, attempted_len: usize) -> Self {
        Self::TraceSnapshot {
            limit,
            attempted_len,
        }
    }

    pub(crate) const fn step(max_steps: usize, completed_steps: usize, state_len: usize) -> Self {
        Self::Step {
            max_steps,
            completed_steps,
            state_len,
        }
    }
}

impl Error for LimitError {}
