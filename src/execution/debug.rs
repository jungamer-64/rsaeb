use crate::policy::{ExecutionPolicy, ParsePolicy, RuleAttemptPolicy};

use super::session::{BorrowedRuleAttemptSession, BorrowedRunSession, OwnedRunSession};
use super::transition::{
    BorrowedAppliedStep, BorrowedFailedRun, BorrowedMissedRuleAttempt, BorrowedReturnedRun,
    BorrowedRuleAttemptAppliedStep, BorrowedRuleAttemptFailedRun, BorrowedRuleAttemptReturnedRun,
    BorrowedRuleAttemptStableRun, BorrowedRuleAttemptTransition, BorrowedStableRun,
    BorrowedStepTransition, OwnedAppliedStep, OwnedFailedRun, OwnedReturnedRun, OwnedStableRun,
    OwnedStepTransition,
};

impl<P: ParsePolicy, E: ExecutionPolicy> core::fmt::Debug for BorrowedRunSession<'_, P, E> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedRunSession")
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> core::fmt::Debug for OwnedRunSession<P, E> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedRunSession")
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy> core::fmt::Debug
    for BorrowedRuleAttemptSession<'_, P, E, A>
{
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedRuleAttemptSession")
            .field("completed_attempts", &self.completed_attempts())
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> core::fmt::Debug for BorrowedStepTransition<'_, P, E> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Applied(applied) => formatter.debug_tuple("Applied").field(applied).finish(),
            Self::Stable(stable) => formatter.debug_tuple("Stable").field(stable).finish(),
            Self::Returned(returned) => formatter.debug_tuple("Returned").field(returned).finish(),
            Self::Failed(failed) => formatter.debug_tuple("Failed").field(failed).finish(),
        }
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> core::fmt::Debug for OwnedStepTransition<P, E> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Applied(applied) => formatter.debug_tuple("Applied").field(applied).finish(),
            Self::Stable(stable) => formatter.debug_tuple("Stable").field(stable).finish(),
            Self::Returned(returned) => formatter.debug_tuple("Returned").field(returned).finish(),
            Self::Failed(failed) => formatter.debug_tuple("Failed").field(failed).finish(),
        }
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy> core::fmt::Debug
    for BorrowedRuleAttemptTransition<'_, P, E, A>
{
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Missed(missed) => formatter.debug_tuple("Missed").field(missed).finish(),
            Self::Applied(applied) => formatter.debug_tuple("Applied").field(applied).finish(),
            Self::Stable(stable) => formatter.debug_tuple("Stable").field(stable).finish(),
            Self::Returned(returned) => formatter.debug_tuple("Returned").field(returned).finish(),
            Self::Failed(failed) => formatter.debug_tuple("Failed").field(failed).finish(),
        }
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> core::fmt::Debug for BorrowedAppliedStep<'_, P, E> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedAppliedStep")
            .field("step", &self.step())
            .field("rule", &self.rule())
            .field("state", &self.state())
            .finish()
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> core::fmt::Debug for OwnedAppliedStep<P, E> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedAppliedStep")
            .field("step", &self.step())
            .field("rule", &self.rule())
            .field("state", &self.state())
            .finish()
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy> core::fmt::Debug
    for BorrowedMissedRuleAttempt<'_, P, E, A>
{
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedMissedRuleAttempt")
            .field("attempt", &self.attempt())
            .field("miss", &self.miss())
            .field("state", &self.state())
            .finish()
    }
}

impl<P: ParsePolicy> core::fmt::Debug for BorrowedStableRun<'_, P> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedStableRun")
            .field("steps", &self.steps())
            .field("state", &self.state())
            .finish()
    }
}

impl<P: ParsePolicy> core::fmt::Debug for OwnedStableRun<P> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedStableRun")
            .field("steps", &self.steps())
            .field("state", &self.state())
            .finish()
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy> core::fmt::Debug
    for BorrowedRuleAttemptAppliedStep<'_, P, E, A>
{
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedRuleAttemptAppliedStep")
            .field("attempt", &self.attempt())
            .field("step", &self.step())
            .field("rule", &self.rule())
            .field("state", &self.state())
            .finish()
    }
}

impl<P: ParsePolicy> core::fmt::Debug for BorrowedRuleAttemptStableRun<'_, P> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedRuleAttemptStableRun")
            .field("attempts", &self.attempts())
            .field("steps", &self.steps())
            .field("final_miss", &self.final_miss())
            .field("state", &self.state())
            .finish()
    }
}

impl<P: ParsePolicy> core::fmt::Debug for BorrowedReturnedRun<'_, P> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedReturnedRun")
            .field("step", &self.step())
            .field("rule", &self.rule())
            .field("output", &self.output())
            .finish()
    }
}

impl<P: ParsePolicy> core::fmt::Debug for OwnedReturnedRun<P> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedReturnedRun")
            .field("step", &self.step())
            .field("rule", &self.rule())
            .field("output", &self.output())
            .finish()
    }
}

impl<P: ParsePolicy> core::fmt::Debug for BorrowedRuleAttemptReturnedRun<'_, P> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedRuleAttemptReturnedRun")
            .field("attempt", &self.attempt())
            .field("step", &self.step())
            .field("rule", &self.rule())
            .field("output", &self.output())
            .finish()
    }
}

impl<P: ParsePolicy> core::fmt::Debug for BorrowedFailedRun<'_, P> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedFailedRun")
            .field("error", &self.error())
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl<P: ParsePolicy> core::fmt::Debug for OwnedFailedRun<P> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedFailedRun")
            .field("error", &self.error())
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl<P: ParsePolicy> core::fmt::Debug for BorrowedRuleAttemptFailedRun<'_, P> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedRuleAttemptFailedRun")
            .field("error", &self.error())
            .field("completed_attempts", &self.completed_attempts())
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}
