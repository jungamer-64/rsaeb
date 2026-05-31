use core::marker::PhantomData;

use crate::input::RunSeed;
use crate::policy::{DefaultPolicy, ExecutionPolicy, RuleAttemptPolicy};

/// Run-start witness for rule-attempt execution.
///
/// Rule-attempt execution consumes the same admitted runtime input as ordinary
/// execution, plus a separate rule-attempt policy. Grouping them prevents
/// callers from selecting a detached policy beside an unrelated run seed.
#[derive(Debug, PartialEq, Eq)]
pub struct RuleAttemptSeed<E: ExecutionPolicy = DefaultPolicy, A: RuleAttemptPolicy = DefaultPolicy>
{
    /// Admitted runtime input and execution policy.
    seed: RunSeed<E>,
    /// Compile-time rule-attempt policy selected for this value.
    policy: PhantomData<A>,
}

impl<E: ExecutionPolicy, A: RuleAttemptPolicy> RuleAttemptSeed<E, A> {
    /// Binds one admitted run seed to a rule-attempt policy.
    #[must_use]
    pub const fn new(seed: RunSeed<E>) -> Self {
        Self {
            seed,
            policy: PhantomData,
        }
    }

    /// Splits the seed into the ordinary run seed.
    pub(crate) fn into_parts(self) -> RunSeed<E> {
        self.seed
    }
}
