use crate::policy::{ExecutionPolicy, RuleAttemptPolicy};

use super::session::{
    BorrowedContinuingRuleAttemptSession, BorrowedFinalRuleAttemptSession,
    BorrowedRuleAttemptCursor, BorrowedRunSession,
};
use super::transition::{
    BorrowedAlwaysReturnRun, BorrowedAlwaysRewriteStep, BorrowedContinuingRuleAttemptTransition,
    BorrowedFailedRun, BorrowedFinalRuleAttemptTransition, BorrowedOnceReturnRun,
    BorrowedOnceRewriteStep, BorrowedRuleAttemptAlwaysReturnRun,
    BorrowedRuleAttemptAlwaysRewriteStep, BorrowedRuleAttemptFailedRun,
    BorrowedRuleAttemptOnceReturnRun, BorrowedRuleAttemptOnceRewriteStep, BorrowedStableRun,
    BorrowedStepTransition,
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
            Self::AlwaysRewritten(applied) => formatter
                .debug_tuple("AlwaysRewritten")
                .field(applied)
                .finish(),
            Self::OnceRewritten(applied) => formatter
                .debug_tuple("OnceRewritten")
                .field(applied)
                .finish(),
            Self::Stable(stable) => formatter.debug_tuple("Stable").field(stable).finish(),
            Self::AlwaysReturned(returned) => formatter
                .debug_tuple("AlwaysReturned")
                .field(returned)
                .finish(),
            Self::OnceReturned(returned) => formatter
                .debug_tuple("OnceReturned")
                .field(returned)
                .finish(),
            Self::Failed(failed) => formatter.debug_tuple("Failed").field(failed).finish(),
        }
    }
}

impl<E: ExecutionPolicy, A: RuleAttemptPolicy> core::fmt::Debug
    for BorrowedContinuingRuleAttemptTransition<'_, E, A>
{
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::AlwaysRewritten(applied) => formatter
                .debug_tuple("AlwaysRewritten")
                .field(applied)
                .finish(),
            Self::OnceRewritten(applied) => formatter
                .debug_tuple("OnceRewritten")
                .field(applied)
                .finish(),
            Self::AlwaysReturned(returned) => formatter
                .debug_tuple("AlwaysReturned")
                .field(returned)
                .finish(),
            Self::OnceReturned(returned) => formatter
                .debug_tuple("OnceReturned")
                .field(returned)
                .finish(),
            Self::Failed(failed) => formatter.debug_tuple("Failed").field(failed).finish(),
        }
    }
}

impl<E: ExecutionPolicy, A: RuleAttemptPolicy> core::fmt::Debug
    for BorrowedFinalRuleAttemptTransition<'_, E, A>
{
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::AlwaysRewritten(applied) => formatter
                .debug_tuple("AlwaysRewritten")
                .field(applied)
                .finish(),
            Self::OnceRewritten(applied) => formatter
                .debug_tuple("OnceRewritten")
                .field(applied)
                .finish(),
            Self::AlwaysReturned(returned) => formatter
                .debug_tuple("AlwaysReturned")
                .field(returned)
                .finish(),
            Self::OnceReturned(returned) => formatter
                .debug_tuple("OnceReturned")
                .field(returned)
                .finish(),
            Self::Failed(failed) => formatter.debug_tuple("Failed").field(failed).finish(),
        }
    }
}

/// Implements debug output for borrowed rewrite step witnesses.
macro_rules! impl_rewrite_step_debug {
    ($step:ident, $name:literal) => {
        impl<E: ExecutionPolicy> core::fmt::Debug for $step<'_, E> {
            fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                formatter
                    .debug_struct($name)
                    .field("step", &self.step())
                    .field("rule", &self.rule())
                    .field("state", &self.state())
                    .finish()
            }
        }
    };
}

impl_rewrite_step_debug!(BorrowedAlwaysRewriteStep, "BorrowedAlwaysRewriteStep");
impl_rewrite_step_debug!(BorrowedOnceRewriteStep, "BorrowedOnceRewriteStep");

impl core::fmt::Debug for BorrowedStableRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedStableRun")
            .field("steps", &self.steps())
            .field("state", &self.state())
            .finish()
    }
}

/// Implements debug output for borrowed rule-attempt rewrite witnesses.
macro_rules! impl_rule_attempt_rewrite_step_debug {
    ($step:ident, $name:literal) => {
        impl<E: ExecutionPolicy, A: RuleAttemptPolicy> core::fmt::Debug for $step<'_, E, A> {
            fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                formatter
                    .debug_struct($name)
                    .field("attempt", &self.attempt())
                    .field("step", &self.step())
                    .field("rule", &self.rule())
                    .field("state", &self.state())
                    .finish()
            }
        }
    };
}

impl_rule_attempt_rewrite_step_debug!(
    BorrowedRuleAttemptAlwaysRewriteStep,
    "BorrowedRuleAttemptAlwaysRewriteStep"
);
impl_rule_attempt_rewrite_step_debug!(
    BorrowedRuleAttemptOnceRewriteStep,
    "BorrowedRuleAttemptOnceRewriteStep"
);

/// Implements debug output for borrowed return terminal witnesses.
macro_rules! impl_return_run_debug {
    ($run:ident, $name:literal) => {
        impl core::fmt::Debug for $run<'_> {
            fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                formatter
                    .debug_struct($name)
                    .field("step", &self.step())
                    .field("rule", &self.rule())
                    .field("output", &self.output())
                    .finish()
            }
        }
    };
}

/// Implements debug output for borrowed rule-attempt return terminal witnesses.
macro_rules! impl_rule_attempt_return_run_debug {
    ($run:ident, $name:literal) => {
        impl core::fmt::Debug for $run<'_> {
            fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                formatter
                    .debug_struct($name)
                    .field("attempt", &self.attempt())
                    .field("step", &self.step())
                    .field("rule", &self.rule())
                    .field("output", &self.output())
                    .finish()
            }
        }
    };
}

impl_return_run_debug!(BorrowedAlwaysReturnRun, "BorrowedAlwaysReturnRun");
impl_return_run_debug!(BorrowedOnceReturnRun, "BorrowedOnceReturnRun");
impl_rule_attempt_return_run_debug!(
    BorrowedRuleAttemptAlwaysReturnRun,
    "BorrowedRuleAttemptAlwaysReturnRun"
);
impl_rule_attempt_return_run_debug!(
    BorrowedRuleAttemptOnceReturnRun,
    "BorrowedRuleAttemptOnceReturnRun"
);

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
