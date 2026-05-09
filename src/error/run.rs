use alloc::vec::Vec;
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
    /// Runtime state would exceed the configured state limit.
    StateLimit(StateLimitError),
    /// `(return)` output would exceed the configured return-output limit.
    ReturnLimit(ReturnLimitError),
    /// Trace snapshot materialization would exceed the configured trace limit.
    TraceLimit(TraceLimitError),
    /// Execution exceeded the configured step limit.
    StepLimit(StepLimitError),
}

impl Error for RunError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Input(error) => Some(error),
            Self::Allocation(error) => Some(error),
            Self::StateSize(error) => Some(error),
            Self::StateLimit(error) => Some(error),
            Self::ReturnLimit(error) => Some(error),
            Self::TraceLimit(error) => Some(error),
            Self::StepLimit(error) => Some(error),
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

impl From<StateLimitError> for RunError {
    fn from(value: StateLimitError) -> Self {
        Self::StateLimit(value)
    }
}

impl From<ReturnLimitError> for RunError {
    fn from(value: ReturnLimitError) -> Self {
        Self::ReturnLimit(value)
    }
}

impl From<TraceLimitError> for RunError {
    fn from(value: TraceLimitError) -> Self {
        Self::TraceLimit(value)
    }
}

impl From<StepLimitError> for RunError {
    fn from(value: StepLimitError) -> Self {
        Self::StepLimit(value)
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

/// State-limit failure reported before allocating an oversized state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateLimitError {
    limit: usize,
    attempted_len: usize,
    context: StateLimitContext,
}

impl StateLimitError {
    pub(crate) const fn new(
        limit: usize,
        attempted_len: usize,
        context: StateLimitContext,
    ) -> Self {
        Self {
            limit,
            attempted_len,
            context,
        }
    }

    /// Configured maximum runtime state length.
    #[must_use]
    pub const fn limit(&self) -> usize {
        self.limit
    }

    /// State length that would have been accepted without this guard.
    #[must_use]
    pub const fn attempted_len(&self) -> usize {
        self.attempted_len
    }

    /// Whether the limit was exceeded by input or by a rewrite.
    #[must_use]
    pub const fn context(&self) -> StateLimitContext {
        self.context
    }
}

impl Error for StateLimitError {}

/// Return-output limit failure reported before allocating oversized output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnLimitError {
    limit: usize,
    attempted_len: usize,
}

impl ReturnLimitError {
    pub(crate) const fn new(limit: usize, attempted_len: usize) -> Self {
        Self {
            limit,
            attempted_len,
        }
    }

    /// Configured maximum `(return)` output length.
    #[must_use]
    pub const fn limit(&self) -> usize {
        self.limit
    }

    /// Return payload length that would have been allocated.
    #[must_use]
    pub const fn attempted_len(&self) -> usize {
        self.attempted_len
    }
}

impl Error for ReturnLimitError {}

/// Trace snapshot materialization limit failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceLimitError {
    limit: usize,
    attempted_len: usize,
}

impl TraceLimitError {
    pub(crate) const fn new(limit: usize, attempted_len: usize) -> Self {
        Self {
            limit,
            attempted_len,
        }
    }

    /// Configured maximum trace snapshot byte length.
    #[must_use]
    pub const fn limit(&self) -> usize {
        self.limit
    }

    /// Trace state/output snapshot length that would have been allocated.
    #[must_use]
    pub const fn attempted_len(&self) -> usize {
        self.attempted_len
    }
}

impl Error for TraceLimitError {}

/// Step-limit failure with the last runtime state preserved as bytes.
#[derive(Debug, PartialEq, Eq)]
pub struct StepLimitError {
    max_steps: usize,
    state: Vec<u8>,
}

impl StepLimitError {
    pub(crate) fn new(max_steps: usize, state: Vec<u8>) -> Self {
        Self { max_steps, state }
    }

    /// Configured maximum step count.
    #[must_use]
    pub const fn max_steps(&self) -> usize {
        self.max_steps
    }

    /// Runtime state when the limit was hit.
    #[must_use]
    pub fn state(&self) -> &[u8] {
        &self.state
    }

    /// Consumes the error and returns the runtime state.
    #[must_use]
    pub fn into_state(self) -> Vec<u8> {
        self.state
    }
}

impl Error for StepLimitError {}
