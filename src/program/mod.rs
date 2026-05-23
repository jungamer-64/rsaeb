//! Parsed program and run-to-completion result types.
//!
//! [`Program`] is the immutable parsed A=B rule table. Hosts parse typed
//! [`ProgramSource`] under [`ParseLimits`], then run with an admitted
//! [`RunSeed`]. Runtime budget and byte-count types live in
//! [`limits`](crate::limits); runtime input lives in [`input`](crate::input).

/// Parser limit value types and defaults.
pub(crate) mod limits;
/// Run result and output byte domains.
mod result;
/// Parsed rule table storage.
mod rule_set;
/// Program-level tracing entrypoints.
mod tracing;

use crate::error::{InternalInvariantError, ParseError, RunError};
use crate::execution::OwnedRunSession;
use crate::input::RunSeed;
use crate::inspect::{OnceRuleCount, RuleCount, RulePositions, RuleView};
use crate::limits::ParseLimits;
use crate::parser::parse_rules_impl;
use crate::rule::{Rule, RuleAction};
use crate::source::ProgramSource;

pub(crate) use rule_set::RuleSet;

pub use result::{ReturnOutput, ReturnOutputView, RunOutcome, RunResult, RuntimeStateSnapshot};

/// Parsed A=B rewrite program.
///
/// A parsed program is immutable and reusable. Per-run `(once)` state lives in
/// the runtime invocation, not in this value, so repeated runs with the same
/// [`Program`] start from fresh rule availability.
pub struct Program {
    /// Immutable rule table plus parsed `(once)` metadata.
    rule_set: RuleSet,
}

impl core::fmt::Debug for Program {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Program")
            .field("rule_count", &self.rule_count())
            .field("once_rule_count", &self.once_rule_count())
            .finish()
    }
}

impl Program {
    /// Wraps a parser-built rule set as a reusable program.
    pub(crate) fn from_rule_set(rule_set: RuleSet) -> Self {
        Self { rule_set }
    }

    /// Parses typed program source into a reusable program value.
    ///
    /// [`ProgramSource`] marks the source boundary, while [`ParseLimits`]
    /// carries the host's parser resource policy. This method performs the
    /// actual A=B syntax validation and builds the immutable rule table.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` when source exceeds parser limits, executable code
    /// is not ASCII printable syntax, a non-empty code line does not contain
    /// exactly one `=`, reserved syntax appears as payload data, or allocation
    /// fails while building the parsed program.
    pub fn parse(source: ProgramSource<'_>, limits: ParseLimits) -> Result<Self, ParseError> {
        Ok(Self::from_rule_set(parse_rules_impl(source, limits)?))
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
        self.rule_set
            .as_slice()
            .iter()
            .zip(RulePositions::new())
            .map(|(rule, position)| RuleView::new(position, rule))
    }

    /// Borrows the immutable rule table in execution order.
    pub(crate) fn rule_slice(&self) -> &[Rule] {
        self.rule_set.as_slice()
    }

    /// Resolves a committed rule position back to a borrowed rule view.
    ///
    /// # Errors
    ///
    /// Returns `RunError::InternalInvariant` if the position no longer
    /// resolves inside this parsed program.
    pub(crate) fn rule_view_at(
        &self,
        position: crate::inspect::RulePosition,
    ) -> Result<RuleView<'_>, RunError> {
        let rule = self
            .rule_set
            .as_slice()
            .get(position.zero_based())
            .ok_or_else(InternalInvariantError::missing_committed_rule)?;
        Ok(RuleView::new(position, rule))
    }

    /// Resolves a committed return-rule position to its return-output view.
    ///
    /// # Errors
    ///
    /// Returns `RunError::InternalInvariant` if the position no longer
    /// resolves inside this parsed program or if it points at a non-return
    /// rule.
    pub(crate) fn return_output_at(
        &self,
        position: crate::inspect::RulePosition,
    ) -> Result<ReturnOutputView<'_>, RunError> {
        let rule = self
            .rule_set
            .as_slice()
            .get(position.zero_based())
            .ok_or_else(InternalInvariantError::missing_committed_rule)?;
        let RuleAction::Return(output) = rule.action() else {
            return Err(InternalInvariantError::returned_rule_without_output().into());
        };
        Ok(ReturnOutputView::new(output))
    }

    /// Number of once slots required for each new run.
    pub(crate) const fn once_slot_count(&self) -> crate::rule::OnceRuleCount {
        self.rule_set.once_slot_count()
    }

    /// Starts a stateful run session that owns this parsed program.
    ///
    /// This consumes `self` so hosts move the parsed rule table into the
    /// execution typestate instead of keeping a separate program borrow alive.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when allocating per-run execution state fails.
    pub fn into_run(self, seed: RunSeed) -> Result<OwnedRunSession, RunError> {
        OwnedRunSession::new(self, seed)
    }

    /// Runs this program with admitted runtime seed.
    ///
    /// This is the borrowed run-to-completion API. Use [`Program::into_run`]
    /// when the host needs owned stepwise control after each committed rule.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when allocation fails, state-size arithmetic
    /// overflows, or a configured execution budget would be exceeded.
    pub fn run(&self, seed: RunSeed) -> Result<RunResult, RunError> {
        crate::execution::finish_borrowed_run(self, seed)
    }
}
