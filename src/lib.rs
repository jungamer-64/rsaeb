//! Byte-oriented interpreter for A=B ordered rewrite programs.
//!
//! `rsaeb` is a `no_std + alloc` library crate. It parses compact A=B source
//! into an immutable [`Program`] and runs that program against validated
//! [`RuntimeInput`]. Files, stdout, stderr, arguments, environment variables,
//! and lossy display formatting are outside the interpreter core.
//!
//! # Domain boundary
//!
//! Program syntax and runtime input are intentionally different byte domains:
//!
//! - program code is compact printable ASCII syntax;
//! - comments are ignored bytes after `#` and may contain arbitrary bytes;
//! - runtime input is ASCII data and may contain whitespace/reserved bytes;
//! - program payloads cannot contain whitespace, reserved syntax characters, or
//!   non-ASCII/control bytes.
//!
//! # Basic execution
//!
//! Use [`run_str`] or [`run_bytes`] for a one-shot parse and run:
//!
//! ```
//! use rsaeb::{RunLimits, RunOutcome, run_str};
//!
//! # fn main() -> Result<(), rsaeb::AebError> {
//! let result = run_str("a=b", b"a", RunLimits::default())?;
//!
//! assert!(matches!(
//!     result.outcome(),
//!     RunOutcome::Stable(output) if output.as_bytes() == b"b"
//! ));
//! # Ok(())
//! # }
//! ```
//!
//! Parse [`Program`] once when the same rules should be reused. Per-run
//! `(once)` state is owned by each runtime invocation, not by the program:
//!
//! ```
//! use rsaeb::{Program, RunLimits, RunOutcome, RuntimeInput, StepLimit};
//!
//! # fn main() -> Result<(), rsaeb::AebError> {
//! let program = Program::parse_str("(once)a=b\na=c")?;
//! let limits = RunLimits::new(StepLimit::new(10_000));
//!
//! let first = program.run(RuntimeInput::parse(b"aa")?, limits)?;
//! let second = program.run(RuntimeInput::parse(b"aa")?, limits)?;
//!
//! assert!(matches!(
//!     first.outcome(),
//!     RunOutcome::Stable(output) if output.as_bytes() == b"bc"
//! ));
//! assert!(matches!(
//!     second.outcome(),
//!     RunOutcome::Stable(output) if output.as_bytes() == b"bc"
//! ));
//! # Ok(())
//! # }
//! ```
//!
//! # Stepwise execution
//!
//! Use [`Program::start_execution`] when a host wants to wait after each
//! applied rule:
//!
//! ```
//! use rsaeb::{
//!     ExecutionCompletion, ExecutionStep, Program, RunLimits, RuntimeInput, StepLimit,
//! };
//!
//! # fn main() -> Result<(), rsaeb::AebError> {
//! let program = Program::parse_str("a=b\nb=c")?;
//! let mut execution = program.start_execution(
//!     RuntimeInput::parse(b"a")?,
//!     RunLimits::new(StepLimit::new(10)),
//! )?;
//!
//! let first = execution.step()?;
//! assert!(matches!(
//!     first,
//!     ExecutionStep::Applied { effect, .. }
//!         if effect.state().bytes().eq(b"b".iter().copied())
//! ));
//!
//! let second = execution.step()?;
//! assert!(matches!(
//!     second,
//!     ExecutionStep::Applied { effect, .. }
//!         if effect.state().bytes().eq(b"c".iter().copied())
//! ));
//!
//! let completed = execution.step()?;
//! assert!(matches!(
//!     completed,
//!     ExecutionStep::Complete(ExecutionCompletion::Stable { steps, state })
//!         if steps.get() == 2 && state.bytes().eq(b"c".iter().copied())
//! ));
//! # Ok(())
//! # }
//! ```
//!
//! # Limits
//!
//! [`RunLimits`] carries the step budget and byte budgets for runtime states
//! and `(return)` outputs. Trace snapshot materialization uses an explicit
//! [`TraceSnapshotByteLimit`]. Step limits are checked only when another
//! matching rule would apply after the configured number of completed steps:
//!
//! ```
//! use rsaeb::{LimitError, Program, RunError, RunLimits, RuntimeInput, StepLimit};
//!
//! # fn main() -> Result<(), rsaeb::AebError> {
//! let result = Program::parse_str("a=b")?.run(
//!     RuntimeInput::parse(b"a")?,
//!     RunLimits::new(StepLimit::new(0)),
//! );
//!
//! assert!(matches!(
//!     result,
//!     Err(RunError::Limit(LimitError::Step { completed_steps, .. }))
//!         if completed_steps.get() == 0
//! ));
//! # Ok(())
//! # }
//! ```
//!
//! # Rule inspection and tracing
//!
//! Parsed rules are exposed as borrowed structured views, not as stored source
//! strings:
//!
//! ```
//! use rsaeb::{Program, RuleActionView, RuleAnchor, RuleRepeat};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::parse_str("( once ) ( start ) a = ( end ) b # comment")?;
//! let rule = program.rules().next().expect("one parsed rule");
//!
//! assert_eq!(rule.repeat(), RuleRepeat::Once);
//! assert_eq!(rule.anchor(), RuleAnchor::Start);
//! assert!(rule.lhs().eq_bytes(b"a"));
//! assert!(matches!(
//!     rule.action(),
//!     RuleActionView::MoveEnd(payload) if payload.eq_bytes(b"b")
//! ));
//! assert_eq!(rule.canonical_source()?, b"(once)(start)a=(end)b");
//! # Ok(())
//! # }
//! ```
//!
//! Borrowed trace events allocate no snapshots. Snapshot tracing is layered on
//! top when a caller needs owned event bytes:
//!
//! ```
//! use rsaeb::{BorrowedTraceEvent, Program, RunLimits, RuntimeInput, StepLimit};
//!
//! # fn main() -> Result<(), rsaeb::AebError> {
//! let program = Program::parse_str("a=b\nb=(return)ok")?;
//! let mut byte_counts = Vec::new();
//!
//! program.run_with_borrowed_trace(
//!     RuntimeInput::parse(b"a")?,
//!     RunLimits::new(StepLimit::new(10)),
//!     |event| {
//!         byte_counts.push(event.byte_count().get());
//!         if let BorrowedTraceEvent::Step { rule, .. } = event {
//!             let _line = rule.line_number();
//!         }
//!     },
//! )?;
//!
//! assert_eq!(byte_counts, [1, 1, 2]);
//! # Ok(())
//! # }
//! ```
//!
//! # Error model
//!
//! Source parsing, runtime input validation, runtime execution, trace snapshot
//! materialization, and user trace-sink failures are reported with structured
//! error types such as [`ParseError`], [`InputError`], [`RunError`],
//! [`TraceSnapshotError`], [`TraceSnapshotRunError`],
//! [`FallibleTraceSnapshotRunError`], and [`TracedRunError`]. [`AebError`] is
//! the convenience umbrella used by one-shot helpers.

