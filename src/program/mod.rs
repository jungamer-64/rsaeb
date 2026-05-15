mod limits;
mod result;
mod rule_set;
mod tracing;

use crate::error::{ParseError, RunError};
use crate::parser::parse_program_impl;
use crate::rule::{Rule, RuleCount, RuleView};
use crate::runtime::{RunningExecution, RuntimeInput};
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

    /// Starts a stateful execution session for this program.
    ///
    /// The input must already be validated as [`RuntimeInput`]. This function
    /// materializes it into the mutable runtime-state byte domain under
    /// `limits`.
    ///
    /// The returned [`RunningExecution`] can be advanced one matching rule at a
    /// time. Use [`Program::run`] when the caller wants to run to completion in
    /// one call.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when the validated input exceeds this run's state
    /// limit or when allocating per-run execution state fails.
    pub fn start_execution(
        &self,
        input: &RuntimeInput,
        limits: RunLimits,
    ) -> Result<RunningExecution<'_>, RunError> {
        RunningExecution::new(self, input, limits)
    }

    /// Runs this program with already-validated runtime input.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when the input exceeds this run's state limit,
    /// allocation fails, state-size arithmetic overflows, or a configured
    /// `RunLimits` budget would be exceeded.
    pub fn run(&self, input: &RuntimeInput, limits: RunLimits) -> Result<RunResult, RunError> {
        RunningExecution::new(self, input, limits)?.finish()
    }
}
