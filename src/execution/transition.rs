use crate::error::{RunFinishError, RunStepError};
use crate::inspect::RuleView;
use crate::limits::StepCount;
use crate::policy::{ExecutionPolicy, ParsePolicy};
use crate::program::{ExecutableProgram, ReturnOutput, RunResult};
use crate::trace::RuntimeStateView;

use super::engine::TerminalRunCore;
use super::session::BorrowedRunSession;

/// Result of advancing a borrowed run session once.
///
/// Only [`BorrowedStepTransition::Applied`] carries a continuation session. Stable,
/// returned, and failed transitions are terminal.
pub enum BorrowedStepTransition<'program, P: ParsePolicy, E: ExecutionPolicy> {
    /// One ordinary rewrite rule was applied and execution can continue.
    Applied(BorrowedAppliedStep<'program, P, E>),
    /// No rule matched the final runtime state.
    Stable(BorrowedStableRun<'program, P>),
    /// A matched rule executed `(return)`.
    Returned(BorrowedReturnedRun<'program, P>),
    /// A matching rule failed before committing.
    Failed(BorrowedFailedRun<'program, P>),
}

/// One committed non-terminal rule application in a borrowed session.
pub struct BorrowedAppliedStep<'program, P: ParsePolicy, E: ExecutionPolicy> {
    /// Step number committed by this transition.
    pub(super) step: StepCount,
    /// Borrowed rewrite rule committed by this transition.
    pub(super) rule: RuleView<'program>,
    /// Continuation session after the committed rule application.
    pub(super) session: BorrowedRunSession<'program, P, E>,
}

/// Terminal borrowed run state reached by no matching rule.
pub struct BorrowedStableRun<'program, P: ParsePolicy> {
    /// Parsed program borrowed by the terminal state.
    pub(super) program: &'program ExecutableProgram<P>,
    /// Terminal runtime core containing the stable state.
    pub(super) core: TerminalRunCore,
}

/// Terminal borrowed run state reached by `(return)`.
pub struct BorrowedReturnedRun<'program, P: ParsePolicy> {
    /// Step number that executed the return action.
    pub(super) step: StepCount,
    /// Borrowed return rule committed by this transition.
    pub(super) rule: RuleView<'program>,
    /// Parsed program borrowed by the terminal state.
    pub(super) program: &'program ExecutableProgram<P>,
    /// Materialized return output produced by the committed return rule.
    pub(super) output: ReturnOutput,
}

/// Runtime failure that preserves uncommitted borrowed state for inspection.
pub struct BorrowedFailedRun<'program, P: ParsePolicy> {
    /// Runtime error that stopped the candidate step before commit.
    pub(super) error: RunStepError,
    /// Parsed program borrowed by the failed terminal state.
    pub(super) program: &'program ExecutableProgram<P>,
    /// Uncommitted runtime core retained for diagnostic inspection.
    pub(super) core: TerminalRunCore,
}

impl<'program, P: ParsePolicy, E: ExecutionPolicy> BorrowedAppliedStep<'program, P, E> {
    /// One-based applied step count.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Borrowed rule committed by this transition.
    #[must_use]
    pub const fn rule(&self) -> RuleView<'program> {
        self.rule
    }

    /// Runtime state after the applied step.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Continue running after observing this applied step.
    ///
    /// This is the only borrowed transition that can resume execution.
    #[must_use]
    pub fn into_session(self) -> BorrowedRunSession<'program, P, E> {
        self.session
    }
}

impl<'program, P: ParsePolicy> BorrowedStableRun<'program, P> {
    /// Number of execution steps committed before reaching the stable state.
    #[must_use]
    pub const fn steps(&self) -> StepCount {
        self.core.completed_steps()
    }

    /// Borrow the parsed program used by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &'program ExecutableProgram<P> {
        self.program
    }

    /// Borrowed final runtime state.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }

    /// Materializes this stable run as a run result.
    ///
    /// # Errors
    ///
    /// Returns `RunFinishError` if final state materialization cannot allocate.
    pub fn into_result(self) -> Result<RunResult, RunFinishError> {
        self.core.into_stable_result()
    }
}

impl<'program, P: ParsePolicy> BorrowedReturnedRun<'program, P> {
    /// One-based applied step count for the return rule.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Borrow the parsed program used by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &'program ExecutableProgram<P> {
        self.program
    }

    /// Borrowed return rule committed by this terminal state.
    #[must_use]
    pub const fn rule(&self) -> RuleView<'program> {
        self.rule
    }

    /// Materialized return output from runtime execution.
    #[must_use]
    pub const fn output(&self) -> &ReturnOutput {
        &self.output
    }

    /// Materializes this returned run as a run result.
    #[must_use]
    pub fn into_result(self) -> RunResult {
        RunResult::from_return(self.output, self.step)
    }
}

impl<'program, P: ParsePolicy> BorrowedFailedRun<'program, P> {
    /// Captures a failed borrowed session without committing the attempted step.
    pub(super) fn new(
        error: RunStepError,
        program: &'program ExecutableProgram<P>,
        core: TerminalRunCore,
    ) -> Self {
        Self {
            error,
            program,
            core,
        }
    }

    /// Runtime error that prevented the step from committing.
    #[must_use]
    pub const fn error(&self) -> &RunStepError {
        &self.error
    }

    /// Number of execution steps that completed before the failed step attempt.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.core.completed_steps()
    }

    /// Borrow the parsed program used by this failed session.
    #[must_use]
    pub fn program(&self) -> &'program ExecutableProgram<P> {
        self.program
    }

    /// Borrow the uncommitted runtime state preserved by this error.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }

    /// Discard the uncommitted run session and return the runtime error.
    ///
    /// Borrowed failed runs are terminal; there is no retryable borrowed
    /// continuation after an uncommitted failure.
    #[must_use]
    pub fn into_error(self) -> RunStepError {
        self.error
    }
}

impl<P: ParsePolicy> core::fmt::Display for BorrowedFailedRun<'_, P> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl<P: ParsePolicy> core::error::Error for BorrowedFailedRun<'_, P> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.error)
    }
}
