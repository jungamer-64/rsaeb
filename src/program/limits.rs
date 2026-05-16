const DEFAULT_BYTE_BUDGET: usize = 16_777_216;

/// Default rewrite step budget for callers that want the crate policy value.
pub const DEFAULT_MAX_STEPS: StepLimit = StepLimit::new(1_000_000);
/// Default runtime-state byte budget for callers that want the crate policy value.
pub const DEFAULT_MAX_STATE_LEN: RuntimeStateByteLimit =
    RuntimeStateByteLimit::new(DEFAULT_BYTE_BUDGET);
/// Default runtime-input byte budget for callers that want the crate policy value.
pub const DEFAULT_MAX_INPUT_LEN: RuntimeInputByteLimit =
    RuntimeInputByteLimit::new(DEFAULT_BYTE_BUDGET);
/// Default `(return)` output byte budget for callers that want the crate policy value.
pub const DEFAULT_MAX_RETURN_LEN: ReturnByteLimit = ReturnByteLimit::new(DEFAULT_BYTE_BUDGET);
/// Default trace snapshot byte budget for callers that want the crate default.
pub const DEFAULT_MAX_TRACE_SNAPSHOT_LEN: TraceSnapshotByteLimit =
    TraceSnapshotByteLimit::new(DEFAULT_BYTE_BUDGET);

/// Maximum number of rewrite steps allowed before the next matching rule fails.
///
/// A limit of `0` allows parsing and input materialization, but the first
/// matching rule fails with a step-limit error instead of committing.
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
///
/// This applies both to the materialized initial input state and to every state
/// that would be produced by a rewrite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuntimeStateByteLimit {
    value: usize,
}

impl RuntimeStateByteLimit {
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

/// Maximum runtime input length accepted before owned byte classification.
///
/// This limit belongs to [`RuntimeInput::validate`](crate::RuntimeInput::validate),
/// not to [`Program::run`](crate::Program::run). Runtime state limits are
/// checked separately when execution materializes the validated input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuntimeInputByteLimit {
    value: usize,
}

impl RuntimeInputByteLimit {
    /// Creates a runtime-input byte limit from a primitive length.
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
///
/// This applies only to output produced by a matched `(return)` rule. Stable
/// final states are governed by [`RuntimeStateByteLimit`].
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
///
/// This limit is checked per event when converting borrowed trace events into
/// snapshot events.
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
///
/// Counts report committed steps only. Failed step attempts do not increment
/// this value.
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
/// state. Runtime input length is validated before execution with
/// [`RuntimeInputByteLimit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunLimits {
    steps: StepLimit,
    state_len: RuntimeStateByteLimit,
    return_len: ReturnByteLimit,
}

impl RunLimits {
    /// Creates limits with every runtime budget specified explicitly.
    #[must_use]
    pub const fn new(
        max_steps: StepLimit,
        max_state_len: RuntimeStateByteLimit,
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
    pub const fn state_byte_limit(self) -> RuntimeStateByteLimit {
        self.state_len
    }

    /// Maximum byte length accepted for `(return)` output.
    #[must_use]
    pub const fn return_byte_limit(self) -> ReturnByteLimit {
        self.return_len
    }

    /// Returns limits with a different step budget.
    ///
    /// The other runtime budgets are preserved.
    #[must_use]
    pub const fn with_step_limit(mut self, max_steps: StepLimit) -> Self {
        self.steps = max_steps;
        self
    }

    /// Returns limits with a different runtime-state budget.
    ///
    /// The step and return-output budgets are preserved.
    #[must_use]
    pub const fn with_state_byte_limit(mut self, max_state_len: RuntimeStateByteLimit) -> Self {
        self.state_len = max_state_len;
        self
    }

    /// Returns limits with a different return-output budget.
    ///
    /// The step and runtime-state budgets are preserved.
    #[must_use]
    pub const fn with_return_byte_limit(mut self, max_return_len: ReturnByteLimit) -> Self {
        self.return_len = max_return_len;
        self
    }
}

/// Resource limits for one trace-snapshot runtime invocation.
///
/// Runtime execution limits and trace snapshot materialization limits are
/// separate domains, but snapshot tracing needs both. Keeping them in one value
/// prevents trace APIs from growing parallel primitive-like budget arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceSnapshotLimits {
    run: RunLimits,
    snapshot: TraceSnapshotByteLimit,
}

impl TraceSnapshotLimits {
    /// Creates trace-snapshot limits from runtime limits and a snapshot byte
    /// budget.
    #[must_use]
    pub const fn new(run_limits: RunLimits, snapshot_byte_limit: TraceSnapshotByteLimit) -> Self {
        Self {
            run: run_limits,
            snapshot: snapshot_byte_limit,
        }
    }

    /// Runtime limits used by the underlying interpreter execution.
    #[must_use]
    pub const fn run_limits(self) -> RunLimits {
        self.run
    }

    /// Maximum bytes materialized for one trace snapshot event.
    #[must_use]
    pub const fn snapshot_byte_limit(self) -> TraceSnapshotByteLimit {
        self.snapshot
    }
}
