use core::marker::PhantomData;

use crate::error::{RunError, RunStartError};
use crate::input::AdmittedRun;
use crate::policy::{ExecutionPolicy, ParsePolicy, RuleAttemptPolicy};
use crate::program::{Program, RunResult};

use super::session::{
    BorrowedRuleAttemptSession, BorrowedRunSession, OwnedRuleAttemptSession, OwnedRunSession,
    finish_borrowed_run,
};

/// Sealed implementation detail for execution mode traits.
mod sealed {
    /// Private supertrait that keeps execution modes closed over crate-defined markers.
    pub trait Sealed {}
}

/// Borrowed run-to-completion execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompleteRun {}

/// Borrowed stepwise execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowedSteps {}

/// Owned stepwise execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnedSteps {}

/// Borrowed rule-attempt execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BorrowedRuleAttempts<A: RuleAttemptPolicy> {
    /// Rule-attempt policy selected by this mode.
    policy: PhantomData<fn() -> A>,
}

/// Owned rule-attempt execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnedRuleAttempts<A: RuleAttemptPolicy> {
    /// Rule-attempt policy selected by this mode.
    policy: PhantomData<fn() -> A>,
}

/// Sealed type-level selector for [`Program::execute`].
///
/// Implementations exist only for crate-defined borrowed execution modes, so
/// callers cannot smuggle a runtime mode value or an unrelated policy domain
/// into the execution boundary.
pub trait BorrowedExecutionMode<P: ParsePolicy, E: ExecutionPolicy>: sealed::Sealed {
    /// Result value produced by this mode.
    type Output<'program>
    where
        P: 'program;

    /// Error type produced before or during execution.
    type Error;

    /// Executes the selected borrowed mode.
    ///
    /// # Errors
    ///
    /// Returns this mode's phase-specific error if starting or advancing the run fails.
    fn execute<'program>(
        program: &'program Program<P>,
        admitted: AdmittedRun<E>,
    ) -> Result<Self::Output<'program>, Self::Error>;
}

/// Sealed type-level selector for [`Program::into_execute`].
///
/// Implementations exist only for crate-defined owned execution modes, so owned
/// execution cannot be requested through the borrowed entrypoint or vice versa.
pub trait OwnedExecutionMode<P: ParsePolicy, E: ExecutionPolicy>: sealed::Sealed {
    /// Result value produced by this mode.
    type Output;

    /// Error type produced before execution can start.
    type Error;

    /// Executes the selected owned mode.
    ///
    /// # Errors
    ///
    /// Returns this mode's phase-specific error if starting execution fails.
    fn into_execute(
        program: Program<P>,
        admitted: AdmittedRun<E>,
    ) -> Result<Self::Output, Self::Error>;
}

impl sealed::Sealed for CompleteRun {}

impl sealed::Sealed for BorrowedSteps {}

impl sealed::Sealed for OwnedSteps {}

impl<A: RuleAttemptPolicy> sealed::Sealed for BorrowedRuleAttempts<A> {}

impl<A: RuleAttemptPolicy> sealed::Sealed for OwnedRuleAttempts<A> {}

impl<P: ParsePolicy, E: ExecutionPolicy> BorrowedExecutionMode<P, E> for CompleteRun {
    type Output<'program>
        = RunResult
    where
        P: 'program;

    type Error = RunError;

    fn execute<'program>(
        program: &'program Program<P>,
        admitted: AdmittedRun<E>,
    ) -> Result<Self::Output<'program>, Self::Error> {
        finish_borrowed_run(program, admitted)
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> BorrowedExecutionMode<P, E> for BorrowedSteps {
    type Output<'program>
        = BorrowedRunSession<'program, P, E>
    where
        P: 'program;

    type Error = RunStartError;

    fn execute<'program>(
        program: &'program Program<P>,
        admitted: AdmittedRun<E>,
    ) -> Result<Self::Output<'program>, Self::Error> {
        BorrowedRunSession::new(program, admitted)
    }
}

impl<P, E, A> BorrowedExecutionMode<P, E> for BorrowedRuleAttempts<A>
where
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    type Output<'program>
        = BorrowedRuleAttemptSession<'program, P, E, A>
    where
        P: 'program;

    type Error = RunStartError;

    fn execute<'program>(
        program: &'program Program<P>,
        admitted: AdmittedRun<E>,
    ) -> Result<Self::Output<'program>, Self::Error> {
        BorrowedRuleAttemptSession::new(program, admitted)
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> OwnedExecutionMode<P, E> for OwnedSteps {
    type Output = OwnedRunSession<P, E>;

    type Error = RunStartError;

    fn into_execute(
        program: Program<P>,
        admitted: AdmittedRun<E>,
    ) -> Result<Self::Output, Self::Error> {
        OwnedRunSession::new(program, admitted)
    }
}

impl<P, E, A> OwnedExecutionMode<P, E> for OwnedRuleAttempts<A>
where
    P: ParsePolicy,
    E: ExecutionPolicy,
    A: RuleAttemptPolicy,
{
    type Output = OwnedRuleAttemptSession<P, E, A>;

    type Error = RunStartError;

    fn into_execute(
        program: Program<P>,
        admitted: AdmittedRun<E>,
    ) -> Result<Self::Output, Self::Error> {
        OwnedRuleAttemptSession::new(program, admitted)
    }
}
