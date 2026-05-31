use core::marker::PhantomData;

use crate::error::{RunError, RunStartError};
use crate::input::RunSeed;
use crate::policy::{ExecutionPolicy, ParsePolicy, RuleAttemptPolicy};
use crate::program::{Program, RunResult};

use super::session::{
    BorrowedRuleAttemptSession, BorrowedRunSession, OwnedRuleAttemptSession, OwnedRunSession,
};

/// Run-to-completion execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Complete;

/// Stepwise execution mode that pauses after matching rule applications.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stepwise;

/// Rule-attempt execution mode that pauses after each executable rule line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleAttempts<A: RuleAttemptPolicy> {
    /// Rule-attempt policy selected by this mode.
    policy: PhantomData<fn() -> A>,
}

/// Borrowed execution behavior selected by a mode type.
pub trait BorrowedExecutionMode<'program, P: ParsePolicy, E: ExecutionPolicy>:
    private::Sealed
{
    /// Successful output produced by this mode.
    type Output;
    /// Failure type produced by this mode.
    type Error;

    /// Starts or completes borrowed execution for this mode.
    #[doc(hidden)]
    fn execute(
        program: &'program Program<P>,
        seed: RunSeed<E>,
    ) -> Result<Self::Output, Self::Error>;
}

/// Owned execution behavior selected by a mode type.
pub trait OwnedExecutionMode<P: ParsePolicy, E: ExecutionPolicy>: private::Sealed {
    /// Successful output produced by this mode.
    type Output;
    /// Failure type produced by this mode.
    type Error;

    /// Starts owned execution for this mode.
    #[doc(hidden)]
    fn execute(program: Program<P>, seed: RunSeed<E>) -> Result<Self::Output, Self::Error>;
}

impl<'program, P: ParsePolicy, E: ExecutionPolicy> BorrowedExecutionMode<'program, P, E>
    for Complete
{
    type Output = RunResult;
    type Error = RunError;

    fn execute(
        program: &'program Program<P>,
        seed: RunSeed<E>,
    ) -> Result<Self::Output, Self::Error> {
        super::session::finish_borrowed_run(program, seed)
    }
}

impl<'program, P: ParsePolicy + 'program, E: ExecutionPolicy> BorrowedExecutionMode<'program, P, E>
    for Stepwise
{
    type Output = BorrowedRunSession<'program, P, E>;
    type Error = RunStartError;

    fn execute(
        program: &'program Program<P>,
        seed: RunSeed<E>,
    ) -> Result<Self::Output, Self::Error> {
        BorrowedRunSession::new(program, seed)
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy> OwnedExecutionMode<P, E> for Stepwise {
    type Output = OwnedRunSession<P, E>;
    type Error = RunStartError;

    fn execute(program: Program<P>, seed: RunSeed<E>) -> Result<Self::Output, Self::Error> {
        OwnedRunSession::new(program, seed)
    }
}

impl<'program, P: ParsePolicy + 'program, E: ExecutionPolicy, A: RuleAttemptPolicy>
    BorrowedExecutionMode<'program, P, E> for RuleAttempts<A>
{
    type Output = BorrowedRuleAttemptSession<'program, P, E, A>;
    type Error = RunStartError;

    fn execute(
        program: &'program Program<P>,
        seed: RunSeed<E>,
    ) -> Result<Self::Output, Self::Error> {
        BorrowedRuleAttemptSession::new(program, seed)
    }
}

impl<P: ParsePolicy, E: ExecutionPolicy, A: RuleAttemptPolicy> OwnedExecutionMode<P, E>
    for RuleAttempts<A>
{
    type Output = OwnedRuleAttemptSession<P, E, A>;
    type Error = RunStartError;

    fn execute(program: Program<P>, seed: RunSeed<E>) -> Result<Self::Output, Self::Error> {
        OwnedRuleAttemptSession::new(program, seed)
    }
}

/// Private sealing boundary for execution mode traits.
mod private {
    use crate::policy::RuleAttemptPolicy;

    /// Marker trait implemented only by built-in execution modes.
    pub trait Sealed {}

    impl Sealed for super::Complete {}
    impl Sealed for super::Stepwise {}
    impl<A: RuleAttemptPolicy> Sealed for super::RuleAttempts<A> {}
}
