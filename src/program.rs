use alloc::vec::Vec;
use core::convert::Infallible;

use crate::allocation::{AllocationContext, AllocationError, try_push};
use crate::bytes::{ReturnOutputByteCount, RuntimeStateByteCount};
use crate::error::{
    AebError, FallibleTraceSnapshotRunError, ParseError, RunError, TraceSnapshotError,
    TraceSnapshotRunError, TracedRunError,
};
use crate::parser::parse_program_impl;
use crate::rule::{OnceRuleSlotCount, ParsedRule, Rule, RuleCount, RulePosition, RuleView};
use crate::runtime::{Execution, RuntimeInput};
use crate::source::ProgramSource;
use crate::trace::{BorrowedTraceEvent, TraceSnapshotEvent};

const DEFAULT_BYTE_BUDGET: usize = 16_777_216;

/// Default rewrite step budget for callers that want the crate policy value.
pub const DEFAULT_MAX_STEPS: StepLimit = StepLimit::new(1_000_000);
/// Default runtime-state byte budget for callers that want the crate policy value.
pub const DEFAULT_MAX_STATE_LEN: StateByteLimit = StateByteLimit::new(DEFAULT_BYTE_BUDGET);
/// Default `(return)` output byte budget for callers that want the crate policy value.
pub const DEFAULT_MAX_RETURN_LEN: ReturnByteLimit = ReturnByteLimit::new(DEFAULT_BYTE_BUDGET);
/// Default trace snapshot byte budget for callers that want the crate default.
pub const DEFAULT_MAX_TRACE_SNAPSHOT_LEN: TraceSnapshotByteLimit =
    TraceSnapshotByteLimit::new(DEFAULT_BYTE_BUDGET);

/// Maximum number of rewrite steps allowed before the next matching rule fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StepLimit {
    value: usize,
}

impl StepLimit {
    /// Creates a step limit from a primitive count.
    #[must_use]
    pub const fn new(value: usize) -> Self {
        Self { value }
    }

    /// Returns this limit as a primitive count.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }
}

/// Maximum runtime state length in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StateByteLimit {
    value: usize,
}

impl StateByteLimit {
    /// Creates a runtime-state byte limit from a primitive length.
    #[must_use]
    pub const fn new(value: usize) -> Self {
        Self { value }
    }

    /// Returns this limit as a primitive length.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }
}

/// Maximum `(return)` output length in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReturnByteLimit {
    value: usize,
}

impl ReturnByteLimit {
    /// Creates a `(return)` output byte limit from a primitive length.
    #[must_use]
    pub const fn new(value: usize) -> Self {
        Self { value }
    }

    /// Returns this limit as a primitive length.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }
}

/// Maximum state/output bytes materialized for one trace snapshot event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TraceSnapshotByteLimit {
    value: usize,
}

impl TraceSnapshotByteLimit {
    /// Creates a trace snapshot byte limit from a primitive length.
    #[must_use]
    pub const fn new(value: usize) -> Self {
        Self { value }
    }

    /// Returns this limit as a primitive length.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }
}

/// Number of completed rewrite steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StepCount {
    value: usize,
}

impl StepCount {
    pub(crate) const ZERO: Self = Self { value: 0 };

    /// Returns this completed-step count as a primitive count.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }

    pub(crate) fn checked_next(self) -> Option<Self> {
        let value = self.value.checked_add(1)?;
        Some(Self { value })
    }
}

/// Resource limits for one runtime invocation.
///
/// The interpreter checks these limits before allocating oversized runtime
/// states or return outputs. Step limits alone are not enough for a rewriting
/// system because a tiny number of steps can still expand into a very large
/// state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunLimits {
    steps: StepLimit,
    state_len: StateByteLimit,
    return_len: ReturnByteLimit,
}

impl RunLimits {
    /// Creates limits with every runtime budget specified explicitly.
    #[must_use]
    pub const fn new(
        max_steps: StepLimit,
        max_state_len: StateByteLimit,
        max_return_len: ReturnByteLimit,
    ) -> Self {
        Self {
            steps: max_steps,
            state_len: max_state_len,
            return_len: max_return_len,
        }
    }

    /// Maximum number of rewrite steps that may be applied.
    #[must_use]
    pub const fn step_limit(self) -> StepLimit {
        self.steps
    }

    /// Maximum runtime state length, including initial input and rewrite results.
    #[must_use]
    pub const fn state_byte_limit(self) -> StateByteLimit {
        self.state_len
    }

