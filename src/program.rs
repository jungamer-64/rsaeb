use alloc::vec::Vec;
use core::convert::Infallible;

use crate::allocation::{AllocationContext, AllocationError, try_push};
use crate::bytes::ByteCount;
use crate::error::{AebError, ParseError, RunError, TracedRunError};
use crate::parser::parse_program_impl;
use crate::rule::{ParsedRule, Rule, RuleCount, RulePosition, RuleView};
use crate::runtime::Runtime;
use crate::trace::{BorrowedTraceEvent, TraceSnapshotEvent};

pub const DEFAULT_MAX_STEPS: StepLimit = StepLimit::new(1_000_000);
pub const DEFAULT_MAX_STATE_LEN: StateByteLimit = StateByteLimit::new(16 * 1024 * 1024);
pub const DEFAULT_MAX_RETURN_LEN: ReturnByteLimit = ReturnByteLimit::new(16 * 1024 * 1024);
pub const DEFAULT_MAX_TRACE_SNAPSHOT_LEN: TraceSnapshotByteLimit =
    TraceSnapshotByteLimit::new(16 * 1024 * 1024);

/// Maximum number of rewrite steps allowed before the next matching rule fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StepLimit {
    value: usize,
}

impl StepLimit {
    #[must_use]
    pub const fn new(value: usize) -> Self {
        Self { value }
    }

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
    #[must_use]
    pub const fn new(value: usize) -> Self {
        Self { value }
    }

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
    #[must_use]
    pub const fn new(value: usize) -> Self {
        Self { value }
    }

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
    #[must_use]
    pub const fn new(value: usize) -> Self {
        Self { value }
    }

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

    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }

    pub(crate) fn checked_next(self) -> Option<Self> {
        let value = self.value.checked_add(1)?;
        Some(Self { value })
    }
}

/// Parses source bytes and runs them once with the given input bytes.
///
/// # Errors
///
/// Returns `AebError::Parse` when `source` is not valid A=B program syntax.
/// Returns `AebError::Run` when `input` is invalid, an allocation fails, or a
/// configured runtime limit would be exceeded.
pub fn run_bytes(source: &[u8], input: &[u8], limits: RunLimits) -> Result<RunResult, AebError> {
    let program = Program::parse_bytes(source)?;
    program.run(input, limits).map_err(AebError::Run)
}

/// Parses a UTF-8 source string and runs it once with the given input bytes.
///
/// # Errors
///
/// Returns `AebError::Parse` when `source` is not valid A=B program syntax.
/// Returns `AebError::Run` when `input` is invalid, an allocation fails, or a
/// configured runtime limit would be exceeded.
pub fn run_str(source: &str, input: &[u8], limits: RunLimits) -> Result<RunResult, AebError> {
    run_bytes(source.as_bytes(), input, limits)
}

/// Resource limits for one runtime invocation.
///
/// The interpreter checks these limits before allocating oversized runtime
/// states, return outputs, or trace snapshots. Step limits alone are not
/// enough for a rewriting system because a tiny number of steps can still
/// expand into a very large state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunLimits {
    steps: StepLimit,
    state_len: StateByteLimit,
    return_len: ReturnByteLimit,
    trace_snapshot_len: TraceSnapshotByteLimit,
}

impl RunLimits {
    /// Creates limits with an explicit step limit and default byte budgets.
    #[must_use]
    pub const fn new(max_steps: StepLimit) -> Self {
        Self {
            steps: max_steps,
            state_len: DEFAULT_MAX_STATE_LEN,
            return_len: DEFAULT_MAX_RETURN_LEN,
            trace_snapshot_len: DEFAULT_MAX_TRACE_SNAPSHOT_LEN,
        }
    }

    /// Creates limits with every budget specified explicitly.
    #[must_use]
    pub const fn bounded(
        max_steps: StepLimit,
        max_state_len: StateByteLimit,
        max_return_len: ReturnByteLimit,
        max_trace_snapshot_len: TraceSnapshotByteLimit,
    ) -> Self {
        Self {
            steps: max_steps,
            state_len: max_state_len,
            return_len: max_return_len,
            trace_snapshot_len: max_trace_snapshot_len,
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

    /// Maximum state/output byte length materialized for one trace snapshot event.
    #[must_use]
    pub const fn trace_snapshot_byte_limit(self) -> TraceSnapshotByteLimit {
        self.trace_snapshot_len
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

    /// Returns limits with a different trace snapshot byte budget.
    #[must_use]
    pub const fn with_trace_snapshot_byte_limit(
        mut self,
        max_trace_snapshot_len: TraceSnapshotByteLimit,
    ) -> Self {
        self.trace_snapshot_len = max_trace_snapshot_len;
        self
    }
}

impl Default for RunLimits {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_STEPS)
    }
}

enum TraceSnapshotError<E> {
    Run(RunError),
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
}

impl RuleSet {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push_parsed_rule(&mut self, parsed: ParsedRule) -> Result<(), AllocationError> {
        let position = RulePosition::from_zero_based(self.rules.len())
            .ok_or_else(|| AllocationError::capacity_overflow(AllocationContext::ProgramRules))?;

