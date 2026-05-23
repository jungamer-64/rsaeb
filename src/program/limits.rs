use core::fmt;

const DEFAULT_BYTE_BUDGET: usize = 16_777_216;

/// Default program-source byte budget for callers that want the crate policy value.
pub const DEFAULT_MAX_SOURCE_LEN: SourceByteLimit = SourceByteLimit::new(DEFAULT_BYTE_BUDGET);
/// Default executable code-line byte budget for callers that want the crate policy value.
pub const DEFAULT_MAX_CODE_LINE_LEN: CodeLineByteLimit =
    CodeLineByteLimit::new(DEFAULT_BYTE_BUDGET);
/// Default executable payload byte budget for callers that want the crate policy value.
pub const DEFAULT_MAX_PAYLOAD_LEN: PayloadByteLimit = PayloadByteLimit::new(DEFAULT_BYTE_BUDGET);
/// Default parsed-rule budget for callers that want the crate policy value.
pub const DEFAULT_MAX_RULES: RuleLimit = RuleLimit::new(1_000_000);
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
/// Default parser budgets for callers that want the crate policy value.
pub const DEFAULT_PARSE_LIMITS: ParseLimits = ParseLimits::new(
    DEFAULT_MAX_SOURCE_LEN,
    DEFAULT_MAX_CODE_LINE_LEN,
    DEFAULT_MAX_PAYLOAD_LEN,
    DEFAULT_MAX_RULES,
);

/// Source byte length measured before parsing starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceByteCount {
    value: usize,
}

impl SourceByteCount {
    /// Creates a source byte count from a primitive length.
    #[must_use]
    pub(crate) const fn new(value: usize) -> Self {
        Self { value }
    }

    /// Returns this count as a primitive length.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }
}

impl fmt::Display for SourceByteCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}

/// Executable code-line byte length after comment removal and before whitespace compaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CodeLineByteCount {
    value: usize,
}

impl CodeLineByteCount {
    /// Creates a code-line byte count from a primitive length.
    #[must_use]
    pub(crate) const fn new(value: usize) -> Self {
        Self { value }
    }

    /// Returns this count as a primitive length.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }
}

impl fmt::Display for CodeLineByteCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}

/// Maximum source length accepted by [`program::Program::parse`](crate::program::Program::parse).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceByteLimit {
    value: usize,
}

impl SourceByteLimit {
    /// Creates a source byte limit from a primitive length.
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

/// Maximum executable code-line length accepted by [`program::Program::parse`](crate::program::Program::parse).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CodeLineByteLimit {
    value: usize,
}

impl CodeLineByteLimit {
    /// Creates a code-line byte limit from a primitive length.
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

/// Maximum parsed payload length accepted by [`program::Program::parse`](crate::program::Program::parse).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PayloadByteLimit {
    value: usize,
}

impl PayloadByteLimit {
    /// Creates a payload byte limit from a primitive length.
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

/// Maximum executable rule count accepted by [`program::Program::parse`](crate::program::Program::parse).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuleLimit {
    value: usize,
}

impl RuleLimit {
    /// Creates a parsed-rule limit from a primitive count.
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

/// Resource limits for one parser invocation.
///
/// Parser limits are host policy. They are checked before parser-owned
/// allocations grow beyond the declared source, line, payload, or rule budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseLimits {
    source_len: SourceByteLimit,
    code_line_len: CodeLineByteLimit,
    payload_len: PayloadByteLimit,
    rules: RuleLimit,
}

impl ParseLimits {
    /// Creates parser limits with every budget specified explicitly.
    #[must_use]
    pub const fn new(
        max_source_len: SourceByteLimit,
        max_code_line_len: CodeLineByteLimit,
        max_payload_len: PayloadByteLimit,
        max_rules: RuleLimit,
    ) -> Self {
        Self {
            source_len: max_source_len,
            code_line_len: max_code_line_len,
            payload_len: max_payload_len,
            rules: max_rules,
        }
    }

    /// Maximum source bytes accepted before line parsing starts.
    #[must_use]
    pub const fn source_byte_limit(self) -> SourceByteLimit {
        self.source_len
    }

    /// Maximum bytes accepted in one executable code line before whitespace compaction.
    #[must_use]
    pub const fn code_line_byte_limit(self) -> CodeLineByteLimit {
        self.code_line_len
    }

    /// Maximum bytes accepted in one executable payload.
    #[must_use]
    pub const fn payload_byte_limit(self) -> PayloadByteLimit {
        self.payload_len
    }

    /// Maximum executable rules accepted in one parsed program.
    #[must_use]
    pub const fn rule_limit(self) -> RuleLimit {
        self.rules
    }
}

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
/// This limit belongs to [`input::RuntimeInput::validate`](crate::input::RuntimeInput::validate),
/// not to [`program::Program::run`](crate::program::Program::run). Runtime state limits are
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