#![no_std]
#![forbid(unsafe_code)]
#![deny(
    missing_docs,
    rustdoc::broken_intra_doc_links,
    rustdoc::bare_urls,
    unconditional_panic,
    clippy::panic,
    clippy::panic_in_result_fn,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::todo,
    clippy::unimplemented,
    clippy::unreachable,
    clippy::arithmetic_side_effects
)]

extern crate alloc;

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod test_support;

mod allocation;
mod bytes;
mod error;
mod parser;
mod program;
mod rule;
mod runtime;
mod source;
mod syntax;
mod trace;

pub use allocation::{AllocationContext, AllocationError, AllocationErrorKind};
pub use bytes::{
    NonAsciiCodeByte, NonAsciiInputByte, NonPrintableCodeByte, PayloadByteCount,
    ReservedSyntaxByte, ReturnOutputByteCount, RuntimeStateByteCount, TraceSnapshotByteCount,
};
pub use error::{
    AebError, FallibleTraceSnapshotRunError, InputColumn, InputError, LeftModifierKind, LimitError,
    ParseError, ParseErrorKind, ParseErrorLocation, PayloadKind, RightActionKind, RunError,
    StateLimitContext, StateSizeError, TraceSnapshotError, TraceSnapshotRunError, TracedRunError,
};
pub use program::{
    DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_STEPS,
    DEFAULT_MAX_TRACE_SNAPSHOT_LEN, Program, ReturnByteLimit, ReturnOutput, RunLimits, RunOutcome,
    RunResult, RuntimeStateSnapshot, StateByteLimit, StepCount, StepLimit, TraceSnapshotByteLimit,
    run_bytes, run_str,
};
pub use rule::{
    PayloadView, RuleActionView, RuleAnchor, RuleCount, RuleNumber, RulePosition, RuleRepeat,
    RuleView,
};
pub use runtime::{Execution, ExecutionCompletion, ExecutionEffect, ExecutionStep, RuntimeInput};
pub use source::{SourceColumn, SourceLineNumber, SourcePosition};
pub use trace::{
    BorrowedTraceEffect, BorrowedTraceEvent, RuntimeStateView, TraceSnapshotEffect,
    TraceSnapshotEvent,
};
