use crate::bytes::{ReturnOutputByteCount, RuntimeStateByteCount};
use crate::error::{LimitError, RunError};
use crate::limits::{
    ExecutionLimits, ReturnByteLimit, RuleAttemptCount, RuleAttemptLimit, RuntimeStateByteLimit,
    StepCount, StepLimit,
};

/// Runtime byte budget that can reject a measured byte count.
trait RuntimeByteLimit<Count>: Copy {
    /// Checks whether the measured byte count is inside this budget.
    fn accepts_count(self, attempted_len: Count) -> bool;

    /// Builds the typed runtime limit error for a rejected byte count.
    fn limit_error(self, attempted_len: Count) -> LimitError;
}

/// Monotonic execution budget that can reserve the next committed count.
trait ReservationLimit<Count>: Copy {
    /// Checks whether another count may be reserved after the completed count.
    fn allows_next_after_count(self, completed_count: Count) -> bool;

    /// Builds the typed runtime limit error for a rejected reservation.
    fn limit_error(self, completed_count: Count, state_len: RuntimeStateByteCount) -> LimitError;
}

/// Monotonic count that can advance by one without losing its domain.
trait ReservableCount: Copy {
    /// Returns the next representable count.
    fn checked_next_count(self) -> Option<Self>;
}

/// Execution budgets plus the number of committed execution steps.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RuntimeBudgetState {
    /// Host execution policy admitted for this run.
    limits: ExecutionLimits,
    /// Steps committed so far.
    completed_steps: StepCount,
}

/// Rule-attempt budget plus the number of consumed executable rule-line attempts.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RuleAttemptBudgetState {
    /// Host rule-attempt policy admitted for this run.
    limit: RuleAttemptLimit,
    /// Rule attempts consumed so far.
    completed_attempts: RuleAttemptCount,
}

/// Borrow-tied next step reservation.
#[derive(Debug)]
pub(crate) struct StepReservation<'budget> {
    /// Budget state borrowed until the reservation is either committed or dropped.
    budget: &'budget mut RuntimeBudgetState,
    /// Step count to publish when the rule application commits.
    next_step: StepCount,
}

/// Borrow-tied next rule-attempt reservation.
#[derive(Debug)]
pub(crate) struct RuleAttemptReservation<'budget> {
    /// Attempt budget borrowed until the reservation is either committed or dropped.
    budget: &'budget mut RuleAttemptBudgetState,
    /// Rule-attempt count to publish when the rule line is consumed.
    next_attempt: RuleAttemptCount,
}

impl RuntimeBudgetState {
    /// Starts runtime budget tracking for a newly admitted run.
    pub(crate) const fn new(limits: ExecutionLimits) -> Self {
        Self {
            limits,
            completed_steps: StepCount::ZERO,
        }
    }

    /// Number of execution steps committed so far.
    pub(crate) const fn completed_steps(&self) -> StepCount {
        self.completed_steps
    }

    /// Checks a candidate rewrite state against runtime state limits.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if the rewritten state would exceed the configured
    /// runtime state limit.
    pub(crate) fn ensure_rewrite_state_len(
        &self,
        attempted_len: RuntimeStateByteCount,
    ) -> Result<(), RunError> {
        ensure_runtime_byte_limit(self.limits.state_byte_limit(), attempted_len)
    }

    /// Checks a `(return)` payload against return-output limits.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if the return payload exceeds the configured return
    /// output limit.
    pub(crate) fn ensure_return_len(
        &self,
        attempted_len: ReturnOutputByteCount,
    ) -> Result<(), RunError> {
        ensure_runtime_byte_limit(self.limits.return_byte_limit(), attempted_len)
    }

    /// Reserves the next step number before a rule commits.
    ///
    /// # Errors
    ///
    /// Returns `LimitError` if the step limit is reached or the next step
    /// count cannot be represented.
    pub(crate) fn reserve_next_step(
        &mut self,
        state_len: RuntimeStateByteCount,
    ) -> Result<StepReservation<'_>, RunError> {
        let next_step =
            reserve_next_count(self.limits.step_limit(), self.completed_steps, state_len)?;

        Ok(StepReservation {
            budget: self,
            next_step,
        })
    }
}

impl StepReservation<'_> {
    /// Checks a candidate rewrite state against runtime state limits.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if the rewritten state would exceed the configured
    /// runtime state limit.
    pub(crate) fn ensure_rewrite_state_len(
        &self,
        attempted_len: RuntimeStateByteCount,
    ) -> Result<(), RunError> {
        self.budget.ensure_rewrite_state_len(attempted_len)
    }

    /// Checks a `(return)` payload against return-output limits.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if the return payload exceeds the configured return
    /// output limit.
    pub(crate) fn ensure_return_len(
        &self,
        attempted_len: ReturnOutputByteCount,
    ) -> Result<(), RunError> {
        self.budget.ensure_return_len(attempted_len)
    }

    /// Publishes the reserved step count.
    pub(crate) fn commit(self) -> StepCount {
        self.budget.completed_steps = self.next_step;
        self.next_step
    }
}