    /// Maximum byte length accepted for `(return)` output.
    #[must_use]
    pub const fn return_byte_limit(self) -> ReturnByteLimit {
        self.return_len
    }

    /// Returns limits with a different step budget.
    #[must_use]
    pub const fn with_step_limit(mut self, max_steps: StepLimit) -> Self {
        self.steps = max_steps;
        self
    }

    /// Returns limits with a different runtime-state budget.
    #[must_use]
    pub const fn with_state_byte_limit(mut self, max_state_len: StateByteLimit) -> Self {
        self.state_len = max_state_len;
        self
    }

    /// Returns limits with a different return-output budget.
    #[must_use]
    pub const fn with_return_byte_limit(mut self, max_return_len: ReturnByteLimit) -> Self {
        self.return_len = max_return_len;
        self
    }
}

enum SnapshotTraceCallbackError<E> {
    Snapshot(TraceSnapshotError),
    Trace(E),
}

/// Parsed A=B rewrite program.
///
/// A parsed program is immutable and reusable. Per-run `(once)` state lives in
/// the runtime invocation, not in this value.
#[derive(Debug, PartialEq, Eq)]
pub struct Program {
    rule_set: RuleSet,
}

#[derive(Debug, PartialEq, Eq, Default)]
pub(crate) struct RuleSet {
    rules: Vec<Rule>,
    once_slot_count: OnceRuleSlotCount,
}

impl RuleSet {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push_parsed_rule(&mut self, parsed: ParsedRule) -> Result<(), AllocationError> {
        let position = RulePosition::from_zero_based(self.rules.len()).ok_or_else(|| {
            AllocationError::capacity_overflow(AllocationContext::ProgramRuleTable)
        })?;

        let (rule, next_once_slot_count) =
            Rule::from_parsed(parsed, position, self.once_slot_count)?;

        try_push(&mut self.rules, rule, AllocationContext::ProgramRuleTable)?;

        self.once_slot_count = next_once_slot_count;
        Ok(())
    }

    pub(crate) fn rule_count(&self) -> RuleCount {
        RuleCount::new(self.rules.len())
    }

    pub(crate) fn once_rule_count(&self) -> RuleCount {
        self.once_slot_count.as_rule_count()
    }

    pub(crate) const fn once_slot_count(&self) -> OnceRuleSlotCount {
        self.once_slot_count
    }

    pub(crate) fn as_slice(&self) -> &[Rule] {
        &self.rules
    }
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

