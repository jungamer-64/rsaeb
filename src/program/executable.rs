use crate::error::{RunError, RunFinishError, RunStartError};
use crate::execution::{BorrowedRuleAttemptSession, BorrowedRunSession, OwnedRunSession};
use crate::input::AdmittedRun;
use crate::policy::{ExecutionPolicy, ParsePolicy, RuleAttemptPolicy};
use crate::runtime::state::State;
use crate::trace::TraceRequest;

use super::{ActiveRuleCursor, Program, RunResult};
use crate::limits::StepCount;

/// Borrowed proof that a parsed program contains at least one executable rule.
///
/// Callers cannot construct this value directly. It is minted only from
/// [`BorrowedExecutableProgram`] or [`OwnedExecutableProgram`], so run and trace
/// internals cannot be entered with a shape-erased [`Program`].
#[derive(Debug, Clone, Copy)]
pub struct ExecutableProgramRef<'program, P: ParsePolicy> {
    /// Parsed program proven to contain at least one executable rule.
    program: &'program Program<P>,
}

/// Borrowed witness that a parsed program has at least one executable rule.
#[derive(Debug, Clone, Copy)]
pub struct BorrowedExecutableProgram<'program, P: ParsePolicy> {
    /// Parsed program proven to contain at least one executable rule.
    program: &'program Program<P>,
    /// First executable cursor minted from the proven non-empty rule table.
    first_cursor: ActiveRuleCursor<'program>,
}

/// Borrowed witness that a parsed program has no executable rules.
#[derive(Debug, Clone, Copy)]
pub struct BorrowedEmptyProgram<'program, P: ParsePolicy> {
    /// Parsed program proven to contain no executable rules.
    program: &'program Program<P>,
}

/// Owned witness that a parsed program has at least one executable rule.
#[derive(Debug)]
pub struct OwnedExecutableProgram<P: ParsePolicy> {
    /// Parsed program proven to contain at least one executable rule.
    program: Program<P>,
}

/// Owned witness that a parsed program has no executable rules.
#[derive(Debug)]
pub struct OwnedEmptyProgram<P: ParsePolicy> {
    /// Parsed program proven to contain no executable rules.
    program: Program<P>,
}

impl<'program, P: ParsePolicy> BorrowedExecutableProgram<'program, P> {
    /// Classifies a borrowed parsed program as executable.
    ///
    /// # Errors
    ///
    /// Returns `BorrowedEmptyProgram` when the parsed program has no executable rules.
    pub(crate) fn from_program(
        program: &'program Program<P>,
    ) -> Result<Self, BorrowedEmptyProgram<'program, P>> {
        match program.rule_scan().first_cursor() {
            Some(first_cursor) => Ok(Self {
                program,
                first_cursor,
            }),
            None => Err(BorrowedEmptyProgram { program }),
        }
    }

    /// Borrows the executable parsed program.
    #[must_use]
    pub const fn program(&self) -> &'program Program<P> {
        self.program
    }

    /// Borrows this executable witness as the run/trace execution boundary.
    #[must_use]
    pub const fn as_executable_ref(&self) -> ExecutableProgramRef<'program, P> {
        ExecutableProgramRef {
            program: self.program,
        }
    }

    /// Executes this executable program to completion.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when execution setup fails or a later matching rule would
    /// exceed configured limits.
    pub fn execute<E>(self, admitted: AdmittedRun<E>) -> Result<RunResult, RunError>
    where
        E: ExecutionPolicy,
    {
        crate::execution::finish_borrowed_run(self.as_executable_ref(), admitted)
    }

    /// Runs this executable program while emitting trace events selected by a typed request.
    ///
    /// # Errors
    ///
    /// Returns the selected trace request's error type when runtime execution,
    /// snapshot materialization, or the user trace sink fails.
    pub fn trace<E, R>(self, admitted: AdmittedRun<E>, request: R) -> Result<RunResult, R::Error>
    where
        E: ExecutionPolicy,
        R: TraceRequest<'program, P, E>,
    {
        request.trace(self.as_executable_ref(), admitted)
    }

    /// Starts borrowed stepwise execution.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule state fails.
    pub fn steps<E>(
        self,
        admitted: AdmittedRun<E>,
    ) -> Result<BorrowedRunSession<'program, P, E>, RunStartError>
    where
        E: ExecutionPolicy,
    {
        BorrowedRunSession::new(self.program, admitted)
    }

    /// Starts borrowed rule-attempt execution.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule state fails.
    pub fn rule_attempts<A, E>(
        self,
        admitted: AdmittedRun<E>,
    ) -> Result<BorrowedRuleAttemptSession<'program, P, E, A>, RunStartError>
    where
        A: RuleAttemptPolicy,
        E: ExecutionPolicy,
    {
        BorrowedRuleAttemptSession::new(self.program, self.first_cursor, admitted)
    }
}

