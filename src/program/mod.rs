mod limits;
mod result;
mod rule_set;
mod tracing;

#[cfg(test)]
mod tests;

use crate::error::{ParseError, RunError};
use crate::parser::parse_program_impl;
use crate::rule::{OnceRuleSlotCount, Rule, RuleCount, RuleView};
use crate::runtime::{Execution, RuntimeInput};
use crate::source::ProgramSource;

pub(crate) use rule_set::RuleSet;

pub use limits::{
    DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_STEPS,
    DEFAULT_MAX_TRACE_SNAPSHOT_LEN, ReturnByteLimit, RunLimits, StateByteLimit, StepCount,
    StepLimit, TraceSnapshotByteLimit, TraceSnapshotLimits,
};
pub use result::{ReturnOutput, RunOutcome, RunResult, RuntimeStateSnapshot};

/// Parsed A=B rewrite program.
///
/// A parsed program is immutable and reusable. Per-run `(once)` state lives in
/// the runtime invocation, not in this value.
#[derive(Debug, PartialEq, Eq)]
pub struct Program {
    rule_set: RuleSet,
}

impl Program {
    pub(crate) fn from_rule_set(rule_set: RuleSet) -> Self {
        Self { rule_set }
    }

    /// Parses typed program source into a reusable program value.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` when executable code is not ASCII printable syntax,
    /// when a non-empty code line does not contain exactly one `=`, when
    /// reserved syntax appears as payload data, or when allocation fails while
    /// building the parsed program.
    pub fn parse(source: ProgramSource<'_>) -> Result<Self, ParseError> {
        parse_program_impl(source)
    }

    /// Returns the number of executable rules in the parsed program.
    #[must_use]
    pub fn rule_count(&self) -> RuleCount {
        self.rule_set.rule_count()
    }

    /// Returns the number of parsed `(once)` rules.
    #[must_use]
    pub fn once_rule_count(&self) -> RuleCount {
        self.rule_set.once_rule_count()
    }

    /// Iterates over structured parsed-rule views in execution order.
    pub fn rules(&self) -> impl Iterator<Item = RuleView<'_>> + '_ {
        self.rule_set.as_slice().iter().map(Rule::view)
    }

    pub(crate) fn rule_slice(&self) -> &[Rule] {
        self.rule_set.as_slice()
    }

    pub(crate) const fn once_slot_count(&self) -> OnceRuleSlotCount {
        self.rule_set.once_slot_count()
    }

    /// Starts a stateful execution session for this program.
    ///
    /// The returned [`Execution`] can be advanced one matching rule at a time.
    /// Use [`Program::run`] when the caller wants to run to completion in one
    /// call.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when the raw input is invalid, the input exceeds this run's state
    /// limit, when allocating per-run `(once)` state fails, or when an internal
    /// runtime invariant is violated.
    pub fn start_execution(
        &self,
        input: RuntimeInput<'_>,
        limits: RunLimits,
    ) -> Result<Execution<'_>, RunError> {
        Execution::new(self, input, limits)
    }

    /// Runs this program with raw runtime input validated inside the run limits.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when the raw input is invalid, the input exceeds this run's state
    /// limit, an allocation fails, state-size arithmetic overflows, a
    /// configured `RunLimits` budget would be exceeded, or an internal runtime
    /// invariant is violated.
    pub fn run(&self, input: RuntimeInput<'_>, limits: RunLimits) -> Result<RunResult, RunError> {
        Execution::new(self, input, limits)?.finish()
    }
}
