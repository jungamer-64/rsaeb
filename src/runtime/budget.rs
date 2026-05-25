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

/// Reserved next step number that becomes visible only after commit.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct StepPermit {
    /// Step count to publish when the rule application commits.
    next_step: StepCount,
}

/// Reserved next rule-attempt number that becomes visible only after commit.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RuleAttemptPermit {
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
    pub(crate) const fn completed_steps(self) -> StepCount {
        self.completed_steps
    }

    /// Checks a candidate rewrite state against runtime state limits.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if the rewritten state would exceed the configured
    /// runtime state limit.
    pub(crate) fn ensure_rewrite_state_len(
        self,
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
        self,
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
    fn ensure_next_step_allowed(self, state_len: RuntimeStateByteCount) -> Result<(), LimitError> {
        if !self
            .limits
            .step_limit()
            .allows_next_after(self.completed_steps)
        {
            return Err(LimitError::step(
                self.limits.step_limit(),
                self.completed_steps,
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
    ) -> Result<StepPermit, LimitError> {
        self.ensure_next_step_allowed(state_len)?;

        let Some(next_step) = self.completed_steps.checked_next() else {
            return Err(LimitError::step(
                self.limits.step_limit(),
                self.completed_steps,
                state_len,
            ));
        };

        Ok(StepPermit { next_step })
    }

    /// Publishes a reserved step after the matched rule commits.
    pub(crate) fn commit(&mut self, permit: StepPermit) -> StepCount {
        self.completed_steps = permit.next_step;
        permit.next_step
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
    pub(crate) const fn completed_attempts(self) -> RuleAttemptCount {
        self.completed_attempts
    }

    /// Checks whether another rule attempt can be consumed.
    ///
    /// # Errors
    ///
    /// Returns `LimitError` if the configured rule-attempt limit has already
    /// been reached.
    fn ensure_next_attempt_allowed(
        self,
        state_len: RuntimeStateByteCount,
    ) -> Result<(), LimitError> {
        if !self.limit.allows_next_after(self.completed_attempts) {
            return Err(LimitError::rule_attempt(
                self.limit,
                self.completed_attempts,
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
    ) -> Result<RuleAttemptPermit, LimitError> {
        self.ensure_next_attempt_allowed(state_len)?;

        let Some(next_attempt) = self.completed_attempts.checked_next() else {
            return Err(LimitError::rule_attempt(
                self.limit,
                self.completed_attempts,
                state_len,
            ));
        };

        Ok(RuleAttemptPermit { next_attempt })
    }

    /// Publishes a reserved rule-attempt count after evaluating the rule line.
    pub(crate) fn commit(&mut self, permit: RuleAttemptPermit) -> RuleAttemptCount {
        self.completed_attempts = permit.next_attempt;
        permit.next_attempt
    }
}
