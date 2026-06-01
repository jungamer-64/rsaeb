use crate::bytes::{ReturnOutputByteCount, RuntimeStateByteCount};
use crate::error::{
    ReturnOutputLimitError, RuleAttemptLimitError, RuleAttemptStepError, RunStepError,
    RuntimeStateLimitError, StepLimitError,
};
use crate::limits::{RuleAttemptCount, RuleAttemptLimit, StepCount};
use crate::policy::{ExecutionPolicy, RuleAttemptPolicy};
use core::marker::PhantomData;

/// Execution budgets plus the number of committed execution steps.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RuntimeBudgetState<E: ExecutionPolicy> {
    /// Steps committed so far.
    completed_steps: StepCount,
    /// Compile-time execution policy selected for this run.
    policy: PhantomData<E>,
}

/// Rule-attempt budget plus the number of consumed executable rule-line attempts.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RuleAttemptBudgetState<A: RuleAttemptPolicy> {
    /// Rule attempts consumed so far.
    completed_attempts: RuleAttemptCount,
    /// Compile-time rule-attempt policy selected for this run.
    policy: PhantomData<A>,
}

/// Borrow-tied next step reservation.
#[derive(Debug)]
pub(crate) struct StepReservation<'budget, E: ExecutionPolicy> {
    /// Budget state borrowed until the reservation is either committed or dropped.
    budget: &'budget mut RuntimeBudgetState<E>,
    /// Step count to publish when the rule application commits.
    next_step: StepCount,
}

/// Borrow-tied next rule-attempt reservation.
#[derive(Debug)]
pub(crate) struct RuleAttemptReservation<'budget, A: RuleAttemptPolicy> {
    /// Attempt budget borrowed until the reservation is either committed or dropped.
    budget: &'budget mut RuleAttemptBudgetState<A>,
    /// Rule-attempt count to publish when the rule line is consumed.
    next_attempt: RuleAttemptCount,
}

impl<E: ExecutionPolicy> RuntimeBudgetState<E> {
    /// Starts runtime budget tracking for a newly admitted run.
    pub(crate) const fn new() -> Self {
        Self {
            completed_steps: StepCount::ZERO,
            policy: PhantomData,
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
    /// Returns `RunStepError` if the rewritten state would exceed the configured
    /// runtime state limit.
    pub(crate) fn ensure_rewrite_state_len(
        attempted_len: RuntimeStateByteCount,
    ) -> Result<(), RunStepError> {
        let limit = E::STATE_BYTE_LIMIT;
        if limit.admit(attempted_len).is_some() {
            return Ok(());
        }

        Err(RuntimeStateLimitError::new(limit, attempted_len).into())
    }

    /// Checks a `(return)` payload against return-output limits.
    ///
    /// # Errors
    ///
    /// Returns `RunStepError` if the return payload exceeds the configured return
    /// output limit.
    pub(crate) fn ensure_return_len(
        attempted_len: ReturnOutputByteCount,
    ) -> Result<(), RunStepError> {
        let limit = E::RETURN_BYTE_LIMIT;
        if limit.admit(attempted_len).is_some() {
            return Ok(());
        }

        Err(ReturnOutputLimitError::new(limit, attempted_len).into())
    }

    /// Reserves the next step number before a rule commits.
    ///
    /// # Errors
    ///
    /// Returns `RunStepError` if the step limit is reached or the next step
    /// count cannot be represented.
    pub(crate) fn reserve_next_step(
        &mut self,
        state_len: RuntimeStateByteCount,
    ) -> Result<StepReservation<'_, E>, RunStepError> {
        let limit = E::STEP_LIMIT;
        let next_step = reserve_next_step(limit, self.completed_steps, state_len)?;

        Ok(StepReservation {
            budget: self,
            next_step,
        })
    }
}

impl<E: ExecutionPolicy> StepReservation<'_, E> {
    /// Publishes the reserved step count.
    pub(crate) fn commit(self) -> StepCount {
        self.budget.completed_steps = self.next_step;
        self.next_step
    }
}

impl<A: RuleAttemptPolicy> RuleAttemptBudgetState<A> {
    /// Starts rule-attempt budget tracking for a newly admitted rule-attempt run.
    pub(crate) const fn new() -> Self {
        Self {
            completed_attempts: RuleAttemptCount::ZERO,
            policy: PhantomData,
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
    /// Returns `RuleAttemptStepError` if the attempt limit is reached or the next attempt
    /// count cannot be represented.
    pub(crate) fn reserve_next_attempt(
        &mut self,
        state_len: RuntimeStateByteCount,
    ) -> Result<RuleAttemptReservation<'_, A>, RuleAttemptStepError> {
        let next_attempt =
            reserve_next_attempt(A::RULE_ATTEMPT_LIMIT, self.completed_attempts, state_len)?;

        Ok(RuleAttemptReservation {
            budget: self,
            next_attempt,
        })
    }
}

impl<A: RuleAttemptPolicy> RuleAttemptReservation<'_, A> {
    /// Publishes the reserved rule-attempt count.
    pub(crate) fn commit(self) -> RuleAttemptCount {
        self.budget.completed_attempts = self.next_attempt;
        self.next_attempt
    }
}

/// Reserves the next committed execution step.
///
/// # Errors
///
/// Returns a typed limit error if the supplied limit is exhausted or the next count cannot
/// be represented.
fn reserve_next_step(
    limit: crate::limits::StepLimit,
    completed_count: StepCount,
    state_len: RuntimeStateByteCount,
) -> Result<StepCount, RunStepError> {
    if !limit.allows_next_after(completed_count) {
        return Err(StepLimitError::new(limit, completed_count, state_len).into());
    }

    completed_count
        .checked_next()
        .ok_or_else(|| StepLimitError::new(limit, completed_count, state_len).into())
}

/// Reserves the next consumed executable rule-line attempt.
///
/// # Errors
///
/// Returns a typed limit error if the supplied limit is exhausted or the next count cannot
/// be represented.
fn reserve_next_attempt(
    limit: RuleAttemptLimit,
    completed_count: RuleAttemptCount,
    state_len: RuntimeStateByteCount,
) -> Result<RuleAttemptCount, RuleAttemptStepError> {
    if !limit.allows_next_after(completed_count) {
        return Err(RuleAttemptLimitError::new(limit, completed_count, state_len).into());
    }

    completed_count
        .checked_next()
        .ok_or_else(|| RuleAttemptLimitError::new(limit, completed_count, state_len).into())
}
