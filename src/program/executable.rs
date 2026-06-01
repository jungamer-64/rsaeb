use crate::error::RunStartError;
use crate::execution::{BorrowedRuleAttemptSession, BorrowedRunSession, OwnedRunSession};
use crate::input::AdmittedRun;
use crate::policy::{ExecutionPolicy, ParsePolicy, RuleAttemptPolicy};

use super::{ActiveRuleCursor, Program};

/// Borrowed witness that a parsed program has at least one executable rule.
#[derive(Debug)]
pub struct BorrowedExecutableProgram<'program, P: ParsePolicy> {
    /// Parsed program proven to contain at least one executable rule.
    program: &'program Program<P>,
    /// First executable cursor minted from the proven non-empty rule table.
    first_cursor: ActiveRuleCursor<'program>,
}

/// Borrowed witness that a parsed program has no executable rules.
#[derive(Debug)]
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
}
