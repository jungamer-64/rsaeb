use rsaeb::input::{RunSeed, RuntimeInput, RuntimeInputSource};
use rsaeb::limits::{
    ExecutionLimits, ReturnByteLimit, RuntimeInputByteLimit, RuntimeInputLimits,
    RuntimeStateByteLimit, StepLimit,
};

use crate::support::TestFailure;

#[derive(Clone, Copy)]
pub struct TestRunPolicy {
    input: RuntimeInputLimits,
    execution: ExecutionLimits,
}

impl TestRunPolicy {
    #[must_use]
    pub const fn new(
        max_input_len: RuntimeInputByteLimit,
        max_steps: StepLimit,
        max_state_len: RuntimeStateByteLimit,
        max_return_len: ReturnByteLimit,
    ) -> Self {
        Self {
            input: RuntimeInputLimits::new(max_input_len),
            execution: ExecutionLimits::new(max_steps, max_state_len, max_return_len),
        }
    }

    #[must_use]
    const fn input(self) -> RuntimeInputLimits {
        self.input
    }

    #[must_use]
    const fn execution(self) -> ExecutionLimits {
        self.execution
    }
}

/// Validates and admits test input into a run seed.
///
/// # Errors
///
/// Returns `TestFailure` if validation or run admission fails.
pub fn run_seed(bytes: &[u8], policy: TestRunPolicy) -> Result<RunSeed, TestFailure> {
    let input = RuntimeInput::validate(RuntimeInputSource::from_bytes(bytes), policy.input())?;
    Ok(RunSeed::admit(input, policy.execution())?)
}
