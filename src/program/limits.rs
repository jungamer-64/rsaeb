const DEFAULT_BYTE_BUDGET: usize = 16_777_216;

/// Default rewrite step budget for callers that want the crate policy value.
pub const DEFAULT_MAX_STEPS: StepLimit = StepLimit::new(1_000_000);
/// Default runtime-state byte budget for callers that want the crate policy value.
pub const DEFAULT_MAX_STATE_LEN: StateByteLimit = StateByteLimit::new(DEFAULT_BYTE_BUDGET);
/// Default `(return)` output byte budget for callers that want the crate policy value.
pub const DEFAULT_MAX_RETURN_LEN: ReturnByteLimit = ReturnByteLimit::new(DEFAULT_BYTE_BUDGET);
/// Default trace snapshot byte budget for callers that want the crate default.
pub const DEFAULT_MAX_TRACE_SNAPSHOT_LEN: TraceSnapshotByteLimit =
    TraceSnapshotByteLimit::new(DEFAULT_BYTE_BUDGET);

/// Maximum number of rewrite steps allowed before the next matching rule fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StepLimit {
    value: usize,
}

impl StepLimit {
    /// Creates a step limit from a primitive count.
    #[must_use]
    pub const fn new(value: usize) -> Self {
        Self { value }
    }

    /// Returns this limit as a primitive count.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }
}

/// Maximum runtime state length in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StateByteLimit {
    value: usize,
}

impl StateByteLimit {
    /// Creates a runtime-state byte limit from a primitive length.
    #[must_use]
    pub const fn new(value: usize) -> Self {
        Self { value }
    }

    /// Returns this limit as a primitive length.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }
}

/// Maximum `(return)` output length in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReturnByteLimit {
    value: usize,
}

impl ReturnByteLimit {
    /// Creates a `(return)` output byte limit from a primitive length.
    #[must_use]
    pub const fn new(value: usize) -> Self {
        Self { value }
    }

    /// Returns this limit as a primitive length.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }
}

/// Maximum state/output bytes materialized for one trace snapshot event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TraceSnapshotByteLimit {
    value: usize,
}

impl TraceSnapshotByteLimit {
    /// Creates a trace snapshot byte limit from a primitive length.
    #[must_use]
    pub const fn new(value: usize) -> Self {
        Self { value }
    }

    /// Returns this limit as a primitive length.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }
}

/// Number of completed rewrite steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StepCount {
    value: usize,
}

impl StepCount {
    pub(crate) const ZERO: Self = Self { value: 0 };

    /// Returns this completed-step count as a primitive count.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }

    pub(crate) fn checked_next(self) -> Option<Self> {
        let value = self.value.checked_add(1)?;
        Some(Self { value })
    }
}

/// Resource limits for one runtime invocation.
///
/// The interpreter checks these limits before allocating oversized runtime
/// states or return outputs. Step limits alone are not enough for a rewriting
/// system because a tiny number of steps can still expand into a very large
/// state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunLimits {
    steps: StepLimit,
    state_len: StateByteLimit,
    return_len: ReturnByteLimit,
}

impl RunLimits {
    /// Creates limits with every runtime budget specified explicitly.
    #[must_use]
    pub const fn new(
        max_steps: StepLimit,
        max_state_len: StateByteLimit,
        max_return_len: ReturnByteLimit,
    ) -> Self {
        Self {
            steps: max_steps,
            state_len: max_state_len,
            return_len: max_return_len,
        }
    }

    /// Maximum number of rewrite steps that may be applied.
    #[must_use]
    pub const fn step_limit(self) -> StepLimit {
        self.steps
    }

    /// Maximum runtime state length, including initial input and rewrite results.
    #[must_use]
    pub const fn state_byte_limit(self) -> StateByteLimit {
        self.state_len
    }

    /// Maximum byte length accepted for `(return)` output.
    #[must_use]
    pub const fn return_byte_limit(self) -> ReturnByteLimit {
        self.return_len
    }

    /// Returns limits with a different step budget.
    #[must_use]
    pub const fn with_step_limit(mut self, max_steps: StepLimit) -> Self {
        self.steps = max_steps;
        self
    }

    /// Returns limits with a different runtime-state budget.
    #[must_use]
    pub const fn with_state_byte_limit(mut self, max_state_len: StateByteLimit) -> Self {
        self.state_len = max_state_len;
        self
    }

    /// Returns limits with a different return-output budget.
    #[must_use]
    pub const fn with_return_byte_limit(mut self, max_return_len: ReturnByteLimit) -> Self {
        self.return_len = max_return_len;
        self
    }
}
