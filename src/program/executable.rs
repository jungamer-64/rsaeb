use core::fmt;

use crate::error::{
    EmptyProgramParseError, ExecutableProgramParseError, RunError, RunFinishError, RunStartError,
};
use crate::execution::{BorrowedRuleAttemptCursor, BorrowedRunSession};
use crate::input::AdmittedRun;
use crate::inspect::{OnceRuleCount, RuleCount, RuleView};
use crate::limits::StepCount;
use crate::parser::parse_rules_into;
use crate::policy::{ExecutionPolicy, ParsePolicy, RuleAttemptPolicy};
use crate::runtime::state::State;
use crate::source::RawProgramSource;
use crate::trace::TraceRequest;

use super::{
    EmptyRuleSetBuilder, ExecutableRuleSet, ExecutableRuleSetBuilder, RuleScan, RunResult,
};

/// Parsed source with no executable rule lines.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct EmptyProgram;

/// Parsed source with at least one executable rule line.
#[derive(PartialEq, Eq)]
pub struct ExecutableProgram {
    /// Immutable non-empty executable rule topology.
    rule_set: ExecutableRuleSet,
}

impl EmptyProgram {
    /// Parses source bytes that must contain no executable rules.
    ///
    /// The empty-program target type is the public shape selection. Source bytes
    /// are not wrapped in a separate expected-shape marker; syntax validation
    /// and executable-rule rejection happen in this parse boundary.
    ///
    /// # Errors
    ///
    /// Returns `EmptyProgramParseError` when parsing fails or when the parsed
    /// source contains executable rules.
    pub fn parse_bytes<P: ParsePolicy>(source: &[u8]) -> Result<Self, EmptyProgramParseError> {
        Self::parse_raw::<P>(RawProgramSource::from_bytes(source))
    }

    /// Parses UTF-8 source text that must contain no executable rules.
    ///
    /// # Errors
    ///
    /// Returns `EmptyProgramParseError` when parsing fails or when the parsed
    /// source contains executable rules.
    pub fn parse_text<P: ParsePolicy>(source: &str) -> Result<Self, EmptyProgramParseError> {
        Self::parse_raw::<P>(RawProgramSource::from_text(source))
    }

    /// Parses raw source into the empty-program target type.
    ///
    /// # Errors
    ///
    /// Returns `EmptyProgramParseError` when parsing fails or when executable
    /// rules are present.
    fn parse_raw<P: ParsePolicy>(
        source: RawProgramSource<'_>,
    ) -> Result<Self, EmptyProgramParseError> {
        parse_rules_into::<P, EmptyRuleSetBuilder>(source)?;
        Ok(Self::new())
    }

    /// Builds a typed empty-program value.
    const fn new() -> Self {
        Self
    }

    /// Iterates over structured parsed-rule views.
    pub fn rules<'rule>(self) -> core::iter::Empty<RuleView<'rule>> {
        core::iter::empty()
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

impl fmt::Debug for EmptyProgram {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("EmptyProgram").finish()
    }
}

impl ExecutableProgram {
    /// Parses source bytes that must contain at least one executable rule.
    ///
    /// The executable-program target type is the public shape selection. Source
    /// bytes are not wrapped in a separate expected-shape marker; syntax
    /// validation and non-empty executable-rule proof happen in this parse
    /// boundary.
    ///
    /// # Errors
    ///
    /// Returns `ExecutableProgramParseError` when parsing fails or when the
    /// parsed source contains no executable rules.
    pub fn parse_bytes<P: ParsePolicy>(source: &[u8]) -> Result<Self, ExecutableProgramParseError> {
        Self::parse_raw::<P>(RawProgramSource::from_bytes(source))
    }

    /// Parses UTF-8 source text that must contain at least one executable rule.
    ///
    /// # Errors
    ///
    /// Returns `ExecutableProgramParseError` when parsing fails or when the
    /// parsed source contains no executable rules.
    pub fn parse_text<P: ParsePolicy>(source: &str) -> Result<Self, ExecutableProgramParseError> {
        Self::parse_raw::<P>(RawProgramSource::from_text(source))
    }

    /// Parses raw source into the executable-program target type.
    ///
    /// # Errors
    ///
    /// Returns `ExecutableProgramParseError` when parsing fails or when no
    /// executable rules are present.
    fn parse_raw<P: ParsePolicy>(
        source: RawProgramSource<'_>,
    ) -> Result<Self, ExecutableProgramParseError> {
        let rule_set = parse_rules_into::<P, ExecutableRuleSetBuilder>(source)?;
        Ok(Self::from_rule_set(rule_set))
    }

    /// Wraps a parser-built non-empty rule set as an executable program.
    fn from_rule_set(rule_set: ExecutableRuleSet) -> Self {
        Self { rule_set }
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
    #[must_use]
    pub fn once_rule_count(&self) -> OnceRuleCount {
        self.rule_set.once_rule_count()
    }

    /// Iterates over structured parsed-rule views in execution order.
    pub fn rules(&self) -> impl Iterator<Item = RuleView<'_>> + '_ {
        self.rule_set.iter().map(|positioned| positioned.view())
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
        crate::execution::finish_borrowed_run(self, admitted)
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
        R: TraceRequest<'program, E>,
    {
        request.trace(self, admitted)
    }

    /// Starts borrowed stepwise execution.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule state fails.
    pub fn steps<E>(
        &self,
        admitted: AdmittedRun<E>,
    ) -> Result<BorrowedRunSession<'_, E>, RunStartError>
    where
        E: ExecutionPolicy,
    {
        BorrowedRunSession::new(self, admitted)
    }

    /// Starts borrowed rule-attempt execution as a typed continuing/final cursor.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule-attempt state fails.
    pub fn rule_attempts<A, E>(
        &self,
        admitted: AdmittedRun<E>,
    ) -> Result<BorrowedRuleAttemptCursor<'_, E, A>, RunStartError>
    where
        A: RuleAttemptPolicy,
        E: ExecutionPolicy,
    {
        BorrowedRuleAttemptCursor::new(self, admitted)
    }

    /// Mints a private runtime scan over the immutable rule table.
    pub(crate) fn rule_scan(&self) -> RuleScan<'_> {
        self.rule_set.scan()
    }
}

impl fmt::Debug for ExecutableProgram {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExecutableProgram")
            .field("rule_count", &self.rule_count())
            .field("once_rule_count", &self.once_rule_count())
            .finish()
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
