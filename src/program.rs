use alloc::vec::Vec;
use core::convert::Infallible;

use crate::allocation::{AllocationContext, AllocationError, try_push};
use crate::bytes::Payload;
use crate::error::{AebError, ParseError, RunError, TracedRunError};
use crate::parser::parse_program_impl;
use crate::rule::{
    Action, OnceRuleSlot, Rule, RuleAnchor, RulePosition, RuleRepeat, RuleRepeatPlan, RuleView,
};
use crate::runtime::Runtime;
use crate::trace::{BorrowedTraceEvent, TraceSnapshotEvent};

pub const DEFAULT_MAX_STEPS: usize = 1_000_000;
pub const DEFAULT_MAX_STATE_LEN: usize = 16 * 1024 * 1024;
pub const DEFAULT_MAX_RETURN_LEN: usize = 16 * 1024 * 1024;
pub const DEFAULT_MAX_TRACE_SNAPSHOT_LEN: usize = 16 * 1024 * 1024;

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
    steps: usize,
    state_len: usize,
    return_len: usize,
    trace_snapshot_len: usize,
}

impl RunLimits {
    /// Creates limits with an explicit step limit and default byte budgets.
    #[must_use]
    pub const fn new(max_steps: usize) -> Self {
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
        max_steps: usize,
        max_state_len: usize,
        max_return_len: usize,
        max_trace_snapshot_len: usize,
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
    pub const fn max_steps(self) -> usize {
        self.steps
    }

    /// Maximum runtime state length, including initial input and rewrite results.
    #[must_use]
    pub const fn max_state_len(self) -> usize {
        self.state_len
    }

    /// Maximum byte length accepted for `(return)` output.
    #[must_use]
    pub const fn max_return_len(self) -> usize {
        self.return_len
    }

    /// Maximum state/output byte length materialized for one trace snapshot event.
    #[must_use]
    pub const fn max_trace_snapshot_len(self) -> usize {
        self.trace_snapshot_len
    }

    /// Returns limits with a different step budget.
    #[must_use]
    pub const fn with_max_steps(mut self, max_steps: usize) -> Self {
        self.steps = max_steps;
        self
    }

    /// Returns limits with a different runtime-state budget.
    #[must_use]
    pub const fn with_max_state_len(mut self, max_state_len: usize) -> Self {
        self.state_len = max_state_len;
        self
    }

    /// Returns limits with a different return-output budget.
    #[must_use]
    pub const fn with_max_return_len(mut self, max_return_len: usize) -> Self {
        self.return_len = max_return_len;
        self
    }

    /// Returns limits with a different trace snapshot byte budget.
    #[must_use]
    pub const fn with_max_trace_snapshot_len(mut self, max_trace_snapshot_len: usize) -> Self {
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

    pub(crate) fn push_rule(
        &mut self,
        line_number: usize,
        repeat: RuleRepeat,
        anchor: RuleAnchor,
        lhs: Payload,
        action: Action,
    ) -> Result<(), AllocationError> {
        let repeat = if repeat.is_once() {
            RuleRepeatPlan::once(self.next_once_rule_slot())
        } else {
            RuleRepeatPlan::always()
        };

        try_push(
            &mut self.rules,
            Rule::new(line_number, repeat, anchor, lhs, action),
            AllocationContext::ProgramRules,
        )?;

        Ok(())
    }

    fn next_once_rule_slot(&self) -> OnceRuleSlot {
        OnceRuleSlot::new(self.once_rule_count())
    }

    pub(crate) fn rule_count(&self) -> usize {
        self.rules.len()
    }

    pub(crate) fn once_rule_count(&self) -> usize {
        self.rules
            .iter()
            .filter(|rule| rule.repeat().is_once())
            .count()
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
    pub fn rule_count(&self) -> usize {
        self.rule_set.rule_count()
    }

    /// Returns the number of `(once)` rules that need runtime state.
    #[must_use]
    pub fn once_rule_count(&self) -> usize {
        self.rule_set.once_rule_count()
    }

    /// Iterates over structured parsed-rule views in execution order.
    pub fn rules(&self) -> impl Iterator<Item = RuleView<'_>> + '_ {
        self.rule_set
            .as_slice()
            .iter()
            .enumerate()
            .map(|(index, rule)| rule.view(RulePosition::new(index)))
    }

    pub(crate) fn rule_slice(&self) -> &[Rule] {
        self.rule_set.as_slice()
    }

    pub(crate) fn runtime_once_rule_count(&self) -> usize {
        self.rule_set.once_rule_count()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunTermination {
    /// No rule matched the final runtime state.
    Stable,
    /// A matched rule executed the `(return)` action.
    Return,
}

impl RunTermination {
    /// Whether this termination came from `(return)`.
    #[must_use]
    pub const fn is_return(self) -> bool {
        matches!(self, Self::Return)
    }
}

/// Result of one program execution.
#[derive(Debug, PartialEq, Eq)]
pub struct RunResult {
    output: Vec<u8>,
    steps: usize,
    termination: RunTermination,
}

impl RunResult {
    pub(crate) fn stable(output: Vec<u8>, steps: usize) -> Self {
        Self {
            output,
            steps,
            termination: RunTermination::Stable,
        }
    }

    pub(crate) fn from_return(output: Vec<u8>, steps: usize) -> Self {
        Self {
            output,
            steps,
            termination: RunTermination::Return,
        }
    }

    /// Final output bytes.
    #[must_use]
    pub fn output(&self) -> &[u8] {
        &self.output
    }

    /// Consumes the result and returns final output bytes.
    #[must_use]
    pub fn into_output(self) -> Vec<u8> {
        self.output
    }

    /// Number of rewrite steps applied.
    #[must_use]
    pub const fn steps(&self) -> usize {
        self.steps
    }

    /// Structured termination reason.
    #[must_use]
    pub const fn termination(&self) -> RunTermination {
        self.termination
    }

    /// Whether execution stopped by `(return)`.
    #[must_use]
    pub const fn returned(&self) -> bool {
        self.termination.is_return()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        TestFailure, TestResult, expect_event, expect_run_error, expect_state_limit,
    };
    use crate::{
        LimitError, RuleActionView, RuleAnchor, RuleRepeat, StateLimitContext, TraceSnapshotEffect,
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
        assert_eq!(result.output(), b"b");
        assert_eq!(result.steps(), 1);
        assert!(!result.returned());

        let result = run_bytes(b"a=b#\xff", b"a", RunLimits::default())?;
        assert_eq!(result.output(), b"b");
        Ok(())
    }

    #[test]
    fn parsed_program_is_reusable_and_once_state_is_per_run() -> TestResult {
        let program = Program::parse_str("(once)a=b\na=c")?;

        let first = program.run(b"aa", RunLimits::new(10_000))?;
        let second = program.run(b"aa", RunLimits::new(10_000))?;

        assert_eq!(first.output(), b"bc");
        assert_eq!(second.output(), b"bc");
        assert_eq!(program.once_rule_count(), 1);
        Ok(())
    }

    #[test]
    fn rule_view_generates_canonical_source_without_stored_source_blob() -> TestResult {
        let program = Program::parse_str("a = b # comment\n(start)c=(end)d")?;
        let rules = program.rules().collect::<Vec<_>>();

        assert_eq!(rules.len(), 2);
        let first = rules
            .first()
            .copied()
            .ok_or(TestFailure::Message("expected first rule"))?;
        let second = rules
            .get(1)
            .copied()
            .ok_or(TestFailure::Message("expected second rule"))?;

        assert_eq!(first.position().zero_based(), 0);
        assert_eq!(first.line_number(), 1);
        assert_eq!(first.repeat(), RuleRepeat::Always);
        assert_eq!(first.anchor(), RuleAnchor::Anywhere);
        assert!(first.lhs().eq_bytes(b"a"));
        assert!(matches!(
            first.action(),
            RuleActionView::Replace(payload) if payload.eq_bytes(b"b")
        ));
        assert_eq!(first.canonical_source()?, b"a=b");

        assert_eq!(second.position().zero_based(), 1);
        assert_eq!(second.line_number(), 2);
        assert_eq!(second.repeat(), RuleRepeat::Always);
        assert_eq!(second.anchor(), RuleAnchor::Start);
        assert!(second.lhs().eq_bytes(b"c"));
        assert!(matches!(
            second.action(),
            RuleActionView::MoveEnd(payload) if payload.eq_bytes(b"d")
        ));
        assert_eq!(second.canonical_source()?, b"(start)c=(end)d");
        Ok(())
    }

    #[test]
    fn canonical_source_reparses_to_the_same_executable_rule() -> TestResult {
        let program = Program::parse_str("( once ) ( start ) a = ( end ) b # comment")?;
        let canonical = expect_rule(&program, 0)?.canonical_source()?;

        let reparsed = Program::parse_bytes(canonical.as_slice())?;
        let reparsed_rule = expect_rule(&reparsed, 0)?;

        assert_eq!(reparsed.rule_count(), 1);
        assert_eq!(reparsed.once_rule_count(), 1);
        assert_eq!(reparsed_rule.repeat(), RuleRepeat::Once);
        assert_eq!(reparsed_rule.anchor(), RuleAnchor::Start);
        assert!(reparsed_rule.lhs().eq_bytes(b"a"));
        assert_eq!(reparsed_rule.canonical_source()?, b"(once)(start)a=(end)b");
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

                            assert_eq!(program.rule_count(), 1);
                            assert_eq!(canonical, source, "source: {source:?}");

                            let reparsed = Program::parse_bytes(&canonical)?;
                            let reparsed_rule = expect_rule(&reparsed, 0)?;
                            assert_eq!(
                                reparsed_rule.canonical_source()?,
                                source,
                                "source: {source:?}",
                            );
                        }
                    }
                }
            }
        }

        Ok(())
    }