    /// Runs this program and emits trace-snapshot, infallible events.
    ///
    /// This convenience API materializes `Vec<u8>` snapshots. Use
    /// `run_with_borrowed_trace` when the trace sink only needs to inspect each
    /// event during the callback.
    ///
    /// # Errors
    ///
    /// Returns `TraceSnapshotRunError::Run` for ordinary runtime failures.
    /// Returns `TraceSnapshotRunError::Snapshot` when snapshot materialization
    /// exceeds `trace_snapshot_limit` or allocation fails.
    pub fn run_with_trace_snapshots<'program, F>(
        &'program self,
        input: RuntimeInput<'_>,
        limits: RunLimits,
        trace_snapshot_limit: TraceSnapshotByteLimit,
        mut trace: F,
    ) -> Result<RunResult, TraceSnapshotRunError>
    where
        F: FnMut(TraceSnapshotEvent<'program>),
    {
        match self.try_run_with_trace_snapshots(input, limits, trace_snapshot_limit, |event| {
            trace(event);
            Ok::<(), Infallible>(())
        }) {
            Ok(result) => Ok(result),
            Err(FallibleTraceSnapshotRunError::Run(error)) => {
                Err(TraceSnapshotRunError::Run(error))
            }
            Err(FallibleTraceSnapshotRunError::Snapshot(error)) => {
                Err(TraceSnapshotRunError::Snapshot(error))
            }
            Err(FallibleTraceSnapshotRunError::Trace(error)) => match error {},
        }
    }

    /// Runs this program and emits trace-snapshot, fallible events.
    ///
    /// # Errors
    ///
    /// Returns `FallibleTraceSnapshotRunError::Run` for runtime failures.
    /// Returns `FallibleTraceSnapshotRunError::Snapshot` for snapshot
    /// materialization failures. Returns
    /// `FallibleTraceSnapshotRunError::Trace` when the user-provided trace
    /// callback returns an error.
    pub fn try_run_with_trace_snapshots<'program, F, E>(
        &'program self,
        input: RuntimeInput<'_>,
        limits: RunLimits,
        trace_snapshot_limit: TraceSnapshotByteLimit,
        mut trace: F,
    ) -> Result<RunResult, FallibleTraceSnapshotRunError<E>>
    where
        F: FnMut(TraceSnapshotEvent<'program>) -> Result<(), E>,
    {
        let result = self.try_run_with_borrowed_trace(input, limits, |event| {
            let snapshot = event
                .to_snapshot(trace_snapshot_limit)
                .map_err(SnapshotTraceCallbackError::Snapshot)?;
            trace(snapshot).map_err(SnapshotTraceCallbackError::Trace)
        });

        match result {
            Ok(result) => Ok(result),
            Err(TracedRunError::Run(error)) => Err(FallibleTraceSnapshotRunError::Run(error)),
            Err(TracedRunError::Trace(SnapshotTraceCallbackError::Snapshot(error))) => {
                Err(FallibleTraceSnapshotRunError::Snapshot(error))
            }
            Err(TracedRunError::Trace(SnapshotTraceCallbackError::Trace(error))) => {
                Err(FallibleTraceSnapshotRunError::Trace(error))
            }
        }
    }

    /// Runs this program and emits borrowed, infallible trace events.
    ///
    /// Borrowed trace events allocate nothing. They are valid only for the
    /// callback invocation, so a sink that wants to retain bytes must copy them
    /// explicitly.
    ///
    /// # Errors
    ///
    /// Returns `RunError` for the same runtime failures as `Program::run`.
    pub fn run_with_borrowed_trace<'program, F>(
        &'program self,
        input: RuntimeInput<'_>,
        limits: RunLimits,
        mut trace: F,
    ) -> Result<RunResult, RunError>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>),
    {
        match self.try_run_with_borrowed_trace(input, limits, |event| {
            trace(event);
            Ok::<(), Infallible>(())
        }) {
            Ok(result) => Ok(result),
            Err(TracedRunError::Run(error)) => Err(error),
            Err(TracedRunError::Trace(error)) => match error {},
        }
    }

    /// Runs this program and emits borrowed, fallible trace events.
    ///
    /// # Errors
    ///
    /// Returns `TracedRunError::Run` for ordinary runtime failures. Returns
    /// `TracedRunError::Trace` when the user-provided trace callback returns an
    /// error.
    pub fn try_run_with_borrowed_trace<'program, F, E>(
        &'program self,
        input: RuntimeInput<'_>,
        limits: RunLimits,
        trace: F,
    ) -> Result<RunResult, TracedRunError<E>>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), E>,
    {
        Execution::new(self, input, limits)
            .map_err(TracedRunError::Run)?
            .run_with_borrowed_trace(trace)
    }
}

/// Structured result category for one completed run.
#[derive(Debug, PartialEq, Eq)]
pub enum RunOutcome {
    /// No rule matched the final runtime state.
    Stable(RuntimeStateSnapshot),
    /// A matched rule executed the `(return)` action.
    Return(ReturnOutput),
}

/// Materialized final runtime state for a run that ended without `(return)`.
#[derive(Debug, PartialEq, Eq)]
pub struct RuntimeStateSnapshot {
    bytes: Vec<u8>,
}

