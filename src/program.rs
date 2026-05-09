use alloc::vec::Vec;
use core::convert::Infallible;

use crate::error::{AebError, ParseError, RunError, TracedRunError};
use crate::parser::parse_program_impl;
use crate::rule::{Rule, RuleInfo, RulePosition};
use crate::runtime::Runtime;
use crate::trace::TraceEvent;

pub const DEFAULT_MAX_STEPS: usize = 1_000_000;
pub fn run(
    source: impl AsRef<[u8]>,
    input: impl AsRef<[u8]>,
    options: RunOptions,
) -> Result<RunResult, AebError> {
    let program = Program::parse(source).map_err(AebError::Parse)?;
    program.run(input, options).map_err(AebError::Run)
}

/// Execution options for one runtime invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunOptions {
    max_steps: usize,
}

impl RunOptions {
    /// Creates options with an explicit step limit.
    ///
    /// A run that becomes stable exactly at this count succeeds. The limit is
    /// an error only when another matching rule would need to be applied after
    /// this many steps.
    #[must_use]
    pub const fn new(max_steps: usize) -> Self {
        Self { max_steps }
    }

    /// Maximum number of rewrite steps that may be applied.
    #[must_use]
    pub const fn max_steps(&self) -> usize {
        self.max_steps
    }
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            max_steps: DEFAULT_MAX_STEPS,
        }
    }
}

/// Parsed A=B rewrite program.
///
/// A parsed program is immutable and reusable. Per-run `(once)` state lives in
/// the runtime invocation, not in this value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub(crate) rules: Vec<Rule>,
}

impl Program {
    /// Parses program source bytes into a reusable program value.
    pub fn parse(source: impl AsRef<[u8]>) -> Result<Self, ParseError> {
        parse_program_impl(source.as_ref())
    }

    /// Parses program source bytes into a reusable program value.
    pub fn parse_bytes(source: &[u8]) -> Result<Self, ParseError> {
        parse_program_impl(source)
    }

    /// Parses a UTF-8 source string into a reusable program value.
    pub fn parse_str(source: &str) -> Result<Self, ParseError> {
        parse_program_impl(source.as_bytes())
    }

    /// Returns the number of executable rules in the parsed program.
    #[must_use]
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Iterates over parsed rule metadata in execution order.
    pub fn rules(&self) -> impl Iterator<Item = RuleInfo<'_>> + '_ {
        self.rules
            .iter()
            .enumerate()
            .map(|(index, rule)| rule.info(RulePosition::new(index)))
    }

    /// Runs this program with the given input bytes.
    pub fn run(&self, input: impl AsRef<[u8]>, options: RunOptions) -> Result<RunResult, RunError> {
        Runtime::new(self, input.as_ref())?.run(options.max_steps())
    }

    /// Runs this program and emits infallible trace events.
    pub fn run_with_trace<'program, F>(
        &'program self,
        input: impl AsRef<[u8]>,
        options: RunOptions,
        mut trace: F,
    ) -> Result<RunResult, RunError>
    where
        F: FnMut(TraceEvent<'program>),
    {
        match self.try_run_with_trace(input, options, |event| {
            trace(event);
            Ok::<(), Infallible>(())
        }) {
            Ok(result) => Ok(result),
            Err(TracedRunError::Run(error)) => Err(error),
            Err(TracedRunError::Trace(error)) => match error {},
        }
    }

    /// Runs this program and emits fallible trace events.
    ///
    /// Trace snapshots are allocated only when tracing is enabled. A failure to
    /// allocate such a snapshot is returned as [`RunError::Allocation`]. A
    /// failure from the user callback is returned separately as
    /// [`TracedRunError::Trace`].
    pub fn try_run_with_trace<'program, F, E>(
        &'program self,
        input: impl AsRef<[u8]>,
        options: RunOptions,
        trace: F,
    ) -> Result<RunResult, TracedRunError<E>>
    where
        F: FnMut(TraceEvent<'program>) -> Result<(), E>,
    {
        Runtime::new(self, input.as_ref())
            .map_err(TracedRunError::Run)?
            .run_with_trace(options.max_steps(), trace)
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
    use crate::test_support::{TestFailure, TestResult};
    use crate::{RunTermination, run};
    use std::vec::Vec;
    #[test]
    fn public_free_run_works() -> TestResult {
        let result = run("a=b", b"a", RunOptions::default())?;
        assert_eq!(result.output(), b"b");
        assert_eq!(result.steps(), 1);
        assert!(!result.returned());
        Ok(())
    }

    #[test]
    fn parsed_program_is_reusable_and_once_state_is_per_run() -> TestResult {
        let program = Program::parse("(once)a=b\na=c")?;

        let first = program.run(b"aa", RunOptions::new(10_000))?;
        let second = program.run(b"aa", RunOptions::new(10_000))?;

        assert_eq!(first.output(), b"bc");
        assert_eq!(second.output(), b"bc");
        Ok(())
    }

    #[test]
    fn rule_metadata_is_exposed_without_embedding_display_strings_in_trace_events() -> TestResult {
        let program = Program::parse("a = b # comment\n(start)c=(end)d")?;
        let rules = program.rules().collect::<Vec<_>>();

        assert_eq!(rules.len(), 2);

        let first = rules
            .first()
            .ok_or(TestFailure::Message("expected first rule"))?;
        let second = rules
            .get(1)
            .ok_or(TestFailure::Message("expected second rule"))?;

        assert_eq!(first.position().zero_based(), 0);
        assert_eq!(first.line_number(), 1);
        assert_eq!(first.compact_source(), b"a=b");
        assert_eq!(second.position().zero_based(), 1);
        assert_eq!(second.line_number(), 2);
        assert_eq!(second.compact_source(), b"(start)c=(end)d");
        Ok(())
    }

    #[test]
    fn empty_program_returns_input_unchanged() -> TestResult {
        let result = Program::parse("")?.run(b"a=()#c", RunOptions::new(0))?;

        assert_eq!(result.output(), b"a=()#c");
        assert_eq!(result.steps(), 0);
        assert_eq!(result.termination(), RunTermination::Stable);
        Ok(())
    }
}
