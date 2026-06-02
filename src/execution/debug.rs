use crate::policy::{ExecutionPolicy, ParsePolicy};

use super::session::BorrowedRunSession;
use super::transition::{
    BorrowedAppliedStep, BorrowedFailedRun, BorrowedReturnedRun, BorrowedStableRun,
    BorrowedStepTransition,
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

impl<P: ParsePolicy> core::fmt::Debug for BorrowedStableRun<'_, P> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedStableRun")
            .field("steps", &self.steps())
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
