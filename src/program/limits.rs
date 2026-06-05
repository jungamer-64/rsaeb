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

/// Maximum source length accepted by target-shape parse entrypoints such as
/// [`program::ExecutableProgram::parse_text`](crate::program::ExecutableProgram::parse_text)
/// and [`program::EmptyProgram::parse_text`](crate::program::EmptyProgram::parse_text).
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

    /// Admits a measured source length into this parser budget.
    pub(crate) const fn admit(self, attempted_len: SourceByteCount) -> Option<SourceBytePermit> {
        if attempted_len.get() <= self.value {
            Some(SourceBytePermit)
        } else {
            None
        }
    }
}

/// Permit proving a source byte count fits its parser budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SourceBytePermit;

/// Maximum executable code-line length accepted by target-shape parse entrypoints such as
/// [`program::ExecutableProgram::parse_text`](crate::program::ExecutableProgram::parse_text)
/// and [`program::EmptyProgram::parse_text`](crate::program::EmptyProgram::parse_text).
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

    /// Admits a measured executable line length into this parser budget.
    pub(crate) const fn admit(
        self,
        attempted_len: CodeLineByteCount,
    ) -> Option<CodeLineBytePermit> {
        if attempted_len.get() <= self.value {
            Some(CodeLineBytePermit)
        } else {
            None
        }
    }
}

/// Permit proving an executable code line fits its parser budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CodeLineBytePermit;

/// Maximum parsed payload length accepted by target-shape parse entrypoints such as
/// [`program::ExecutableProgram::parse_text`](crate::program::ExecutableProgram::parse_text)
/// and [`program::EmptyProgram::parse_text`](crate::program::EmptyProgram::parse_text).
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

    /// Admits a measured payload length into this parser budget.
    pub(crate) const fn admit(self, attempted_len: PayloadByteCount) -> Option<PayloadBytePermit> {
        if attempted_len.get() <= self.value {
            Some(PayloadBytePermit)
        } else {
            None
        }
    }
}

/// Permit proving a parsed payload fits its parser budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PayloadBytePermit;

/// Maximum executable rule count accepted by target-shape parse entrypoints such as
/// [`program::ExecutableProgram::parse_text`](crate::program::ExecutableProgram::parse_text)
/// and [`program::EmptyProgram::parse_text`](crate::program::EmptyProgram::parse_text).
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

    /// Admits a parsed-rule count into this parser budget.
    pub(crate) const fn admit(self, attempted_count: RuleCount) -> Option<RuleCountPermit> {
        if attempted_count.get() <= self.value {
            Some(RuleCountPermit)
        } else {
            None
        }
    }
}

/// Permit proving a parsed-rule count fits its parser budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RuleCountPermit;

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

    /// Admits the next committed step after the supplied completed count.
    pub(crate) fn admit_next_after(self, completed_steps: StepCount) -> Option<StepCountPermit> {
        if completed_steps.get() >= self.value {
            return None;
        }
        Some(StepCountPermit {
            next_step: completed_steps.checked_next()?,
        })
    }
}

/// Permit proving a next committed step fits the execution budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StepCountPermit {
    /// Step count admitted by the limit.
    next_step: StepCount,
}

impl StepCountPermit {
    /// Step count admitted by the execution limit.
    pub(crate) const fn step(self) -> StepCount {
        self.next_step
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

    /// Admits the next rule attempt after the supplied completed count.
    pub(crate) fn admit_next_after(
        self,
        completed_attempts: RuleAttemptCount,
    ) -> Option<RuleAttemptCountPermit> {
        if completed_attempts.get() >= self.value {
            return None;
        }
        Some(RuleAttemptCountPermit {
            next_attempt: completed_attempts.checked_next()?,
        })
    }
}

/// Permit proving a next rule attempt fits the rule-attempt budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RuleAttemptCountPermit {
    /// Rule-attempt count admitted by the limit.
    next_attempt: RuleAttemptCount,
}

impl RuleAttemptCountPermit {
    /// Rule-attempt count admitted by the attempt limit.
    pub(crate) const fn attempt(self) -> RuleAttemptCount {
        self.next_attempt
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

    /// Admits a runtime-state length into this execution budget.
    pub(crate) const fn admit(
        self,
        attempted_len: RuntimeStateByteCount,
    ) -> Option<RuntimeStateBytePermit> {
        if attempted_len.get() <= self.value {
            Some(RuntimeStateBytePermit {
                byte_count: attempted_len,
            })
        } else {
            None
        }
    }
}

/// Permit proving a runtime-state length fits its execution budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RuntimeStateBytePermit {
    /// Runtime-state byte count admitted by the limit.
    byte_count: RuntimeStateByteCount,
}

impl RuntimeStateBytePermit {
    /// Runtime-state byte count admitted by the limit.
    pub(crate) const fn byte_count(self) -> RuntimeStateByteCount {
        self.byte_count
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

    /// Admits a runtime-input length into this input budget.
    pub(crate) const fn admit(
        self,
        attempted_len: RuntimeInputByteCount,
    ) -> Option<RuntimeInputBytePermit> {
        if attempted_len.get() <= self.value {
            Some(RuntimeInputBytePermit {
                byte_count: attempted_len,
            })
        } else {
            None
        }
    }
}

/// Permit proving a runtime-input length fits its input budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RuntimeInputBytePermit {
    /// Runtime-input byte count admitted by the limit.
    byte_count: RuntimeInputByteCount,
}

impl RuntimeInputBytePermit {
    /// Runtime-input byte count admitted by the limit.
    pub(crate) const fn byte_count(self) -> RuntimeInputByteCount {
        self.byte_count
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

    /// Admits a return-output length into this execution budget.
    pub(crate) const fn admit(
        self,
        attempted_len: ReturnOutputByteCount,
    ) -> Option<ReturnOutputBytePermit> {
        if attempted_len.get() <= self.value {
            Some(ReturnOutputBytePermit {
                byte_count: attempted_len,
            })
        } else {
            None
        }
    }
}

/// Permit proving a return-output length fits its execution budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReturnOutputBytePermit {
    /// Return-output byte count admitted by the limit.
    byte_count: ReturnOutputByteCount,
}

impl ReturnOutputBytePermit {
    /// Return-output byte count admitted by the limit.
    pub(crate) const fn byte_count(self) -> ReturnOutputByteCount {
        self.byte_count
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

    /// Admits one materialized trace event into this snapshot budget.
    pub(crate) const fn admit(
        self,
        attempted_len: TraceSnapshotByteCount,
    ) -> Option<TraceSnapshotBytePermit> {
        if attempted_len.get() <= self.value {
            Some(TraceSnapshotBytePermit {
                byte_count: attempted_len,
            })
        } else {
            None
        }
    }
}

/// Permit proving a trace-snapshot event fits its trace budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TraceSnapshotBytePermit {
    /// Trace snapshot byte count admitted by the limit.
    byte_count: TraceSnapshotByteCount,
}

impl TraceSnapshotBytePermit {
    /// Trace snapshot byte count admitted by the limit.
    pub(crate) const fn byte_count(self) -> TraceSnapshotByteCount {
        self.byte_count
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