        try_push(
            &mut self.rules,
            Rule::from_parsed(parsed, position),
            AllocationContext::ProgramRules,
        )?;

        Ok(())
    }

    pub(crate) fn rule_count(&self) -> RuleCount {
        RuleCount::new(self.rules.len())
    }

    pub(crate) fn once_rule_count(&self) -> RuleCount {
        RuleCount::new(
            self.rules
                .iter()
                .filter(|rule| rule.repeat().is_once())
                .count(),
        )
    }

    pub(crate) fn as_slice(&self) -> &[Rule] {
        &self.rules
    }
}

impl Program {
    pub(crate) fn from_rule_set(rule_set: RuleSet) -> Self {
        Self { rule_set }
    }

    /// Parses program source bytes into a reusable program value.
    ///
    /// This is the primary constructor because A=B source is a byte format:
    /// comments may contain non-UTF-8 bytes even though executable code may not.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` when executable code is not ASCII printable syntax,
    /// when a non-empty code line does not contain exactly one `=`, when
    /// reserved syntax appears as payload data, or when allocation fails while
    /// building the parsed program.
    pub fn parse_bytes(source: &[u8]) -> Result<Self, ParseError> {
        parse_program_impl(source)
    }

    /// Parses a UTF-8 source string into a reusable program value.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` for the same syntax and allocation failures as
    /// `Program::parse_bytes`.
    pub fn parse_str(source: &str) -> Result<Self, ParseError> {
        Self::parse_bytes(source.as_bytes())
    }

    /// Returns the number of executable rules in the parsed program.
    #[must_use]
    pub fn rule_count(&self) -> RuleCount {
        self.rule_set.rule_count()
    }

    /// Returns the number of `(once)` rules that need runtime state.
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

    /// Runs this program with the given input bytes.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when `input` contains non-ASCII bytes, an allocation
    /// fails, state-size arithmetic overflows, or a configured `RunLimits`
    /// budget would be exceeded.
    pub fn run(&self, input: &[u8], limits: RunLimits) -> Result<RunResult, RunError> {
        Runtime::new(self, input, limits)?.run()
    }