impl<'program, P: ParsePolicy> BorrowedEmptyProgram<'program, P> {
    /// Borrows the empty parsed program.
    #[must_use]
    pub const fn program(&self) -> &'program Program<P> {
        self.program
    }

    /// Stabilizes admitted input for an empty program as a zero-step result.
    ///
    /// # Errors
    ///
    /// Returns `RunFinishError` if materializing the admitted initial state as
    /// stable output fails.
    pub fn stabilize<E>(self, admitted: AdmittedRun<E>) -> Result<RunResult, RunFinishError>
    where
        E: ExecutionPolicy,
    {
        stabilize_empty_input(admitted)
    }
}

impl<P: ParsePolicy> OwnedExecutableProgram<P> {
    /// Classifies an owned parsed program as executable.
    ///
    /// # Errors
    ///
    /// Returns `OwnedEmptyProgram` when the parsed program has no executable rules.
    pub(crate) fn from_program(program: Program<P>) -> Result<Self, OwnedEmptyProgram<P>> {
        if program.rule_scan().first_cursor().is_some() {
            Ok(Self { program })
        } else {
            Err(OwnedEmptyProgram { program })
        }
    }

    /// Borrows the executable parsed program.
    #[must_use]
    pub const fn program(&self) -> &Program<P> {
        &self.program
    }

    /// Borrows this executable witness as the run/trace execution boundary.
    #[must_use]
    pub const fn as_executable_ref(&self) -> ExecutableProgramRef<'_, P> {
        ExecutableProgramRef {
            program: &self.program,
        }
    }

    /// Executes this executable program to completion.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when execution setup fails or a later matching rule would
    /// exceed configured limits.
    pub fn execute<E>(&self, admitted: AdmittedRun<E>) -> Result<RunResult, RunError>
    where
        E: ExecutionPolicy,
    {
        crate::execution::finish_borrowed_run(self.as_executable_ref(), admitted)
    }

    /// Runs this executable program while emitting trace events selected by a typed request.
    ///
    /// # Errors
    ///
    /// Returns the selected trace request's error type when runtime execution,
    /// snapshot materialization, or the user trace sink fails.
    pub fn trace<'program, E, R>(
        &'program self,
        admitted: AdmittedRun<E>,
        request: R,
    ) -> Result<RunResult, R::Error>
    where
        E: ExecutionPolicy,
        R: TraceRequest<'program, P, E>,
    {
        request.trace(self.as_executable_ref(), admitted)
    }

    /// Starts owned stepwise execution.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule state fails.
    pub fn into_steps<E>(
        self,
        admitted: AdmittedRun<E>,
    ) -> Result<OwnedRunSession<P, E>, RunStartError>
    where
        E: ExecutionPolicy,
    {
        OwnedRunSession::new(self.program, admitted)
    }
}

impl<P: ParsePolicy> OwnedEmptyProgram<P> {
    /// Borrows the empty parsed program.
    #[must_use]
    pub const fn program(&self) -> &Program<P> {
        &self.program
    }

    /// Recovers the owned empty parsed program.
    #[must_use]
    pub fn into_program(self) -> Program<P> {
        self.program
    }

    /// Stabilizes admitted input for an empty program as a zero-step result.
    ///
    /// # Errors
    ///
    /// Returns `RunFinishError` if materializing the admitted initial state as
    /// stable output fails.
    pub fn stabilize<E>(self, admitted: AdmittedRun<E>) -> Result<RunResult, RunFinishError>
    where
        E: ExecutionPolicy,
    {
        stabilize_empty_input(admitted)
    }
}

impl<'program, P: ParsePolicy> ExecutableProgramRef<'program, P> {
    /// Borrows the executable parsed program.
    #[must_use]
    pub const fn program(self) -> &'program Program<P> {
        self.program
    }
}

/// Materializes admitted input as the stable output of an empty program.
///
/// # Errors
///
/// Returns `RunFinishError` if final-state materialization fails.
fn stabilize_empty_input<E>(admitted: AdmittedRun<E>) -> Result<RunResult, RunFinishError>
where
    E: ExecutionPolicy,
{
    let (input, _budget) = admitted.into_runtime_parts();
    let snapshot = State::from_input(input)
        .into_snapshot()
        .map_err(RunFinishError::FinalOutput)?;
    Ok(RunResult::stable(snapshot, StepCount::ZERO))
}
