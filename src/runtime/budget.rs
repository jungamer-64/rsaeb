use crate::bytes::RuntimeStateByteCount;
use crate::error::LimitError;
use crate::program::{StepCount, StepLimit};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct StepBudget {
    max_steps: StepLimit,
    completed_steps: StepCount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct StepPermit {
    next_step: StepCount,
}

impl StepBudget {
    pub(super) const fn new(max_steps: StepLimit) -> Self {
        Self {
            max_steps,
            completed_steps: StepCount::ZERO,
        }
    }

    pub(super) const fn completed_steps(self) -> StepCount {
        self.completed_steps
    }

    fn ensure_next_step_allowed(self, state_len: RuntimeStateByteCount) -> Result<(), LimitError> {
        if self.completed_steps.get() >= self.max_steps.get() {
            return Err(LimitError::step(
                self.max_steps,
                self.completed_steps,
                state_len,
            ));
        }

        Ok(())
    }

    pub(super) fn reserve_next_step(
        self,
        state_len: RuntimeStateByteCount,
    ) -> Result<StepPermit, LimitError> {
        self.ensure_next_step_allowed(state_len)?;

        let Some(next_step) = self.completed_steps.checked_next() else {
            return Err(LimitError::step(
                self.max_steps,
                self.completed_steps,
                state_len,
            ));
        };

        Ok(StepPermit { next_step })
    }

    pub(super) fn commit(&mut self, permit: StepPermit) -> StepCount {
        self.completed_steps = permit.next_step;
        permit.next_step
    }
}