    #[test]
    fn state_limit_rejects_oversized_input_before_runtime_allocation() -> TestResult {
        let limits = RunLimits::bounded(10, 1, 10, 10);
        let error = expect_run_error(Program::parse_str("a=b")?.run(b"aa", limits))?;
        let error = expect_state_limit(error)?;

        assert_eq!(
            error,
            LimitError::State {
                context: StateLimitContext::Input,
                limit: 1,
                attempted_len: 2,
            },
        );
        Ok(())
    }

    #[test]
    fn state_limit_rejects_oversized_rewrite_before_allocating_next_state() -> TestResult {
        let limits = RunLimits::bounded(10, 2, 10, 10);
        let error = expect_run_error(Program::parse_str("=a")?.run(b"aa", limits))?;
        let error = expect_state_limit(error)?;

        assert_eq!(
            error,
            LimitError::State {
                context: StateLimitContext::Rewrite,
                limit: 2,
                attempted_len: 3,
            },
        );
        Ok(())
    }

    #[test]
    fn trace_snapshots_are_derived_from_borrowed_trace() -> TestResult {
        let program = Program::parse_str("a=b\nb=(return)ok")?;
        let mut events = Vec::new();
        let result = program.run_with_trace_snapshots(b"a", RunLimits::new(10_000), |event| {
            events.push(event);
        })?;

        assert_eq!(result.output(), b"ok");
        assert!(result.returned());
        assert_eq!(events.len(), 3);
        assert!(matches!(
            events.first(),
            Some(TraceSnapshotEvent::Initial { .. })
        ));
        let initial = expect_event(&events, 0)?;
        let first_step = expect_event(&events, 1)?;
        let second_step = expect_event(&events, 2)?;

        assert_eq!(initial.bytes(), b"a");
        assert_eq!(first_step.bytes(), b"b");
        assert_eq!(second_step.bytes(), b"ok");
        assert!(!first_step.is_return_step());
        assert!(second_step.is_return_step());

        match first_step {
            TraceSnapshotEvent::Step {
                rule,
                effect: TraceSnapshotEffect::Continue { state },
                ..
            } => {
                assert_eq!(state.as_slice(), b"b");
                assert_eq!(rule.canonical_source()?, b"a=b");
            }
            TraceSnapshotEvent::Initial { .. } | TraceSnapshotEvent::Step { .. } => {
                return Err(TestFailure::Message("expected continue step"));
            }
        }
        Ok(())
    }
}