impl RuleAttemptBudgetState {
    /// Starts rule-attempt budget tracking for a newly admitted rule-attempt run.
    pub(crate) const fn new(limit: RuleAttemptLimit) -> Self {
        Self {
            limit,
            completed_attempts: RuleAttemptCount::ZERO,
        }
    }

    /// Number of executable rule-line attempts consumed so far.
    pub(crate) const fn completed_attempts(&self) -> RuleAttemptCount {
        self.completed_attempts
    }

    /// Reserves the next rule-attempt number before a rule line is evaluated.
    ///
    /// # Errors
    ///
    /// Returns `LimitError` if the attempt limit is reached or the next attempt
    /// count cannot be represented.
    pub(crate) fn reserve_next_attempt(
        &mut self,
        state_len: RuntimeStateByteCount,
    ) -> Result<RuleAttemptReservation<'_>, RunError> {
        let next_attempt = reserve_next_count(self.limit, self.completed_attempts, state_len)?;

        Ok(RuleAttemptReservation {
            budget: self,
            next_attempt,
        })
    }
}

impl RuleAttemptReservation<'_> {
    /// Publishes the reserved rule-attempt count.
    pub(crate) fn commit(self) -> RuleAttemptCount {
        self.budget.completed_attempts = self.next_attempt;
        self.next_attempt
    }
}

impl RuntimeByteLimit<RuntimeStateByteCount> for RuntimeStateByteLimit {
    fn accepts_count(self, attempted_len: RuntimeStateByteCount) -> bool {
        self.accepts(attempted_len)
    }

    fn limit_error(self, attempted_len: RuntimeStateByteCount) -> LimitError {
        LimitError::state(self, attempted_len)
    }
}

impl RuntimeByteLimit<ReturnOutputByteCount> for ReturnByteLimit {
    fn accepts_count(self, attempted_len: ReturnOutputByteCount) -> bool {
        self.accepts(attempted_len)
    }

    fn limit_error(self, attempted_len: ReturnOutputByteCount) -> LimitError {
        LimitError::return_output(self, attempted_len)
    }
}

impl ReservationLimit<StepCount> for StepLimit {
    fn allows_next_after_count(self, completed_count: StepCount) -> bool {
        self.allows_next_after(completed_count)
    }

    fn limit_error(
        self,
        completed_count: StepCount,
        state_len: RuntimeStateByteCount,
    ) -> LimitError {
        LimitError::step(self, completed_count, state_len)
    }
}

impl ReservationLimit<RuleAttemptCount> for RuleAttemptLimit {
    fn allows_next_after_count(self, completed_count: RuleAttemptCount) -> bool {
        self.allows_next_after(completed_count)
    }

    fn limit_error(
        self,
        completed_count: RuleAttemptCount,
        state_len: RuntimeStateByteCount,
    ) -> LimitError {
        LimitError::rule_attempt(self, completed_count, state_len)
    }
}

impl ReservableCount for StepCount {
    fn checked_next_count(self) -> Option<Self> {
        self.checked_next()
    }
}

impl ReservableCount for RuleAttemptCount {
    fn checked_next_count(self) -> Option<Self> {
        self.checked_next()
    }
}

/// Checks a runtime byte budget while preserving the concrete limit domain.
///
/// # Errors
///
/// Returns `RunError` if the measured byte count is outside the supplied budget.
fn ensure_runtime_byte_limit<Limit, Count>(
    limit: Limit,
    attempted_len: Count,
) -> Result<(), RunError>
where
    Limit: RuntimeByteLimit<Count>,
    Count: Copy,
{
    if limit.accepts_count(attempted_len) {
        return Ok(());
    }

    Err(limit.limit_error(attempted_len).into())
}

/// Reserves the next monotonic runtime count.
///
/// # Errors
///
/// Returns `RunError` if the supplied limit is exhausted or the next count cannot
/// be represented.
fn reserve_next_count<Limit, Count>(
    limit: Limit,
    completed_count: Count,
    state_len: RuntimeStateByteCount,
) -> Result<Count, RunError>
where
    Limit: ReservationLimit<Count>,
    Count: ReservableCount,
{
    if !limit.allows_next_after_count(completed_count) {
        return Err(limit.limit_error(completed_count, state_len).into());
    }

    completed_count
        .checked_next_count()
        .ok_or_else(|| limit.limit_error(completed_count, state_len).into())
}