    /// Runs this program and emits trace-snapshot, infallible events.
    ///
    /// This convenience API materializes `Vec<u8>` snapshots. Use
    /// `run_with_borrowed_trace` when the trace sink only needs to inspect each
    /// event during the callback.
    ///
    /// # Errors
    ///
    /// Returns `RunError` for ordinary runtime failures. Trace snapshot
    /// materialization is also checked against `RunLimits` and may return
    /// `RunError::Limit` or `RunError::Allocation`.
    pub fn run_with_trace_snapshots<'program, F>(
        &'program self,
        input: &[u8],
        limits: RunLimits,
        mut trace: F,
    ) -> Result<RunResult, RunError>
    where
        F: FnMut(TraceSnapshotEvent<'program>),
    {
        match self.try_run_with_trace_snapshots(input, limits, |event| {
            trace(event);
            Ok::<(), Infallible>(())
        }) {
            Ok(result) => Ok(result),
            Err(TracedRunError::Run(error)) => Err(error),
            Err(TracedRunError::Trace(error)) => match error {},
        }
    }

    /// Runs this program and emits trace-snapshot, fallible events.
    ///
    /// # Errors
    ///
    /// Returns `TracedRunError::Run` for runtime failures, including trace
    /// snapshot allocation or snapshot-size failures. Returns
    /// `TracedRunError::Trace` when the user-provided trace callback returns an
    /// error.
    pub fn try_run_with_trace_snapshots<'program, F, E>(
        &'program self,
        input: &[u8],
        limits: RunLimits,
        mut trace: F,
    ) -> Result<RunResult, TracedRunError<E>>
    where
        F: FnMut(TraceSnapshotEvent<'program>) -> Result<(), E>,
    {
        let result = self.try_run_with_borrowed_trace(input, limits, |event| {
            let snapshot = event.to_snapshot(limits).map_err(TraceSnapshotError::Run)?;
            trace(snapshot).map_err(TraceSnapshotError::Trace)
        });

        match result {
            Ok(result) => Ok(result),
            Err(
                TracedRunError::Run(error) | TracedRunError::Trace(TraceSnapshotError::Run(error)),
            ) => Err(TracedRunError::Run(error)),
            Err(TracedRunError::Trace(TraceSnapshotError::Trace(error))) => {
                Err(TracedRunError::Trace(error))
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
        input: &[u8],
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
        input: &[u8],
        limits: RunLimits,
        trace: F,
    ) -> Result<RunResult, TracedRunError<E>>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), E>,
    {
        Runtime::new(self, input, limits)
            .map_err(TracedRunError::Run)?
            .run_with_borrowed_trace(trace)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RunOutcome {
    /// No rule matched the final runtime state.
    Stable(RuntimeStateSnapshot),
    /// A matched rule executed the `(return)` action.
    Return(ReturnOutput),
}

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
    pub fn byte_count(&self) -> ByteCount {
        ByteCount::new(self.bytes.len())
    }

    /// Whether this snapshot contains no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

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
    pub fn byte_count(&self) -> ByteCount {
        ByteCount::new(self.bytes.len())
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
        result_bytes,
    };
    use crate::{
        ByteCount, LimitError, ReturnByteLimit, RuleActionView, RuleAnchor, RuleCount, RuleRepeat,
        StateByteLimit, StateLimitContext, TraceSnapshotByteLimit, TraceSnapshotEffect,
        TraceSnapshotEvent, run_bytes, run_str,
    };
    use std::vec::Vec;

    fn expect_rule(program: &Program, index: usize) -> Result<RuleView<'_>, TestFailure> {
        program
            .rules()
            .nth(index)
            .ok_or(TestFailure::Message("expected parsed rule"))
    }

    #[test]
    fn public_free_run_works() -> TestResult {
        let result = run_str("a=b", b"a", RunLimits::default())?;
        expect_stable_output(&result, b"b")?;
        ensure_eq(result.steps().get(), 1)?;

        let result = run_bytes(b"a=b#\xff", b"a", RunLimits::default())?;
        expect_stable_output(&result, b"b")?;
        Ok(())
    }

    #[test]
    fn parsed_program_is_reusable_and_once_state_is_per_run() -> TestResult {
        let program = Program::parse_str("(once)a=b\na=c")?;

        let first = program.run(b"aa", RunLimits::new(StepLimit::new(10_000)))?;
        let second = program.run(b"aa", RunLimits::new(StepLimit::new(10_000)))?;

        ensure_eq(result_bytes(&first), b"bc".as_slice())?;
        ensure_eq(result_bytes(&second), b"bc".as_slice())?;
        ensure_eq(program.once_rule_count(), RuleCount::new(1))?;
        Ok(())
    }

    #[test]
    fn run_outcome_separates_stable_state_from_return_output() -> TestResult {
        let stable = Program::parse_str("a=b")?.run(b"a", RunLimits::new(StepLimit::new(1)))?;
        let returned =
            Program::parse_str("a=(return)b")?.run(b"a", RunLimits::new(StepLimit::new(1)))?;

        match stable.into_outcome() {
            RunOutcome::Stable(output) => {
                ensure_eq(output.as_bytes(), b"b".as_slice())?;
                ensure_eq(output.byte_count(), ByteCount::new(1))?;
            }
            RunOutcome::Return(_) => return Err(TestFailure::Message("expected stable outcome")),
        }

        match returned.into_outcome() {
            RunOutcome::Return(output) => {
                ensure_eq(output.as_bytes(), b"b".as_slice())?;
                ensure_eq(output.byte_count(), ByteCount::new(1))?;
            }
            RunOutcome::Stable(_) => return Err(TestFailure::Message("expected return outcome")),
        }

        Ok(())
    }

    #[test]
    fn rule_view_generates_canonical_source_without_stored_source_blob() -> TestResult {
        let program = Program::parse_str("a = b # comment\n(start)c=(end)d")?;
        let rules = program.rules().collect::<Vec<_>>();

        ensure_eq(rules.len(), 2)?;
        let first = rules
            .first()
            .copied()
            .ok_or(TestFailure::Message("expected first rule"))?;
        let second = rules
            .get(1)
            .copied()
            .ok_or(TestFailure::Message("expected second rule"))?;

        ensure_eq(first.position().number().get(), 1)?;
        ensure_eq(first.line_number().get(), 1)?;
        ensure_eq(first.repeat(), RuleRepeat::Always)?;
        ensure_eq(first.anchor(), RuleAnchor::Anywhere)?;
        ensure(first.lhs().eq_bytes(b"a"), "expected first lhs")?;
        ensure_matches(
            matches!(
                first.action(),
                RuleActionView::Replace(payload) if payload.eq_bytes(b"b")
            ),
            "expected replace action",
        )?;
        ensure_eq(first.canonical_source()?, b"a=b".as_slice())?;

        ensure_eq(second.position().number().get(), 2)?;
        ensure_eq(second.line_number().get(), 2)?;
        ensure_eq(second.repeat(), RuleRepeat::Always)?;
        ensure_eq(second.anchor(), RuleAnchor::Start)?;
        ensure(second.lhs().eq_bytes(b"c"), "expected second lhs")?;
        ensure_matches(
            matches!(
                second.action(),
                RuleActionView::MoveEnd(payload) if payload.eq_bytes(b"d")
            ),
            "expected move-end action",
        )?;
        ensure_eq(second.canonical_source()?, b"(start)c=(end)d".as_slice())?;
        Ok(())
    }

    #[test]
    fn canonical_source_reparses_to_the_same_executable_rule() -> TestResult {
        let program = Program::parse_str("( once ) ( start ) a = ( end ) b # comment")?;
        let canonical = expect_rule(&program, 0)?.canonical_source()?;

        let reparsed = Program::parse_bytes(canonical.as_slice())?;
        let reparsed_rule = expect_rule(&reparsed, 0)?;

        ensure_eq(reparsed.rule_count(), RuleCount::new(1))?;
        ensure_eq(reparsed.once_rule_count(), RuleCount::new(1))?;
        ensure_eq(reparsed_rule.repeat(), RuleRepeat::Once)?;
        ensure_eq(reparsed_rule.anchor(), RuleAnchor::Start)?;
        ensure(reparsed_rule.lhs().eq_bytes(b"a"), "expected lhs")?;
        ensure_eq(
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

                            ensure_eq(program.rule_count(), RuleCount::new(1))?;
                            ensure_eq(canonical.as_slice(), source.as_slice())?;

                            let reparsed = Program::parse_bytes(&canonical)?;
                            let reparsed_rule = expect_rule(&reparsed, 0)?;
                            ensure_eq(reparsed_rule.canonical_source()?, source.as_slice())?;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    #[test]
    fn state_limit_rejects_oversized_input_before_runtime_allocation() -> TestResult {
        let limits = RunLimits::bounded(
            StepLimit::new(10),
            StateByteLimit::new(1),
            ReturnByteLimit::new(10),
            TraceSnapshotByteLimit::new(10),
        );
        let error = expect_run_error(Program::parse_str("a=b")?.run(b"aa", limits))?;
        let error = expect_state_limit(error)?;

        ensure_eq(
            error,
            LimitError::State {
                context: StateLimitContext::Input,
                limit: StateByteLimit::new(1),
                attempted_len: ByteCount::new(2),
            },
        )?;
        Ok(())
    }

    #[test]
    fn state_limit_rejects_oversized_rewrite_before_allocating_next_state() -> TestResult {
        let limits = RunLimits::bounded(
            StepLimit::new(10),
            StateByteLimit::new(2),
            ReturnByteLimit::new(10),
            TraceSnapshotByteLimit::new(10),
        );
        let error = expect_run_error(Program::parse_str("=a")?.run(b"aa", limits))?;
        let error = expect_state_limit(error)?;

        ensure_eq(
            error,
            LimitError::State {
                context: StateLimitContext::Rewrite,
                limit: StateByteLimit::new(2),
                attempted_len: ByteCount::new(3),
            },
        )?;
        Ok(())
    }

    #[test]
    fn trace_snapshots_are_derived_from_borrowed_trace() -> TestResult {
        let program = Program::parse_str("a=b\nb=(return)ok")?;
        let mut events = Vec::new();
        let result = program.run_with_trace_snapshots(
            b"a",
            RunLimits::new(StepLimit::new(10_000)),
            |event| {
                events.push(event);
            },
        )?;

        expect_return_output(&result, b"ok")?;
        ensure_eq(events.len(), 3)?;
        ensure_matches(
            matches!(events.first(), Some(TraceSnapshotEvent::Initial { .. })),
            "expected initial trace event",
        )?;
        let initial = expect_event(&events, 0)?;
        let first_step = expect_event(&events, 1)?;
        let second_step = expect_event(&events, 2)?;

        ensure_eq(initial.as_bytes(), b"a".as_slice())?;
        ensure_eq(first_step.as_bytes(), b"b".as_slice())?;
        ensure_eq(second_step.as_bytes(), b"ok".as_slice())?;
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
                ensure_eq(state.as_bytes(), b"b".as_slice())?;
                ensure_eq(rule.canonical_source()?, b"a=b".as_slice())?;
            }
            TraceSnapshotEvent::Initial { .. } | TraceSnapshotEvent::Step { .. } => {
                return Err(TestFailure::Message("expected continue step"));
            }
        }
        Ok(())
    }
}
