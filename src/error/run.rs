use core::error::Error;

use crate::allocation::AllocationError;
use crate::bytes::{
    NonAsciiInputByte, PayloadByteCount, ReturnOutputByteCount, RuntimeInputByteCount,
    RuntimeStateByteCount,
};
use crate::inspect::{OnceRuleCount, RulePosition};
use crate::limits::{
    ReturnByteLimit, RuleAttemptCount, RuleAttemptLimit, RuntimeInputByteLimit,
    RuntimeStateByteLimit, StepCount, StepLimit,
};

/// Run-to-completion execution error.
///
/// This is the composed error returned by run-to-completion execution through
/// [`Program::execute`](crate::program::Program::execute) and traced run APIs.
/// It does not include input validation or run admission, because those happen
/// before execution starts.
#[derive(Debug, PartialEq, Eq)]
pub enum RunError {
    /// The run could not be started.
    Start(RunStartError),
    /// The started run failed before producing a terminal result.
    Finish(RunFinishError),
}

impl Error for RunError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Start(error) => Some(error),
            Self::Finish(error) => Some(error),
        }
    }
}

impl From<RunStartError> for RunError {
    fn from(value: RunStartError) -> Self {
        Self::Start(value)
    }
}

impl From<RunFinishError> for RunError {
    fn from(value: RunFinishError) -> Self {
        Self::Finish(value)
    }
}

/// Error while constructing one runtime execution from an admitted seed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunStartError {
    /// Per-run execution state allocation failed.
    Allocation(AllocationError),
}

impl Error for RunStartError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Allocation(error) => Some(error),
        }
    }
}

impl From<AllocationError> for RunStartError {
    fn from(value: AllocationError) -> Self {
        Self::Allocation(value)
    }
}

/// Error while advancing one ordinary matched-rule step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunStepError {
    /// Runtime allocation failed while preparing or committing the candidate step.
    Allocation(AllocationError),
    /// Parsed `(once)` slot metadata did not match the per-run once-state table.
    OnceRuleState(OnceRuleStateError),
    /// Rewrite length arithmetic could not be represented.
    RewriteSize(RewriteSizeError),
    /// The candidate rewrite would exceed the runtime-state limit.
    RuntimeStateLimit(RuntimeStateLimitError),
    /// The candidate `(return)` output would exceed the return-output limit.
    ReturnOutputLimit(ReturnOutputLimitError),
    /// The candidate step would exceed the step limit.
    StepLimit(StepLimitError),
}

impl Error for RunStepError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Allocation(error) => Some(error),
            Self::OnceRuleState(error) => Some(error),
            Self::RewriteSize(error) => Some(error),
            Self::RuntimeStateLimit(error) => Some(error),
            Self::ReturnOutputLimit(error) => Some(error),
            Self::StepLimit(error) => Some(error),
        }
    }
}

impl From<AllocationError> for RunStepError {
    fn from(value: AllocationError) -> Self {
        Self::Allocation(value)
    }
}

impl From<OnceRuleStateError> for RunStepError {
    fn from(value: OnceRuleStateError) -> Self {
        Self::OnceRuleState(value)
    }
}

impl From<RewriteSizeError> for RunStepError {
    fn from(value: RewriteSizeError) -> Self {
        Self::RewriteSize(value)
    }
}

impl From<RuntimeStateLimitError> for RunStepError {
    fn from(value: RuntimeStateLimitError) -> Self {
        Self::RuntimeStateLimit(value)
    }
}

impl From<ReturnOutputLimitError> for RunStepError {
    fn from(value: ReturnOutputLimitError) -> Self {
        Self::ReturnOutputLimit(value)
    }
}

impl From<StepLimitError> for RunStepError {
    fn from(value: StepLimitError) -> Self {
        Self::StepLimit(value)
    }
}

/// Error while advancing an owned ordinary step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnedRunStepError {
    /// Ordinary runtime step preparation failed.
    Step(RunStepError),
    /// Retaining the owned rule witness failed.
    RuleWitnessAllocation(AllocationError),
}

impl Error for OwnedRunStepError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Step(error) => Some(error),
            Self::RuleWitnessAllocation(error) => Some(error),
        }
    }
}

impl From<RunStepError> for OwnedRunStepError {
    fn from(value: RunStepError) -> Self {
        Self::Step(value)
    }
}

/// Error while advancing one rule-attempt step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleAttemptStepError {
    /// Matched-rule execution failed.
    Step(RunStepError),
    /// The next executable rule-line attempt would exceed the attempt limit.
    RuleAttemptLimit(RuleAttemptLimitError),
}

impl Error for RuleAttemptStepError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Step(error) => Some(error),
            Self::RuleAttemptLimit(error) => Some(error),
        }
    }
}

impl From<RunStepError> for RuleAttemptStepError {
    fn from(value: RunStepError) -> Self {
        Self::Step(value)
    }
}

impl From<RuleAttemptLimitError> for RuleAttemptStepError {
    fn from(value: RuleAttemptLimitError) -> Self {
        Self::RuleAttemptLimit(value)
    }
}

/// Error while finishing a run session that has already started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunFinishError {
    /// A later matched-rule step failed.
    Step(RunStepError),
    /// Stable final-output materialization failed.
    FinalOutput(AllocationError),
}

impl Error for RunFinishError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Step(error) => Some(error),
            Self::FinalOutput(error) => Some(error),
        }
    }
}

impl From<RunStepError> for RunFinishError {
    fn from(value: RunStepError) -> Self {
        Self::Step(value)
    }
}

/// Runtime once-state table did not contain a parser-assigned slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnceRuleStateError {
    /// Rule whose parser-assigned slot was missing.
    rule_position: RulePosition,
    /// Zero-based parser-assigned `(once)` slot index.
    slot_index: usize,
    /// Number of once-state cells allocated for the run.
    once_rule_count: OnceRuleCount,
}

