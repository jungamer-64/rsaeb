#![expect(
    dead_code,
    reason = "shared integration-test policy helpers are compiled per test crate"
)]

use core::marker::PhantomData;

use rsaeb::input::{AdmittedRun, RuntimeInput, RuntimeInputSource};
use rsaeb::limits::{ReturnByteLimit, RuntimeInputByteLimit, RuntimeStateByteLimit, StepLimit};
use rsaeb::policy::{
    DefaultExecutionPolicy, DefaultRuntimeInputPolicy, ExecutionPolicy, RuntimeInputPolicy,
    StaticExecutionPolicy, StaticRuntimeInputPolicy,
};

use crate::support::TestFailure;

pub const DEFAULT_BYTE_BUDGET: usize = 16_777_216;
pub const DEFAULT_COUNT_BUDGET: usize = 1_000_000;
pub type TestInputPolicy<const INPUT_BYTES: usize> = StaticRuntimeInputPolicy<INPUT_BYTES>;
pub type TestExecutionPolicy<
    const STEPS: usize,
    const STATE_BYTES: usize,
    const RETURN_BYTES: usize,
> = StaticExecutionPolicy<STEPS, STATE_BYTES, RETURN_BYTES>;
pub type StaticTestRunPolicy<
    const INPUT_BYTES: usize,
    const STEPS: usize,
    const STATE_BYTES: usize,
    const RETURN_BYTES: usize,
> = TestRunPolicy<
    TestInputPolicy<INPUT_BYTES>,
    TestExecutionPolicy<STEPS, STATE_BYTES, RETURN_BYTES>,
>;
pub type DefaultInputRunPolicy<
    const STEPS: usize,
    const STATE_BYTES: usize,
    const RETURN_BYTES: usize,
> = TestRunPolicy<DefaultRuntimeInputPolicy, TestExecutionPolicy<STEPS, STATE_BYTES, RETURN_BYTES>>;
pub type DefaultExecutionRunPolicy<const INPUT_BYTES: usize> =
    TestRunPolicy<TestInputPolicy<INPUT_BYTES>, DefaultExecutionPolicy>;
pub type DefaultRunPolicy = TestRunPolicy<DefaultRuntimeInputPolicy, DefaultExecutionPolicy>;

pub struct TestRunPolicy<
    I: RuntimeInputPolicy = DefaultRuntimeInputPolicy,
    E: ExecutionPolicy = DefaultExecutionPolicy,
> {
    policy: PhantomData<(I, E)>,
}

impl<I: RuntimeInputPolicy, E: ExecutionPolicy> Clone for TestRunPolicy<I, E> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<I: RuntimeInputPolicy, E: ExecutionPolicy> Copy for TestRunPolicy<I, E> {}

impl<I: RuntimeInputPolicy, E: ExecutionPolicy> TestRunPolicy<I, E> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            policy: PhantomData,
        }
    }

    #[must_use]
    pub const fn input_limit(self) -> RuntimeInputByteLimit {
        I::INPUT_BYTE_LIMIT
    }

    #[must_use]
    pub const fn step_limit(self) -> StepLimit {
        E::STEP_LIMIT
    }

    #[must_use]
    pub const fn state_limit(self) -> RuntimeStateByteLimit {
        E::STATE_BYTE_LIMIT
    }

    #[must_use]
    pub const fn return_limit(self) -> ReturnByteLimit {
        E::RETURN_BYTE_LIMIT
    }
}

/// Validates and admits test input into an execution witness.
///
/// # Errors
///
/// Returns `TestFailure` if validation or run admission fails.
pub fn admitted_run<I: RuntimeInputPolicy, E: ExecutionPolicy>(
    bytes: &[u8],
    _policy: TestRunPolicy<I, E>,
) -> Result<AdmittedRun<E>, TestFailure> {
    let input = RuntimeInput::<I>::validate(RuntimeInputSource::from_bytes(bytes))?;
    Ok(input.admit::<E>()?)
}
