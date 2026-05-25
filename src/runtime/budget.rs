use crate::bytes::{ReturnOutputByteCount, RuntimeStateByteCount};
use crate::error::{LimitError, RunError, RunInvariantError};
use crate::limits::{ExecutionLimits, RuleAttemptCount, RuleAttemptLimit, StepCount};

/// Execution budgets plus the number of committed execution steps.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RuntimeBudgetState {
    /// Host execution policy admitted for this run.
    limits: ExecutionLimits,
    /// Step progress and any outstanding reservation.
    progress: RuntimeBudgetProgress,
}

/// Rule-attempt budget plus the number of consumed executable rule-line attempts.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RuleAttemptBudgetState {
    /// Host rule-attempt policy admitted for this run.
    limit: RuleAttemptLimit,
    /// Rule-attempt progress and any outstanding reservation.
    progress: RuleAttemptBudgetProgress,
}

/// Runtime step budget state.
#[derive(Debug, PartialEq, Eq)]
enum RuntimeBudgetProgress {
    /// No step reservation is outstanding.
    Ready {
        /// Steps committed so far.
        completed_steps: StepCount,
    },
    /// A candidate step has reserved its commit count.
    Reserved {
        /// Steps committed before the reservation.
        completed_steps: StepCount,
        /// Step count that the outstanding permit may publish.
        reserved_step: StepCount,
    },
}

/// Rule-attempt budget state.
#[derive(Debug, PartialEq, Eq)]
enum RuleAttemptBudgetProgress {
    /// No attempt reservation is outstanding.
    Ready {
        /// Rule attempts consumed so far.
        completed_attempts: RuleAttemptCount,
    },
    /// A candidate rule attempt has reserved its commit count.
    Reserved {
        /// Rule attempts consumed before the reservation.
        completed_attempts: RuleAttemptCount,
        /// Attempt count that the outstanding permit may publish.
        reserved_attempt: RuleAttemptCount,
    },
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
            progress: RuntimeBudgetProgress::Ready {
                completed_steps: StepCount::ZERO,
            },
        }
    }

    /// Number of execution steps committed so far.
    pub(crate) const fn completed_steps(&self) -> StepCount {
        match self.progress {
            RuntimeBudgetProgress::Ready { completed_steps }
            | RuntimeBudgetProgress::Reserved {
                completed_steps, ..
            } => completed_steps,
        }
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
    ) -> Result<StepPermit, RunError> {
        let RuntimeBudgetProgress::Ready { completed_steps } = self.progress else {
            return Err(RunInvariantError::BudgetReservation.into());
        };

        self.ensure_next_step_allowed(completed_steps, state_len)?;

        let Some(next_step) = completed_steps.checked_next() else {
            return Err(
                LimitError::step(self.limits.step_limit(), completed_steps, state_len).into(),
            );
        };

        self.progress = RuntimeBudgetProgress::Reserved {
            completed_steps,
            reserved_step: next_step,
        };
        Ok(StepPermit { next_step })
    }

    /// Publishes a reserved step after the matched rule commits.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if no step is currently reserved or the supplied
    /// permit does not match the active reservation.
    pub(crate) fn commit(&mut self, permit: StepPermit) -> Result<StepCount, RunError> {
        match self.progress {
            RuntimeBudgetProgress::Ready { .. } => Err(RunInvariantError::BudgetReservation.into()),
            RuntimeBudgetProgress::Reserved { reserved_step, .. }
                if reserved_step != permit.next_step =>
            {
                Err(RunInvariantError::BudgetReservation.into())
            }
            RuntimeBudgetProgress::Reserved { .. } => {
                self.progress = RuntimeBudgetProgress::Ready {
                    completed_steps: permit.next_step,
                };
                Ok(permit.next_step)
            }
        }
    }
}

impl RuleAttemptBudgetState {
    /// Starts rule-attempt budget tracking for a newly admitted rule-attempt run.
    pub(crate) const fn new(limit: RuleAttemptLimit) -> Self {
        Self {
            limit,
            progress: RuleAttemptBudgetProgress::Ready {
                completed_attempts: RuleAttemptCount::ZERO,
            },
        }
    }

    /// Number of executable rule-line attempts consumed so far.
    pub(crate) const fn completed_attempts(&self) -> RuleAttemptCount {
        match self.progress {
            RuleAttemptBudgetProgress::Ready { completed_attempts }
            | RuleAttemptBudgetProgress::Reserved {
                completed_attempts, ..
            } => completed_attempts,
        }
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
    ) -> Result<RuleAttemptPermit, RunError> {
        let RuleAttemptBudgetProgress::Ready { completed_attempts } = self.progress else {
            return Err(RunInvariantError::BudgetReservation.into());
        };

        self.ensure_next_attempt_allowed(completed_attempts, state_len)?;

        let Some(next_attempt) = completed_attempts.checked_next() else {
            return Err(LimitError::rule_attempt(self.limit, completed_attempts, state_len).into());
        };

        self.progress = RuleAttemptBudgetProgress::Reserved {
            completed_attempts,
            reserved_attempt: next_attempt,
        };
        Ok(RuleAttemptPermit { next_attempt })
    }

    /// Publishes a reserved rule-attempt count after evaluating the rule line.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if no rule attempt is currently reserved or the
    /// supplied permit does not match the active reservation.
    pub(crate) fn commit(
        &mut self,
        permit: RuleAttemptPermit,
    ) -> Result<RuleAttemptCount, RunError> {
        match self.progress {
            RuleAttemptBudgetProgress::Ready { .. } => {
                Err(RunInvariantError::BudgetReservation.into())
            }
            RuleAttemptBudgetProgress::Reserved {
                reserved_attempt, ..
            } if reserved_attempt != permit.next_attempt => {
                Err(RunInvariantError::BudgetReservation.into())
            }
            RuleAttemptBudgetProgress::Reserved { .. } => {
                self.progress = RuleAttemptBudgetProgress::Ready {
                    completed_attempts: permit.next_attempt,
                };
                Ok(permit.next_attempt)
            }
        }
    }
}
