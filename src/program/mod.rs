//! Parsed program and run-to-completion result types.
//!
//! [`Program`] is the immutable parsed A=B rule table. Hosts parse typed
//! [`ProgramSource`] under a [`ParsePolicy`], then
//! run with an admitted [`input::AdmittedRun`](crate::input::AdmittedRun). Runtime budget and byte-count types
//! live in [`limits`](crate::limits); runtime input lives in [`input`](crate::input).
//!
//! A parsed program owns syntax and rule metadata only. Per-run `(once)` state,
//! runtime bytes, completed-step counts, and execution budgets are created from
//! an [`input::AdmittedRun`](crate::input::AdmittedRun) each time execution starts. This keeps parsed source
//! reuse separate from mutable runtime progress.

/// Executable and empty parsed-program witnesses.
mod executable;
/// Parser limit value types and defaults.
pub(crate) mod limits;
/// Run result and output byte domains.
mod result;
/// Parsed rule table storage.
mod rule_set;
use core::marker::PhantomData;

use crate::error::ParseError;
use crate::inspect::{OnceRuleCount, RuleCount, RuleView};
use crate::parser::parse_rules_impl;
use crate::policy::ParsePolicy;
use crate::source::ProgramSource;

pub(crate) use rule_set::{ActiveRuleCursor, RuleCursorAfterMiss, RuleScan};
pub(crate) use rule_set::{RuleSet, RuleSetBuilder};

pub use executable::{
    BorrowedEmptyProgram, BorrowedExecutableProgram, ExecutableProgramRef, OwnedEmptyProgram,
    OwnedExecutableProgram,
};
pub use result::{ReturnOutput, ReturnOutputView, RunOutcome, RunResult, RuntimeStateSnapshot};

/// Parsed A=B rewrite program.
///
/// A parsed program is immutable and reusable. Per-run `(once)` state lives in
/// the runtime invocation, not in this value, so repeated runs with the same
/// [`Program`] start from fresh rule availability. Running a program requires
/// an already admitted [`input::AdmittedRun`](crate::input::AdmittedRun), so parsing
/// never accepts raw runtime input or detached execution policy values.
pub struct Program<P: ParsePolicy> {
    /// Immutable rule table plus parsed `(once)` metadata.
    rule_set: RuleSet,
    /// Compile-time parser policy selected for this program.
    policy: PhantomData<P>,
}

impl<P: ParsePolicy> core::fmt::Debug for Program<P> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Program")
            .field("rule_count", &self.rule_count())
            .field("once_rule_count", &self.once_rule_count())
            .finish()
    }
}

impl<P: ParsePolicy> Program<P> {
    /// Wraps a parser-built rule set as a reusable program.
    pub(crate) fn from_rule_set(rule_set: RuleSet) -> Self {
        Self {
            rule_set,
            policy: PhantomData,
        }
    }

    /// Parses typed program source into a reusable program value.
    ///
    /// [`ProgramSource`] marks the source boundary, while this program's
    /// [`ParsePolicy`] carries the parser resource
    /// policy. This method performs the actual A=B syntax validation and builds
    /// the immutable rule table.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` when source exceeds parser limits, executable code
    /// is not ASCII printable syntax, a non-empty code line does not contain
    /// exactly one `=`, reserved syntax appears as payload data, or allocation
    /// fails while building the parsed program.
    pub fn parse(source: ProgramSource<'_>) -> Result<Self, ParseError> {
        Ok(Self::from_rule_set(parse_rules_impl::<P>(source)?))
    }

    /// Returns the number of executable rules in the parsed program.
    ///
    /// Blank lines and comment-only lines are not executable rules and are not
    /// counted.
    #[must_use]
    pub fn rule_count(&self) -> RuleCount {
        self.rule_set.rule_count()
    }

    /// Returns the number of parsed `(once)` rules.
    ///
    /// This count describes parsed rule metadata only. It is not consumed or
    /// mutated by running the program.
    #[must_use]
    pub fn once_rule_count(&self) -> OnceRuleCount {
        self.rule_set.once_rule_count()
    }

    /// Iterates over structured parsed-rule views in execution order.
    ///
    /// The views borrow from this program. Canonical source can be materialized
    /// from a [`RuleView`] when needed, but source text is not stored as a
    /// second truth beside the parsed rule fields.
    pub fn rules(&self) -> impl Iterator<Item = RuleView<'_>> + '_ {
        self.rule_set.as_slice().iter().map(RuleView::new)
    }

    /// Mints a private runtime scan over the immutable rule table.
    pub(crate) fn rule_scan(&self) -> RuleScan<'_> {
        self.rule_set.scan()
    }

    /// Borrows this parsed program as an executable program when at least one rule exists.
    ///
    /// # Errors
    ///
    /// Returns `BorrowedEmptyProgram` when the parsed program has no executable rules.
    pub fn as_executable(
        &self,
    ) -> Result<BorrowedExecutableProgram<'_, P>, BorrowedEmptyProgram<'_, P>> {
        BorrowedExecutableProgram::from_program(self)
    }

    /// Moves this parsed program into an executable program when at least one rule exists.
    ///
    /// # Errors
    ///
    /// Returns `OwnedEmptyProgram` when the parsed program has no executable rules.
    pub fn into_executable(self) -> Result<OwnedExecutableProgram<P>, OwnedEmptyProgram<P>> {
        OwnedExecutableProgram::from_program(self)
    }
}
