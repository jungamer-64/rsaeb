use crate::policy::{ExecutionPolicy, RuleAttemptPolicy};

use super::session::{
    BorrowedContinuingRuleAttemptSession, BorrowedFinalRuleAttemptSession,
    BorrowedRuleAttemptCursor, BorrowedRunSession,
};
use super::transition::{
    BorrowedAlwaysReturnRun, BorrowedAlwaysReturnStateMismatchRuleAttempt,
    BorrowedAlwaysRewriteStateMismatchRuleAttempt, BorrowedAlwaysRewriteStep,
    BorrowedContinuingRuleAttemptTransition, BorrowedFailedRun, BorrowedFinalRuleAttemptTransition,
    BorrowedOnceReturnRun,
    BorrowedOnceReturnStateMismatchRuleAttempt, BorrowedOnceRewriteConsumedRuleAttempt,
    BorrowedOnceRewriteStateMismatchRuleAttempt, BorrowedOnceRewriteStep,
    BorrowedRuleAttemptAlwaysReturnRun, BorrowedRuleAttemptAlwaysRewriteStep,
    BorrowedRuleAttemptFailedRun, BorrowedRuleAttemptOnceReturnRun,
    BorrowedRuleAttemptOnceRewriteStep, BorrowedRuleAttemptStableAfterAlwaysReturnStateMismatch,
    BorrowedRuleAttemptStableAfterAlwaysRewriteStateMismatch,
    BorrowedRuleAttemptStableAfterOnceReturnStateMismatch,
    BorrowedRuleAttemptStableAfterOnceRewriteConsumed,
    BorrowedRuleAttemptStableAfterOnceRewriteStateMismatch, BorrowedStableRun,
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
            Self::AlwaysRewriteStateMismatch(missed) => formatter
                .debug_tuple("AlwaysRewriteStateMismatch")
                .field(missed)
                .finish(),
            Self::OnceRewriteStateMismatch(missed) => formatter
                .debug_tuple("OnceRewriteStateMismatch")
                .field(missed)
                .finish(),
            Self::AlwaysReturnStateMismatch(missed) => formatter
                .debug_tuple("AlwaysReturnStateMismatch")
                .field(missed)
                .finish(),
            Self::OnceReturnStateMismatch(missed) => formatter
                .debug_tuple("OnceReturnStateMismatch")
                .field(missed)
                .finish(),
            Self::OnceRewriteConsumed(missed) => formatter
                .debug_tuple("OnceRewriteConsumed")
                .field(missed)
                .finish(),
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
            Self::StableAfterAlwaysRewriteStateMismatch(stable) => formatter
                .debug_tuple("StableAfterAlwaysRewriteStateMismatch")
                .field(stable)
                .finish(),
            Self::StableAfterOnceRewriteStateMismatch(stable) => formatter
                .debug_tuple("StableAfterOnceRewriteStateMismatch")
                .field(stable)
                .finish(),
            Self::StableAfterAlwaysReturnStateMismatch(stable) => formatter
                .debug_tuple("StableAfterAlwaysReturnStateMismatch")
                .field(stable)
                .finish(),
            Self::StableAfterOnceReturnStateMismatch(stable) => formatter
                .debug_tuple("StableAfterOnceReturnStateMismatch")
                .field(stable)
                .finish(),
            Self::StableAfterOnceRewriteConsumed(stable) => formatter
                .debug_tuple("StableAfterOnceRewriteConsumed")
                .field(stable)
                .finish(),
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

/// Implements debug output for exact continuing rule-attempt misses.
macro_rules! impl_rule_attempt_miss_debug {
    ($miss:ident, $name:literal) => {
        impl<E: ExecutionPolicy, A: RuleAttemptPolicy> core::fmt::Debug for $miss<'_, E, A> {
            fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                formatter
                    .debug_struct($name)
                    .field("attempt", &self.attempt())
                    .field("rule", &self.rule())
                    .field("state", &self.state())
                    .finish()
            }
        }
    };
}

impl_rule_attempt_miss_debug!(
    BorrowedAlwaysRewriteStateMismatchRuleAttempt,
    "BorrowedAlwaysRewriteStateMismatchRuleAttempt"
);
impl_rule_attempt_miss_debug!(
    BorrowedOnceRewriteStateMismatchRuleAttempt,
    "BorrowedOnceRewriteStateMismatchRuleAttempt"
);
impl_rule_attempt_miss_debug!(
    BorrowedAlwaysReturnStateMismatchRuleAttempt,
    "BorrowedAlwaysReturnStateMismatchRuleAttempt"
);
impl_rule_attempt_miss_debug!(
    BorrowedOnceReturnStateMismatchRuleAttempt,
    "BorrowedOnceReturnStateMismatchRuleAttempt"
);
impl_rule_attempt_miss_debug!(
    BorrowedOnceRewriteConsumedRuleAttempt,
    "BorrowedOnceRewriteConsumedRuleAttempt"
);

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

/// Implements debug output for exact stable rule-attempt terminals.
macro_rules! impl_rule_attempt_stable_miss_debug {
    ($run:ident, $name:literal) => {
        impl core::fmt::Debug for $run<'_> {
            fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                formatter
                    .debug_struct($name)
                    .field("attempts", &self.attempts())
                    .field("steps", &self.steps())
                    .field("rule", &self.rule())
                    .field("state", &self.state())
                    .finish()
            }
        }
    };
}

impl_rule_attempt_stable_miss_debug!(
    BorrowedRuleAttemptStableAfterAlwaysRewriteStateMismatch,
    "BorrowedRuleAttemptStableAfterAlwaysRewriteStateMismatch"
);
impl_rule_attempt_stable_miss_debug!(
    BorrowedRuleAttemptStableAfterOnceRewriteStateMismatch,
    "BorrowedRuleAttemptStableAfterOnceRewriteStateMismatch"
);
impl_rule_attempt_stable_miss_debug!(
    BorrowedRuleAttemptStableAfterAlwaysReturnStateMismatch,
    "BorrowedRuleAttemptStableAfterAlwaysReturnStateMismatch"
);
impl_rule_attempt_stable_miss_debug!(
    BorrowedRuleAttemptStableAfterOnceReturnStateMismatch,
    "BorrowedRuleAttemptStableAfterOnceReturnStateMismatch"
);
impl_rule_attempt_stable_miss_debug!(
    BorrowedRuleAttemptStableAfterOnceRewriteConsumed,
    "BorrowedRuleAttemptStableAfterOnceRewriteConsumed"
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