impl OnceRuleStateError {
    /// Builds a missing once-slot error.
    pub(crate) const fn missing_slot(
        rule_position: RulePosition,
        slot_index: usize,
        once_rule_count: OnceRuleCount,
    ) -> Self {
        Self {
            rule_position,
            slot_index,
            once_rule_count,
        }
    }

    /// Rule whose parser-assigned slot was missing.
    #[must_use]
    pub const fn rule_position(&self) -> RulePosition {
        self.rule_position
    }

    /// Zero-based parser-assigned `(once)` slot index.
    #[must_use]
    pub const fn slot_index(&self) -> usize {
        self.slot_index
    }

    /// Number of once-state cells allocated for the run.
    #[must_use]
    pub const fn once_rule_count(&self) -> OnceRuleCount {
        self.once_rule_count
    }
}

impl Error for OnceRuleStateError {}

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
/// an execution policy. It means the input bytes were valid runtime input, but the
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

/// Rewrite size failure caused by arithmetic overflow.
///
/// This is distinct from a configured byte limit. It means the interpreter
/// could not represent the length of the state that a rewrite would produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteSizeError {
    /// Runtime state length before the rewrite.
    state: RuntimeStateByteCount,
    /// Matched left-side payload length being removed.
    lhs: PayloadByteCount,
    /// Right-side payload length being inserted.
    rhs: PayloadByteCount,
}

impl RewriteSizeError {
    /// Records the lengths that overflowed rewrite-size arithmetic.
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

impl Error for RewriteSizeError {}

/// Runtime state byte-limit failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStateLimitError {
    /// Configured maximum runtime state length.
    limit: RuntimeStateByteLimit,
    /// State length that would have been accepted without this guard.
    attempted_len: RuntimeStateByteCount,
}

impl RuntimeStateLimitError {
    /// Builds a runtime state limit error.
    pub(crate) const fn new(
        limit: RuntimeStateByteLimit,
        attempted_len: RuntimeStateByteCount,
    ) -> Self {
        Self {
            limit,
            attempted_len,
        }
    }

    /// Configured maximum runtime state length.
    #[must_use]
    pub const fn limit(&self) -> RuntimeStateByteLimit {
        self.limit
    }

    /// State length rejected by the limit.
    #[must_use]
    pub const fn attempted_len(&self) -> RuntimeStateByteCount {
        self.attempted_len
    }
}

impl Error for RuntimeStateLimitError {}

/// Return-output byte-limit failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnOutputLimitError {
    /// Configured maximum `(return)` output length.
    limit: ReturnByteLimit,
    /// Return payload length that would have been allocated.
    attempted_len: ReturnOutputByteCount,
}

impl ReturnOutputLimitError {
    /// Builds a return-output limit error.
    pub(crate) const fn new(limit: ReturnByteLimit, attempted_len: ReturnOutputByteCount) -> Self {
        Self {
            limit,
            attempted_len,
        }
    }

    /// Configured maximum return-output length.
    #[must_use]
    pub const fn limit(&self) -> ReturnByteLimit {
        self.limit
    }

    /// Return-output length rejected by the limit.
    #[must_use]
    pub const fn attempted_len(&self) -> ReturnOutputByteCount {
        self.attempted_len
    }
}

impl Error for ReturnOutputLimitError {}

/// Step-limit failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepLimitError {
    /// Configured maximum step count.
    max_steps: StepLimit,
    /// Number of completed execution steps when the next match was found.
    completed_steps: StepCount,
    /// Runtime state length when the limit was hit.
    state_len: RuntimeStateByteCount,
}

impl StepLimitError {
    /// Builds a step-limit error.
    pub(crate) const fn new(
        max_steps: StepLimit,
        completed_steps: StepCount,
        state_len: RuntimeStateByteCount,
    ) -> Self {
        Self {
            max_steps,
            completed_steps,
            state_len,
        }
    }

    /// Configured maximum step count.
    #[must_use]
    pub const fn max_steps(&self) -> StepLimit {
        self.max_steps
    }

    /// Number of committed steps before rejection.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.completed_steps
    }

    /// Runtime state length at rejection.
    #[must_use]
    pub const fn state_len(&self) -> RuntimeStateByteCount {
        self.state_len
    }
}

impl Error for StepLimitError {}

/// Rule-attempt-limit failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleAttemptLimitError {
    /// Configured maximum executable rule-line attempts.
    max_attempts: RuleAttemptLimit,
    /// Number of completed rule attempts when the next rule line was reached.
    completed_attempts: RuleAttemptCount,
    /// Runtime state length when the limit was hit.
    state_len: RuntimeStateByteCount,
}

impl RuleAttemptLimitError {
    /// Builds a rule-attempt-limit error.
    pub(crate) const fn new(
        max_attempts: RuleAttemptLimit,
        completed_attempts: RuleAttemptCount,
        state_len: RuntimeStateByteCount,
    ) -> Self {
        Self {
            max_attempts,
            completed_attempts,
            state_len,
        }
    }

    /// Configured maximum executable rule-line attempts.
    #[must_use]
    pub const fn max_attempts(&self) -> RuleAttemptLimit {
        self.max_attempts
    }

    /// Number of committed rule attempts before rejection.
    #[must_use]
    pub const fn completed_attempts(&self) -> RuleAttemptCount {
        self.completed_attempts
    }

    /// Runtime state length at rejection.
    #[must_use]
    pub const fn state_len(&self) -> RuntimeStateByteCount {
        self.state_len
    }
}

impl Error for RuleAttemptLimitError {}

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
