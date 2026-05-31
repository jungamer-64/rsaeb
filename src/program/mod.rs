//! Parsed program and run-to-completion result types.
//!
//! [`Program`] is the immutable parsed A=B rule table. Hosts parse typed
//! [`ProgramSource`] under a [`ParsePolicy`], then
//! run with an admitted [`AdmittedRun`]. Runtime budget and byte-count types
//! live in [`limits`](crate::limits); runtime input lives in [`input`](crate::input).
//!
//! A parsed program owns syntax and rule metadata only. Per-run `(once)` state,
//! runtime bytes, completed-step counts, and execution budgets are created from
//! an [`AdmittedRun`] each time execution starts. This keeps parsed source
//! reuse separate from mutable runtime progress.

/// Parser limit value types and defaults.
pub(crate) mod limits;
/// Run result and output byte domains.
mod result;
/// Parsed rule table storage.
mod rule_set;
use core::marker::PhantomData;

use crate::error::{ParseError, RunError, RunStartError, TraceSnapshotRunError, TracedRunError};
use crate::execution::{
    BorrowedRuleAttemptSession, BorrowedRunSession, OwnedRuleAttemptSession, OwnedRunSession,
};
use crate::input::AdmittedRun;
use crate::inspect::{OnceRuleCount, RuleCount, RuleView};
use crate::parser::parse_rules_impl;
use crate::policy::{ExecutionPolicy, ParsePolicy, RuleAttemptPolicy, TraceSnapshotPolicy};
use crate::source::ProgramSource;
use crate::trace::{BorrowedTraceEvent, TraceSnapshotEvent};

pub(crate) use rule_set::{
    RuleAttemptTargetSelection, RuleCursor, RuleCursorAfterMiss, RuleScan, RuleTarget,
};
pub(crate) use rule_set::{RuleSet, RuleSetBuilder};

pub use result::{ReturnOutput, ReturnOutputView, RunOutcome, RunResult, RuntimeStateSnapshot};

/// Trace callback failure split used while borrowed events become snapshots.
enum SnapshotTraceCallbackError<E> {
    /// Snapshot materialization failed before the user callback ran.
    Snapshot(crate::error::TraceSnapshotError),
    /// User callback rejected a materialized snapshot event.
    Trace(E),
}

/// Parsed A=B rewrite program.
///
/// A parsed program is immutable and reusable. Per-run `(once)` state lives in
/// the runtime invocation, not in this value, so repeated runs with the same
/// [`Program`] start from fresh rule availability. Running a program requires
/// an already admitted [`AdmittedRun`], so parsing
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

    /// Runs this program to completion with borrowed program ownership.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if execution setup fails, a matching rule exceeds
    /// configured limits, or final state materialization fails.
    pub fn run<E>(&self, admitted: AdmittedRun<E>) -> Result<RunResult, RunError>
    where
        E: ExecutionPolicy,
    {
        crate::execution::finish_borrowed_run(self, admitted)
    }

    /// Starts borrowed stepwise execution.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if per-run rule state allocation fails.
    pub fn start<E>(
        &self,
        admitted: AdmittedRun<E>,
    ) -> Result<BorrowedRunSession<'_, P, E>, RunStartError>
    where
        E: ExecutionPolicy,
    {
        BorrowedRunSession::new(self, admitted)
    }

    /// Starts owned stepwise execution.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if per-run rule state allocation fails.
    pub fn into_start<E>(
        self,
        admitted: AdmittedRun<E>,
    ) -> Result<OwnedRunSession<P, E>, RunStartError>
    where
        E: ExecutionPolicy,
    {
        OwnedRunSession::new(self, admitted)
    }

    /// Starts borrowed rule-attempt execution.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if per-run rule state allocation fails.
    pub fn start_rule_attempts<A>(
        &self,
        admitted: AdmittedRun<impl ExecutionPolicy>,
    ) -> Result<BorrowedRuleAttemptSession<'_, P, impl ExecutionPolicy, A>, RunStartError>
    where
        A: RuleAttemptPolicy,
    {
        BorrowedRuleAttemptSession::new(self, admitted)
    }

    /// Starts owned rule-attempt execution.
    ///
    /// # Errors
    ///
    /// Returns `RunStartError` if per-run rule state allocation fails.
    pub fn into_rule_attempts<A>(
        self,
        admitted: AdmittedRun<impl ExecutionPolicy>,
    ) -> Result<OwnedRuleAttemptSession<P, impl ExecutionPolicy, A>, RunStartError>
    where
        A: RuleAttemptPolicy,
    {
        OwnedRuleAttemptSession::new(self, admitted)
    }

    /// Runs this program while emitting borrowed trace events.
    ///
    /// # Errors
    ///
    /// Returns `TracedRunError::Run` for runtime failures and
    /// `TracedRunError::Trace` for user callback failures.
    pub fn trace_borrowed<'program, E, F, TraceError>(
        &'program self,
        admitted: AdmittedRun<E>,
        trace: F,
    ) -> Result<RunResult, TracedRunError<TraceError>>
    where
        E: ExecutionPolicy,
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), TraceError>,
    {
        crate::execution::trace_borrowed_events(self, admitted, trace)
    }

    /// Runs this program while emitting materialized trace snapshot events.
    ///
    /// # Errors
    ///
    /// Returns `TraceSnapshotRunError::Run` for runtime failures,
    /// `TraceSnapshotRunError::Snapshot` for snapshot materialization failures,
    /// and `TraceSnapshotRunError::Trace` for user callback failures.
    pub fn trace_snapshots<'program, E, T, F, TraceError>(
        &'program self,
        admitted: AdmittedRun<E>,
        mut trace: F,
    ) -> Result<RunResult, TraceSnapshotRunError<TraceError>>
    where
        E: ExecutionPolicy,
        T: TraceSnapshotPolicy,
        F: FnMut(TraceSnapshotEvent<'program>) -> Result<(), TraceError>,
    {
        let result = self.trace_borrowed(admitted, |event| {
            let snapshot = event
                .to_snapshot::<T>()
                .map_err(SnapshotTraceCallbackError::Snapshot)?;
            trace(snapshot).map_err(SnapshotTraceCallbackError::Trace)
        });

        match result {
            Ok(result) => Ok(result),
            Err(TracedRunError::Run(error)) => Err(TraceSnapshotRunError::Run(error)),
            Err(TracedRunError::Trace(SnapshotTraceCallbackError::Snapshot(error))) => {
                Err(TraceSnapshotRunError::Snapshot(error))
            }
            Err(TracedRunError::Trace(SnapshotTraceCallbackError::Trace(error))) => {
                Err(TraceSnapshotRunError::Trace(error))
            }
        }
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
