use crate::bytes::{ReturnOutputByteCount, RuntimeStateByteCount};
use crate::error::{LimitError, RunError};
use crate::limits::{ExecutionLimits, RuleAttemptCount, RuleAttemptLimit, StepCount};

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
        if !self.limits.state_byte_limit().accepts(attempted_len) {
            return Err(LimitError::state(self.limits.state_byte_limit(), attempted_len).into());
        }

        Ok(())
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
        if !self.limits.return_byte_limit().accepts(attempted_len) {
            return Err(
                LimitError::return_output(self.limits.return_byte_limit(), attempted_len).into(),
            );
        }

        Ok(())
    }

    /// Checks whether another execution step can be attempted.
    ///
    /// # Errors
    ///
    /// Returns `LimitError` if the configured step limit has already been
    /// reached.
    fn ensure_next_step_allowed(
        &self,
        completed_steps: StepCount,
        state_len: RuntimeStateByteCount,
    ) -> Result<(), LimitError> {
        if !self.limits.step_limit().allows_next_after(completed_steps) {
            return Err(LimitError::step(
                self.limits.step_limit(),
                completed_steps,
                state_len,
            ));
        }

        Ok(())
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
        self.ensure_next_step_allowed(self.completed_steps, state_len)?;

        let Some(next_step) = self.completed_steps.checked_next() else {
            return Err(LimitError::step(
                self.limits.step_limit(),
                self.completed_steps,
                state_len,
            )
            .into());
        };

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

    /// Checks whether another rule attempt can be consumed.
    ///
    /// # Errors
    ///
    /// Returns `LimitError` if the configured rule-attempt limit has already
    /// been reached.
    fn ensure_next_attempt_allowed(
        &self,
        completed_attempts: RuleAttemptCount,
        state_len: RuntimeStateByteCount,
    ) -> Result<(), LimitError> {
        if !self.limit.allows_next_after(completed_attempts) {
            return Err(LimitError::rule_attempt(
                self.limit,
                completed_attempts,
                state_len,
            ));
        }

        Ok(())
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
        self.ensure_next_attempt_allowed(self.completed_attempts, state_len)?;

        let Some(next_attempt) = self.completed_attempts.checked_next() else {
            return Err(
                LimitError::rule_attempt(self.limit, self.completed_attempts, state_len).into(),
            );
        };

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
