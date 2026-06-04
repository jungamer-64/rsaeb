use crate::policy::{ExecutionPolicy, RuleAttemptPolicy};

use super::session::{
    BorrowedContinuingRuleAttemptSession, BorrowedFinalRuleAttemptSession,
    BorrowedRuleAttemptCursor, BorrowedRunSession,
};
use super::transition::{
    BorrowedAppliedStep, BorrowedContinuingRuleAttemptTransition, BorrowedFailedRun,
    BorrowedFinalRuleAttemptTransition, BorrowedMissedRuleAttempt, BorrowedReturnedRun,
    BorrowedRuleAttemptAppliedStep, BorrowedRuleAttemptFailedRun, BorrowedRuleAttemptReturnedRun,
    BorrowedRuleAttemptStableRun, BorrowedStableRun, BorrowedStepTransition,
};

impl<E: ExecutionPolicy> core::fmt::Debug for BorrowedRunSession<'_, E> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedRunSession")
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl<E: ExecutionPolicy, A: RuleAttemptPolicy> core::fmt::Debug
    for BorrowedRuleAttemptCursor<'_, E, A>
{
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Continuing(session) => {
                formatter.debug_tuple("Continuing").field(session).finish()
            }
            Self::Final(session) => formatter.debug_tuple("Final").field(session).finish(),
        }
    }
}

impl<E: ExecutionPolicy, A: RuleAttemptPolicy> core::fmt::Debug
    for BorrowedContinuingRuleAttemptSession<'_, E, A>
{
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedContinuingRuleAttemptSession")
            .field("completed_attempts", &self.completed_attempts())
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl<E: ExecutionPolicy, A: RuleAttemptPolicy> core::fmt::Debug
    for BorrowedFinalRuleAttemptSession<'_, E, A>
{
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedFinalRuleAttemptSession")
            .field("completed_attempts", &self.completed_attempts())
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl<E: ExecutionPolicy> core::fmt::Debug for BorrowedStepTransition<'_, E> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Applied(applied) => formatter.debug_tuple("Applied").field(applied).finish(),
            Self::Stable(stable) => formatter.debug_tuple("Stable").field(stable).finish(),
            Self::Returned(returned) => formatter.debug_tuple("Returned").field(returned).finish(),
            Self::Failed(failed) => formatter.debug_tuple("Failed").field(failed).finish(),
        }
    }
}

impl<E: ExecutionPolicy, A: RuleAttemptPolicy> core::fmt::Debug
    for BorrowedContinuingRuleAttemptTransition<'_, E, A>
{
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Missed(missed) => formatter.debug_tuple("Missed").field(missed).finish(),
            Self::Applied(applied) => formatter.debug_tuple("Applied").field(applied).finish(),
            Self::Returned(returned) => formatter.debug_tuple("Returned").field(returned).finish(),
            Self::Failed(failed) => formatter.debug_tuple("Failed").field(failed).finish(),
        }
    }
}

impl<E: ExecutionPolicy, A: RuleAttemptPolicy> core::fmt::Debug
    for BorrowedFinalRuleAttemptTransition<'_, E, A>
{
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Stable(stable) => formatter.debug_tuple("Stable").field(stable).finish(),
            Self::Applied(applied) => formatter.debug_tuple("Applied").field(applied).finish(),
            Self::Returned(returned) => formatter.debug_tuple("Returned").field(returned).finish(),
            Self::Failed(failed) => formatter.debug_tuple("Failed").field(failed).finish(),
        }
    }
}

impl<E: ExecutionPolicy> core::fmt::Debug for BorrowedAppliedStep<'_, E> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedAppliedStep")
            .field("step", &self.step())
            .field("rule", &self.rule())
            .field("state", &self.state())
            .finish()
    }
}

impl<E: ExecutionPolicy, A: RuleAttemptPolicy> core::fmt::Debug
    for BorrowedMissedRuleAttempt<'_, E, A>
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

impl core::fmt::Debug for BorrowedStableRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedStableRun")
            .field("steps", &self.steps())
            .field("state", &self.state())
            .finish()
    }
}

impl<E: ExecutionPolicy, A: RuleAttemptPolicy> core::fmt::Debug
    for BorrowedRuleAttemptAppliedStep<'_, E, A>
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

impl core::fmt::Debug for BorrowedRuleAttemptStableRun<'_> {
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

impl core::fmt::Debug for BorrowedReturnedRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedReturnedRun")
            .field("step", &self.step())
            .field("rule", &self.rule())
            .field("output", &self.output())
            .finish()
    }
}

impl core::fmt::Debug for BorrowedRuleAttemptReturnedRun<'_> {
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

impl core::fmt::Debug for BorrowedFailedRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedFailedRun")
            .field("error", &self.error())
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for BorrowedRuleAttemptFailedRun<'_> {
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
