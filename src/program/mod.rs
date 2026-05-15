mod limits;
mod result;
mod rule_set;
mod tracing;

use crate::error::{ParseError, RunError, RuntimeInvariantError};
use crate::parser::parse_program_impl;
use crate::rule::{
    Action, OnceRuleSlotCount, PayloadView, Rule, RuleCount, RulePosition, RuleView,
};
use crate::runtime::{Execution, ExecutionCore, OwnedExecution, RuntimeInput};
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

    pub(crate) fn rule_at_position(&self, position: RulePosition) -> Result<&Rule, RunError> {
        self.rule_set.rule_at_position(position).ok_or_else(|| {
            RuntimeInvariantError::missing_terminal_rule(position, self.rule_count()).into()
        })
    }

    pub(crate) fn return_rule_at(
        &self,
        position: RulePosition,
    ) -> Result<(&Rule, PayloadView<'_>), RunError> {
        let rule = self.rule_at_position(position)?;
        match rule.action() {
            Action::Return(output) => Ok((rule, PayloadView::new(output))),
            Action::Replace(_) | Action::MoveStart(_) | Action::MoveEnd(_) => {
                Err(RuntimeInvariantError::terminal_rule_not_return(position).into())
            }
        }
    }

    pub(crate) fn return_output_at(
        &self,
        position: RulePosition,
    ) -> Result<PayloadView<'_>, RunError> {
        self.return_rule_at(position).map(|(_, output)| output)
    }

    pub(crate) const fn once_slot_count(&self) -> OnceRuleSlotCount {
        self.rule_set.once_slot_count()
    }

    /// Starts a stateful execution session for this program.
    ///
    /// The input must already be validated as [`RuntimeInput`]. This function
    /// materializes it into the mutable runtime-state byte domain under
    /// `limits`.
    ///
    /// The returned [`Execution`] can be advanced one matching rule at a time.
    /// Use [`Program::run`] when the caller wants to run to completion in one
    /// call.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when the validated input exceeds this run's state
    /// limit, when allocating per-run `(once)` state fails, or when an internal
    /// runtime invariant is violated.
    pub fn start_execution(
        &self,
        input: RuntimeInput<'_>,
        limits: RunLimits,
    ) -> Result<Execution<'_>, RunError> {
        Execution::new(self, input, limits)
    }

    /// Consumes this program and starts an owned stateful execution session.
    ///
    /// The input is materialized into the execution state during construction,
    /// so the returned [`OwnedExecution`] does not borrow the input bytes.
    ///
    /// # Errors
    ///
    /// Returns `RunError` for the same startup failures as
    /// [`Program::start_execution`].
    pub fn into_execution(
        self,
        input: RuntimeInput<'_>,
        limits: RunLimits,
    ) -> Result<OwnedExecution, RunError> {
        let core = ExecutionCore::new(&self, input, limits)?;
        Ok(OwnedExecution::new(self, core))
    }

    /// Runs this program with already-validated runtime input.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when the input exceeds this run's state limit,
    /// allocation fails, state-size arithmetic overflows, a configured
    /// `RunLimits` budget would be exceeded, or an internal runtime invariant
    /// is violated.
    pub fn run(&self, input: RuntimeInput<'_>, limits: RunLimits) -> Result<RunResult, RunError> {
        Execution::new(self, input, limits)?.finish()
    }
}
