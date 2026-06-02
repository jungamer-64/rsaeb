use core::{fmt, marker::PhantomData};

use crate::error::{
    EmptyProgramParseError, ExecutableProgramParseError, RunError, RunFinishError, RunStartError,
};
use crate::execution::{BorrowedRuleAttemptSession, BorrowedRunSession};
use crate::input::AdmittedRun;
use crate::inspect::{OnceRuleCount, RuleCount, RuleView};
use crate::limits::StepCount;
use crate::parser::parse_rules_impl;
use crate::policy::{ExecutionPolicy, ParsePolicy, RuleAttemptPolicy};
use crate::runtime::state::State;
use crate::source::ProgramSource;
use crate::trace::TraceRequest;

use super::{ExecutableRuleSet, RuleScan, RuleSetShape, RunResult};

/// Parsed source with no executable rule lines.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct EmptyProgram<P: ParsePolicy> {
    /// Compile-time parser policy selected for this empty program.
    policy: PhantomData<P>,
}

/// Parsed source with at least one executable rule line.
#[derive(PartialEq, Eq)]
pub struct ExecutableProgram<P: ParsePolicy> {
    /// Immutable non-empty rule table plus parsed `(once)` metadata.
    rule_set: ExecutableRuleSet,
    /// Compile-time parser policy selected for this program.
    policy: PhantomData<P>,
}

/// Borrowed executable-program reference used by trace request implementations.
#[derive(Debug, Clone, Copy)]
pub struct ExecutableProgramRef<'program, P: ParsePolicy> {
    /// Parsed program proven to contain at least one executable rule.
    program: &'program ExecutableProgram<P>,
}

impl<P: ParsePolicy> EmptyProgram<P> {
    /// Parses source that is expected to contain no executable rules.
    ///
    /// [`ProgramSource`] marks the source boundary, while this program's
    /// [`ParsePolicy`] carries the parser resource policy. Empty source is
    /// accepted only through this empty-program boundary, so executable methods
    /// are unavailable by construction.
    ///
    /// # Errors
    ///
    /// Returns `EmptyProgramParseError` when parsing fails or when the parsed
    /// source contains executable rules.
    pub fn parse(source: ProgramSource<'_>) -> Result<Self, EmptyProgramParseError> {
        match parse_rules_impl::<P>(source)?.into_shape() {
            RuleSetShape::Empty => Ok(Self::new()),
            RuleSetShape::Executable(rule_set) => Err(EmptyProgramParseError::ExecutableRules {
                rule_count: rule_set.rule_count(),
            }),
        }
    }

    /// Builds a typed empty-program value.
    const fn new() -> Self {
        Self {
            policy: PhantomData,
        }
    }

    /// Returns the number of executable rules in this empty program.
    #[must_use]
    pub const fn rule_count(&self) -> RuleCount {
        RuleCount::new(0)
    }

    /// Returns the number of parsed `(once)` rules in this empty program.
    #[must_use]
    pub const fn once_rule_count(&self) -> OnceRuleCount {
        OnceRuleCount::ZERO
    }

    /// Iterates over structured parsed-rule views.
    pub fn rules(&self) -> core::iter::Empty<RuleView<'_>> {
        core::iter::empty()
    }

    /// Stabilizes admitted input for an empty program as a zero-step result.
    ///
    /// # Errors
    ///
    /// Returns `RunFinishError` if materializing the admitted initial state as
    /// stable output fails.
    pub fn stabilize<E>(&self, admitted: AdmittedRun<E>) -> Result<RunResult, RunFinishError>
    where
        E: ExecutionPolicy,
    {
        stabilize_empty_input(admitted)
    }
}

impl<P: ParsePolicy> fmt::Debug for EmptyProgram<P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmptyProgram")
            .field("rule_count", &self.rule_count())
            .field("once_rule_count", &self.once_rule_count())
            .finish()
    }
}

impl<P: ParsePolicy> ExecutableProgram<P> {
    /// Parses source that is expected to contain at least one executable rule.
    ///
    /// [`ProgramSource`] marks the source boundary, while this program's
    /// [`ParsePolicy`] carries the parser resource policy. Execution, tracing,
    /// stepwise execution, and rule-attempt execution start only from this
    /// executable-program boundary.
    ///
    /// # Errors
    ///
    /// Returns `ExecutableProgramParseError` when parsing fails or when the
    /// parsed source contains no executable rules.
    pub fn parse(source: ProgramSource<'_>) -> Result<Self, ExecutableProgramParseError> {
        match parse_rules_impl::<P>(source)?.into_shape() {
            RuleSetShape::Empty => Err(ExecutableProgramParseError::NoExecutableRules),
            RuleSetShape::Executable(rule_set) => Ok(Self::from_rule_set(rule_set)),
        }
    }

    /// Wraps a parser-built non-empty rule set as an executable program.
    fn from_rule_set(rule_set: ExecutableRuleSet) -> Self {
        Self {
            rule_set,
            policy: PhantomData,
        }
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
        self.rule_set.iter().map(RuleView::new)
    }

    /// Borrows this executable program as the run/trace execution boundary.
    #[must_use]
    pub(crate) const fn executable_ref(&self) -> ExecutableProgramRef<'_, P> {
        ExecutableProgramRef { program: self }
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
        crate::execution::finish_borrowed_run(self.executable_ref(), admitted)
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
        request.trace(self.executable_ref(), admitted)
    }

    /// Starts borrowed stepwise execution.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule state fails.
    pub fn steps<E>(
        &self,
        admitted: AdmittedRun<E>,
    ) -> Result<BorrowedRunSession<'_, P, E>, RunStartError>
    where
        E: ExecutionPolicy,
    {
        BorrowedRunSession::new(self, admitted)
    }

    /// Starts borrowed rule-attempt execution.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if allocating per-run rule state fails.
    pub fn rule_attempts<A, E>(
        &self,
        admitted: AdmittedRun<E>,
    ) -> Result<BorrowedRuleAttemptSession<'_, P, E, A>, RunStartError>
    where
        A: RuleAttemptPolicy,
        E: ExecutionPolicy,
    {
        BorrowedRuleAttemptSession::new(self, admitted)
    }

    /// Mints a private runtime scan over the immutable rule table.
    pub(crate) fn rule_scan(&self) -> RuleScan<'_> {
        self.rule_set.scan()
    }
}

impl<P: ParsePolicy> fmt::Debug for ExecutableProgram<P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExecutableProgram")
            .field("rule_count", &self.rule_count())
            .field("once_rule_count", &self.once_rule_count())
            .finish()
    }
}

impl<'program, P: ParsePolicy> ExecutableProgramRef<'program, P> {
    /// Borrows the executable parsed program.
    #[must_use]
    pub const fn program(self) -> &'program ExecutableProgram<P> {
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