impl RuntimeStateSnapshot {
    pub(crate) fn from_vec(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// Borrow the materialized runtime-state bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consumes the snapshot and returns the materialized bytes.
    #[must_use]
    pub fn into_vec(self) -> Vec<u8> {
        self.bytes
    }

    /// Materialized byte length.
    #[must_use]
    pub fn byte_count(&self) -> RuntimeStateByteCount {
        RuntimeStateByteCount::new(self.bytes.len())
    }

    /// Whether this snapshot contains no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// Materialized final output from a matched `(return)` rule.
#[derive(Debug, PartialEq, Eq)]
pub struct ReturnOutput {
    bytes: Vec<u8>,
}

impl ReturnOutput {
    pub(crate) fn from_vec(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// Borrow the materialized `(return)` output bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consumes the return output and returns the materialized bytes.
    #[must_use]
    pub fn into_vec(self) -> Vec<u8> {
        self.bytes
    }

    /// Materialized byte length.
    #[must_use]
    pub fn byte_count(&self) -> ReturnOutputByteCount {
        ReturnOutputByteCount::new(self.bytes.len())
    }

    /// Whether this return output contains no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// Result of one program execution.
#[derive(Debug, PartialEq, Eq)]
pub struct RunResult {
    steps: StepCount,
    outcome: RunOutcome,
}

impl RunResult {
    pub(crate) fn stable(output: RuntimeStateSnapshot, steps: StepCount) -> Self {
        Self {
            steps,
            outcome: RunOutcome::Stable(output),
        }
    }

    pub(crate) fn from_return(output: ReturnOutput, steps: StepCount) -> Self {
        Self {
            steps,
            outcome: RunOutcome::Return(output),
        }
    }

    /// Structured execution outcome.
    #[must_use]
    pub const fn outcome(&self) -> &RunOutcome {
        &self.outcome
    }

    /// Consumes the result and returns the structured execution outcome.
    #[must_use]
    pub fn into_outcome(self) -> RunOutcome {
        self.outcome
    }

    /// Number of rewrite steps applied.
    #[must_use]
    pub const fn steps(&self) -> StepCount {
        self.steps
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        TestFailure, TestResult, ensure, ensure_eq, ensure_matches, expect_event,
        expect_return_output, expect_run_error, expect_stable_output, expect_state_limit,
        result_bytes, run_program, trace_event_bytes,
    };
    use crate::{
        AebError, InputError, LimitError, ReturnByteLimit, ReturnOutputByteCount, RuleActionView,
        RuleAnchor, RuleCount, RuleRepeat, RuntimeStateByteCount, StateByteLimit,
        StateLimitContext, TraceSnapshotEffect, TraceSnapshotEvent, run_bytes, run_str,
    };
    use std::vec::Vec;

    fn expect_rule(program: &Program, index: usize) -> Result<RuleView<'_>, TestFailure> {
        program
            .rules()
            .nth(index)
            .ok_or(TestFailure::message("expected parsed rule"))
    }

    #[test]
    fn public_free_run_works() -> TestResult {
        let result = run_str(
            "a=b",
            b"a",
            RunLimits::new(
                crate::DEFAULT_MAX_STEPS,
                crate::DEFAULT_MAX_STATE_LEN,
                crate::DEFAULT_MAX_RETURN_LEN,
            ),
        )?;
        expect_stable_output(&result, b"b")?;
        ensure_eq!(result.steps().get(), 1)?;

        let result = run_bytes(
            b"a=b#\xff",
            b"a",
            RunLimits::new(
                crate::DEFAULT_MAX_STEPS,
                crate::DEFAULT_MAX_STATE_LEN,
                crate::DEFAULT_MAX_RETURN_LEN,
            ),
        )?;
        expect_stable_output(&result, b"b")?;
        Ok(())
    }

    #[test]
    fn public_free_run_reports_input_boundary_errors_separately() -> TestResult {
        let result = run_bytes(
            b"a=b",
            &[0xff],
            RunLimits::new(
                crate::DEFAULT_MAX_STEPS,
                crate::DEFAULT_MAX_STATE_LEN,
                crate::DEFAULT_MAX_RETURN_LEN,
            ),
        );

        ensure_matches(
            matches!(
                result,
                Err(AebError::Run(RunError::Input(InputError::NonAscii { column, .. })))
                    if column.get() == 1
            ),
            "expected one-shot run input error",
        )
    }

    #[test]
    fn parsed_program_is_reusable_and_once_state_is_per_run() -> TestResult {
        let program = Program::parse_str("(once)a=b\na=c")?;

        let limits = RunLimits::new(
            StepLimit::new(10_000),
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        );
        let first = run_program(&program, b"aa", limits)?;
        let second = run_program(&program, b"aa", limits)?;

        ensure_eq!(result_bytes(&first), b"bc".as_slice())?;
        ensure_eq!(result_bytes(&second), b"bc".as_slice())?;
        ensure_eq!(program.once_rule_count(), RuleCount::new(1))?;
        Ok(())
    }

    #[test]
    fn always_rules_do_not_allocate_once_slots() -> TestResult {
        let program = Program::parse_str("a=b\nb=c\n(start)c=d")?;

        ensure_eq!(program.rule_count(), RuleCount::new(3))?;
        ensure_eq!(program.once_rule_count(), RuleCount::new(0))?;
        Ok(())
    }

    #[test]
    fn run_outcome_separates_stable_state_from_return_output() -> TestResult {
        let limits = RunLimits::new(
            StepLimit::new(1),
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        );
        let stable = run_program(&Program::parse_str("a=b")?, b"a", limits)?;
        let returned = run_program(&Program::parse_str("a=(return)b")?, b"a", limits)?;

        match stable.into_outcome() {
            RunOutcome::Stable(output) => {
                ensure_eq!(output.as_bytes(), b"b".as_slice())?;
                ensure_eq!(output.byte_count(), RuntimeStateByteCount::new(1))?;
            }
            RunOutcome::Return(_) => return Err(TestFailure::message("expected stable outcome")),
        }

        match returned.into_outcome() {
            RunOutcome::Return(output) => {
                ensure_eq!(output.as_bytes(), b"b".as_slice())?;
                ensure_eq!(output.byte_count(), ReturnOutputByteCount::new(1))?;
            }
            RunOutcome::Stable(_) => return Err(TestFailure::message("expected return outcome")),
        }

        Ok(())
    }

    #[test]
    fn rule_view_generates_canonical_source_without_stored_source_blob() -> TestResult {
        let program = Program::parse_str("a = b # comment\n(start)c=(end)d")?;
        let rules = program.rules().collect::<Vec<_>>();

        ensure_eq!(rules.len(), 2)?;
        let first = rules
            .first()
            .copied()
            .ok_or(TestFailure::message("expected first rule"))?;
        let second = rules
            .get(1)
            .copied()
            .ok_or(TestFailure::message("expected second rule"))?;

        ensure_eq!(first.position().number().get(), 1)?;
        ensure_eq!(first.line_number().get(), 1)?;
        ensure_eq!(first.repeat(), RuleRepeat::Always)?;
        ensure_eq!(first.anchor(), RuleAnchor::Anywhere)?;
        ensure(first.lhs().eq_bytes(b"a"), "expected first lhs")?;
        ensure_matches(
            matches!(
                first.action(),
                RuleActionView::Replace(payload) if payload.eq_bytes(b"b")
            ),
            "expected replace action",
        )?;
        ensure_eq!(first.canonical_source()?, b"a=b".as_slice())?;

        ensure_eq!(second.position().number().get(), 2)?;
        ensure_eq!(second.line_number().get(), 2)?;
        ensure_eq!(second.repeat(), RuleRepeat::Always)?;
        ensure_eq!(second.anchor(), RuleAnchor::Start)?;
        ensure(second.lhs().eq_bytes(b"c"), "expected second lhs")?;
        ensure_matches(
            matches!(
                second.action(),
                RuleActionView::MoveEnd(payload) if payload.eq_bytes(b"d")
            ),
            "expected move-end action",
        )?;
        ensure_eq!(second.canonical_source()?, b"(start)c=(end)d".as_slice())?;
        Ok(())
    }

    #[test]
    fn canonical_source_reparses_to_the_same_executable_rule() -> TestResult {
        let program = Program::parse_str("( once ) ( start ) a = ( end ) b # comment")?;
        let canonical = expect_rule(&program, 0)?.canonical_source()?;

        let reparsed = Program::parse_bytes(canonical.as_slice())?;
        let reparsed_rule = expect_rule(&reparsed, 0)?;

        ensure_eq!(reparsed.rule_count(), RuleCount::new(1))?;
        ensure_eq!(reparsed.once_rule_count(), RuleCount::new(1))?;
        ensure_eq!(reparsed_rule.repeat(), RuleRepeat::Once)?;
        ensure_eq!(reparsed_rule.anchor(), RuleAnchor::Start)?;
        ensure(reparsed_rule.lhs().eq_bytes(b"a"), "expected lhs")?;
        ensure_eq!(
            reparsed_rule.canonical_source()?,
            b"(once)(start)a=(end)b".as_slice(),
        )?;
        Ok(())
    }

    #[test]
    fn canonical_source_roundtrips_all_supported_rule_shapes() -> TestResult {
        const EMPTY: &[u8] = b"";
        const ONCE: &[u8] = b"(once)";
        const START: &[u8] = b"(start)";
        const END: &[u8] = b"(end)";
        const RETURN: &[u8] = b"(return)";
        const A: &[u8] = b"a";
        const B: &[u8] = b"b";

        let repeats: &[&[u8]] = &[EMPTY, ONCE];
        let anchors: &[&[u8]] = &[EMPTY, START, END];
        let left_payloads: &[&[u8]] = &[EMPTY, A];
        let actions: &[&[u8]] = &[EMPTY, START, END, RETURN];
        let right_payloads: &[&[u8]] = &[EMPTY, B];

        for &repeat in repeats {
            for &anchor in anchors {
                for &lhs in left_payloads {
                    for &action in actions {
                        for &rhs in right_payloads {
                            let mut source = Vec::new();
                            source.extend_from_slice(repeat);
                            source.extend_from_slice(anchor);
                            source.extend_from_slice(lhs);
                            source.push(b'=');
                            source.extend_from_slice(action);
                            source.extend_from_slice(rhs);

                            let program = Program::parse_bytes(&source)?;
                            let rule = expect_rule(&program, 0)?;
                            let canonical = rule.canonical_source()?;

                            ensure_eq!(program.rule_count(), RuleCount::new(1))?;
                            ensure_eq!(canonical.as_slice(), source.as_slice())?;

                            let reparsed = Program::parse_bytes(&canonical)?;
                            let reparsed_rule = expect_rule(&reparsed, 0)?;
                            ensure_eq!(reparsed_rule.canonical_source()?, source.as_slice())?;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    #[test]
    fn state_limit_rejects_oversized_input_before_runtime_allocation() -> TestResult {
        let limits = RunLimits::new(
            StepLimit::new(10),
            StateByteLimit::new(1),
            ReturnByteLimit::new(10),
        );
        let error =
            expect_run_error(Program::parse_str("# no executable rules")?.run(b"aa", limits))?;
        let error = expect_state_limit(error)?;

        ensure_eq!(
            error,
            LimitError::State {
                context: StateLimitContext::Input,
                limit: StateByteLimit::new(1),
                attempted_len: RuntimeStateByteCount::new(2),
            },
        )?;
        Ok(())
    }

    #[test]
    fn state_limit_rejects_oversized_rewrite_before_allocating_next_state() -> TestResult {
        let limits = RunLimits::new(
            StepLimit::new(10),
            StateByteLimit::new(2),
            ReturnByteLimit::new(10),
        );
        let error = expect_run_error(Program::parse_str("=a")?.run(b"aa", limits))?;
        let error = expect_state_limit(error)?;

        ensure_eq!(
            error,
            LimitError::State {
                context: StateLimitContext::Rewrite,
                limit: StateByteLimit::new(2),
                attempted_len: RuntimeStateByteCount::new(3),
            },
        )?;
        Ok(())
    }

    #[test]
    fn trace_snapshots_are_derived_from_borrowed_trace() -> TestResult {
        let program = Program::parse_str("a=b\nb=(return)ok")?;
        let mut events = Vec::new();
        let limits = RunLimits::new(
            StepLimit::new(10_000),
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        );
        let result = program.run_with_trace_snapshots(
            b"a",
            limits,
            DEFAULT_MAX_TRACE_SNAPSHOT_LEN,
            |event| {
                events.push(event);
            },
        )?;

        expect_return_output(&result, b"ok")?;
        ensure_eq!(events.len(), 3)?;
        ensure_matches(
            matches!(events.first(), Some(TraceSnapshotEvent::Initial { .. })),
            "expected initial trace event",
        )?;
        let initial = expect_event(&events, 0)?;
        let first_step = expect_event(&events, 1)?;
        let second_step = expect_event(&events, 2)?;

        ensure_eq!(trace_event_bytes(initial), b"a".as_slice())?;
        ensure_eq!(trace_event_bytes(first_step), b"b".as_slice())?;
        ensure_eq!(trace_event_bytes(second_step), b"ok".as_slice())?;
        ensure_matches(
            matches!(
                first_step,
                TraceSnapshotEvent::Step {
                    effect: TraceSnapshotEffect::Continue { .. },
                    ..
                }
            ),
            "expected continue step",
        )?;
        ensure_matches(
            matches!(
                second_step,
                TraceSnapshotEvent::Step {
                    effect: TraceSnapshotEffect::Return { .. },
                    ..
                }
            ),
            "expected return step",
        )?;

        match first_step {
            TraceSnapshotEvent::Step {
                rule,
                effect: TraceSnapshotEffect::Continue { state },
                ..
            } => {
                ensure_eq!(state.as_bytes(), b"b".as_slice())?;
                ensure_eq!(rule.canonical_source()?, b"a=b".as_slice())?;
            }
            TraceSnapshotEvent::Initial { .. } | TraceSnapshotEvent::Step { .. } => {
                return Err(TestFailure::message("expected continue step"));
            }
        }
        Ok(())
    }
}
