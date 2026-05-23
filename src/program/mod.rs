pub(crate) mod limits;
mod result;
mod rule_set;
mod tracing;

use crate::error::{ParseError, RunError};
use crate::execution::RunSession;
use crate::inspect::{OnceRuleCount, RuleCount, RulePositions, RuleView};
use crate::parser::parse_rules_impl;
use crate::rule::Rule;
use crate::input::RuntimeInput;
use crate::limits::{ParseLimits, RunLimits};
use crate::source::ProgramSource;

pub(crate) use rule_set::RuleSet;

pub use result::{ReturnOutput, ReturnOutputView, RunOutcome, RunResult, RuntimeStateSnapshot};

/// Parsed A=B rewrite program.
///
/// A parsed program is immutable and reusable. Per-run `(once)` state lives in
/// the runtime invocation, not in this value, so repeated runs with the same
/// [`Program`] start from fresh rule availability.
pub struct Program {
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

    pub(crate) fn rule_slice(&self) -> &[Rule] {
        self.rule_set.as_slice()
    }

    pub(crate) const fn once_slot_count(&self) -> crate::rule::OnceRuleCount {
        self.rule_set.once_slot_count()
    }

    /// Starts a stateful run session for this program.
    ///
    /// The input must already be validated as [`RuntimeInput`]. This function
    /// consumes it and moves its typed bytes into the mutable runtime-state
    /// domain under `limits`.
    ///
    /// The returned [`RunSession`] can be advanced one matching rule at a time.
    /// Use [`Program::run`] when the caller wants to run to completion in one
    /// call.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when the validated input exceeds this run's state
    /// limit or when allocating per-run execution state fails.
    pub fn start_run(
        &self,
        input: RuntimeInput,
        limits: RunLimits,
    ) -> Result<RunSession<'_>, RunError> {
        RunSession::new(self, input, limits)
    }

    /// Runs this program with already-validated runtime input.
    ///
    /// This is the run-to-completion API. Use [`Program::start_run`] when
    /// the host needs to observe or pause after each committed rule.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when the input exceeds this run's state limit,
    /// allocation fails, state-size arithmetic overflows, or a configured
    /// `RunLimits` budget would be exceeded.
    pub fn run(&self, input: RuntimeInput, limits: RunLimits) -> Result<RunResult, RunError> {
        RunSession::new(self, input, limits)?.finish()
    }
}
