//! Parsed program and run-to-completion result types.
//!
//! [`Program`] is the immutable parsed A=B rule table. Hosts parse typed
//! [`ProgramSource`] under a [`ParsePolicy`], then
//! run with an admitted [`RunSeed`]. Runtime budget and byte-count types live
//! in [`limits`](crate::limits); runtime input lives in [`input`](crate::input).
//!
//! A parsed program owns syntax and rule metadata only. Per-run `(once)` state,
//! runtime bytes, completed-step counts, and execution budgets are created from
//! a [`RunSeed`] each time execution starts. This keeps parsed source reuse
//! separate from mutable runtime progress.

/// Parser limit value types and defaults.
pub(crate) mod limits;
/// Run result and output byte domains.
mod result;
/// Parsed rule table storage.
mod rule_set;
/// Program-level tracing entrypoints.
mod tracing;

use core::marker::PhantomData;

use crate::error::{ParseError, RunError, RunStartError};
use crate::execution::{
    BorrowedRuleAttemptSession, BorrowedRunSession, OwnedRuleAttemptSession, OwnedRunSession,
    RuleAttemptSeed,
};
use crate::input::RunSeed;
use crate::inspect::{OnceRuleCount, RuleCount, RuleView};
use crate::parser::parse_rules_impl;
use crate::policy::{DefaultParsePolicy, ExecutionPolicy, ParsePolicy, RuleAttemptPolicy};
use crate::source::ProgramSource;

pub(crate) use rule_set::{
    RuleAttemptTargetSelection, RuleCursor, RuleCursorAfterMiss, RuleScan, RuleTarget,
};
pub(crate) use rule_set::{RuleSet, RuleSetBuilder};

pub use result::{ReturnOutput, ReturnOutputView, RunOutcome, RunResult, RuntimeStateSnapshot};

/// Parsed A=B rewrite program.
///
/// A parsed program is immutable and reusable. Per-run `(once)` state lives in
/// the runtime invocation, not in this value, so repeated runs with the same
/// [`Program`] start from fresh rule availability. Running a program requires
/// an already admitted [`RunSeed`], so parsing never accepts raw runtime input
/// or detached execution policy values.
pub struct Program<P: ParsePolicy = DefaultParsePolicy> {
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

    /// Starts a stateful run session that borrows this parsed program.
    ///
    /// The parsed rule table stays reusable while per-run state lives in the
    /// returned execution session. Use this when the caller can keep the parsed
    /// program outside the session and wants to inspect or pause after each
    /// applied step.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` when allocating per-run execution state fails.
    pub fn start_run<E: ExecutionPolicy>(
        &self,
        seed: RunSeed<E>,
    ) -> Result<BorrowedRunSession<'_, P, E>, RunStartError> {
        BorrowedRunSession::new(self, seed)
    }

    /// Starts a stateful run session that owns this parsed program.
    ///
    /// Use this when the execution session must carry the parsed program with
    /// it, for example across a `'static` task boundary. Use
    /// [`Program::start_run`] when the host keeps the reusable parsed program
    /// separately.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` when allocating per-run execution state fails.
    pub fn into_run<E: ExecutionPolicy>(
        self,
        seed: RunSeed<E>,
    ) -> Result<OwnedRunSession<P, E>, RunStartError> {
        OwnedRunSession::new(self, seed)
    }

    /// Starts a stateful borrowed run session that advances by executable rule attempt.
    ///
    /// Unlike [`Program::start_run`], this mode reports non-matching rule lines
    /// before continuing to the next rule. Matching rewrites still reset the
    /// rule cursor to the first executable rule.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` when allocating per-run execution state fails.
    pub fn start_rule_attempt_run<E: ExecutionPolicy, A: RuleAttemptPolicy>(
        &self,
        seed: RuleAttemptSeed<E, A>,
    ) -> Result<BorrowedRuleAttemptSession<'_, P, E, A>, RunStartError> {
        BorrowedRuleAttemptSession::new(self, seed)
    }

    /// Starts a stateful owned run session that advances by executable rule attempt.
    ///
    /// This is the owned counterpart to [`Program::start_rule_attempt_run`].
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` when allocating per-run execution state fails.
    pub fn into_rule_attempt_run<E: ExecutionPolicy, A: RuleAttemptPolicy>(
        self,
        seed: RuleAttemptSeed<E, A>,
    ) -> Result<OwnedRuleAttemptSession<P, E, A>, RunStartError> {
        OwnedRuleAttemptSession::new(self, seed)
    }

    /// Runs this program with admitted runtime seed.
    ///
    /// This is the borrowed run-to-completion API. Use [`Program::start_run`]
    /// when the host needs stepwise control while keeping this parsed program
    /// reusable. Use [`Program::into_run`] when the session must own the parsed
    /// program.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when allocation fails, state-size arithmetic
    /// overflows, or a configured execution budget would be exceeded.
    pub fn run<E: ExecutionPolicy>(&self, seed: RunSeed<E>) -> Result<RunResult, RunError> {
        crate::execution::finish_borrowed_run(self, seed)
    }

    /// Starts a rule-attempt cursor minted from this parsed rule table.
    pub(crate) fn rule_attempt_cursor(&self) -> RuleCursor {
        self.rule_set.rule_attempt_cursor()
    }

    /// Selects the next checked rule-attempt target.
    pub(crate) fn select_attempt_target(
        &self,
        cursor: RuleCursor,
    ) -> RuleAttemptTargetSelection<'_> {
        self.rule_set.select_attempt_target(cursor)
    }
}
