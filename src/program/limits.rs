use core::fmt;

use crate::bytes::{
    PayloadByteCount, ReturnOutputByteCount, RuntimeInputByteCount, RuntimeStateByteCount,
    TraceSnapshotByteCount,
};
use crate::inspect::RuleCount;

/// Source byte length measured before parsing starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceByteCount {
    /// Source byte length before parsing.
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
    /// Executable code-line length before whitespace compaction.
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
    /// Maximum accepted source byte length.
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

    /// Checks whether a measured source length exceeds this parser budget.
    pub(crate) const fn accepts(self, attempted_len: SourceByteCount) -> bool {
        attempted_len.get() <= self.value
    }
}

/// Maximum executable code-line length accepted by [`program::Program::parse`](crate::program::Program::parse).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CodeLineByteLimit {
    /// Maximum accepted executable code-line byte length.
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

    /// Checks whether a measured executable line length exceeds this parser budget.
    pub(crate) const fn accepts(self, attempted_len: CodeLineByteCount) -> bool {
        attempted_len.get() <= self.value
    }
}

/// Maximum parsed payload length accepted by [`program::Program::parse`](crate::program::Program::parse).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PayloadByteLimit {
    /// Maximum accepted executable payload byte length.
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

    /// Checks whether a measured payload length exceeds this parser budget.
    pub(crate) const fn accepts(self, attempted_len: PayloadByteCount) -> bool {
        attempted_len.get() <= self.value
    }
}

/// Maximum executable rule count accepted by [`program::Program::parse`](crate::program::Program::parse).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuleLimit {
    /// Maximum accepted executable rule count.
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

    /// Checks whether a parsed-rule count remains inside this parser budget.
    pub(crate) const fn accepts(self, attempted_count: RuleCount) -> bool {
        attempted_count.get() <= self.value
    }
}

/// Maximum number of committed execution steps allowed before the next matching rule fails.
///
/// A limit of `0` allows parsing and input materialization, but the first
/// matching rule fails with a step-limit error instead of committing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StepLimit {
    /// Maximum number of committed execution steps.
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

    /// Checks whether another step may be reserved after the completed count.
    pub(crate) const fn allows_next_after(self, completed_steps: StepCount) -> bool {
        completed_steps.get() < self.value
    }
}

/// Maximum number of executable rule-line attempts allowed in rule-attempt execution.
///
/// This is separate from [`StepLimit`]. A rule attempt observes one executable
/// rule line, whether or not that rule matches the current runtime state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuleAttemptLimit {
    /// Maximum number of consumed rule attempts.
    value: usize,
}

impl RuleAttemptLimit {
    /// Creates a rule-attempt limit from a primitive count.
    #[must_use]
    pub const fn new(value: usize) -> Self {
        Self { value }
    }

    /// Returns this limit as a primitive count.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }

    /// Checks whether another rule attempt may be reserved after the completed count.
    pub(crate) const fn allows_next_after(self, completed_attempts: RuleAttemptCount) -> bool {
        completed_attempts.get() < self.value
    }
}

/// Maximum runtime state length in bytes.
///
/// This applies both to the materialized initial input state and to every state
/// that would be produced by a rewrite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuntimeStateByteLimit {
    /// Maximum runtime-state byte length.
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

    /// Checks whether a runtime-state length remains inside this execution budget.
    pub(crate) const fn accepts(self, attempted_len: RuntimeStateByteCount) -> bool {
        attempted_len.get() <= self.value
    }
}

/// Maximum runtime input length accepted before owned byte classification.
///
/// This limit is exposed by [`RuntimeInputPolicy`](crate::policy::RuntimeInputPolicy) and enforced by
/// [`input::RuntimeInput::validate`](crate::input::RuntimeInput::validate)
/// before owned input allocation starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuntimeInputByteLimit {
    /// Maximum raw runtime-input byte length.
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

    /// Checks whether a runtime-input length remains inside this input budget.
    pub(crate) const fn accepts(self, attempted_len: RuntimeInputByteCount) -> bool {
        attempted_len.get() <= self.value
    }
}

/// Maximum `(return)` output length in bytes.
///
/// This applies only to output produced by a matched `(return)` rule. Stable
/// final states are governed by [`RuntimeStateByteLimit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReturnByteLimit {
    /// Maximum materialized `(return)` output byte length.
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

    /// Checks whether a return-output length remains inside this execution budget.
    pub(crate) const fn accepts(self, attempted_len: ReturnOutputByteCount) -> bool {
        attempted_len.get() <= self.value
    }
}

/// Maximum state/output bytes materialized for one trace snapshot event.
///
/// This limit is checked per event when converting borrowed trace events into
/// snapshot events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TraceSnapshotByteLimit {
    /// Maximum materialized bytes in one trace snapshot event.
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

    /// Checks whether one materialized trace event remains inside this snapshot budget.
    pub(crate) const fn accepts(self, attempted_len: TraceSnapshotByteCount) -> bool {
        attempted_len.get() <= self.value
    }
}

/// Number of committed execution steps.
///
/// Counts report committed rule applications only. A non-terminal rewrite and a
/// terminal `(return)` both increment this value; failed step attempts and
/// non-applying rule attempts do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StepCount {
    /// Committed execution steps.
    value: usize,
}

impl StepCount {
    /// ZERO boundary value.
    pub(crate) const ZERO: Self = Self { value: 0 };

    /// Returns this completed-step count as a primitive count.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }

    /// Returns the checked next result.
    pub(crate) fn checked_next(self) -> Option<Self> {
        let value = self.value.checked_add(1)?;
        Some(Self { value })
    }
}

/// Number of executable rule-line attempts consumed by a rule-attempt run.
///
/// Counts report inspected executable rule lines. A miss increments this count,
/// and a matched rule increments it independently from committed execution steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuleAttemptCount {
    /// Consumed rule attempts.
    value: usize,
}

impl RuleAttemptCount {
    /// ZERO boundary value.
    pub(crate) const ZERO: Self = Self { value: 0 };

    /// Returns this rule-attempt count as a primitive count.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }

    /// Returns the checked next result.
    pub(crate) fn checked_next(self) -> Option<Self> {
        let value = self.value.checked_add(1)?;
        Some(Self { value })
    }
}
