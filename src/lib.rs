//! Byte-oriented interpreter for A=B ordered rewrite programs.
//!
//! This crate-level documentation is the docs.rs API guide. It shows the
//! primary execution, stepping, inspection, tracing, limit, and error surfaces.
//! The crate README carries the longer language reference and project overview.
//!
//! `rsaeb` is a `no_std + alloc` library crate. It parses compact A=B source
//! into an immutable [`Program`] and runs that program against typed
//! [`RuntimeInput`] validated before execution. Files, stdout, stderr,
//! arguments, environment access, and lossy display formatting are outside the
//! interpreter core.
//!
//! # Core boundary
//!
//! Program source and runtime input enter through separate typed boundaries:
//!
//! - [`ProgramSource`] labels bytes as A=B source before [`Program::parse`];
//! - [`RuntimeInput`] owns already-validated ASCII input bytes;
//! - [`RunLimits`] bounds each runtime invocation;
//! - trace snapshots use [`limits::TraceSnapshotByteLimit`] separately from
//!   runtime execution limits.
//!
//! # Basic execution
//!
//! Parse [`ProgramSource`] and [`RuntimeInput`] explicitly before running:
//!
//! ```
//! use rsaeb::limits::{
//!     DEFAULT_MAX_INPUT_LEN, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_STEPS,
//! };
//! use rsaeb::{Program, ProgramSource, RunLimits, RunOutcome, RuntimeInput};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::parse(ProgramSource::from_str("a=b"))?;
//! let limits = RunLimits::new(DEFAULT_MAX_STEPS, DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN);
//! let input = RuntimeInput::validate(b"a", DEFAULT_MAX_INPUT_LEN)?;
//! let result = program.run(&input, limits)?;
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
//! `(once)` state is owned by each runtime invocation, not by the parsed
//! program:
//!
//! ```
//! use rsaeb::limits::{
//!     DEFAULT_MAX_INPUT_LEN, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, StepLimit,
//! };
//! use rsaeb::{
//!     Program, ProgramSource, RunLimits, RunOutcome, RuntimeInput,
//! };
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::parse(ProgramSource::from_str("(once)a=b\na=c"))?;
//! let limits = RunLimits::new(StepLimit::new(10_000), DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN);
//! let input = RuntimeInput::validate(b"aa", DEFAULT_MAX_INPUT_LEN)?;
//!
//! let first = program.run(&input, limits)?;
//! let second = program.run(&input, limits)?;
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
//! use rsaeb::execution::ExecutionTransition;
//! use rsaeb::limits::{
//!     DEFAULT_MAX_INPUT_LEN, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, StepLimit,
//! };
//! use rsaeb::{Program, ProgramSource, RunLimits, RuntimeInput};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::parse(ProgramSource::from_str("a=b\nb=c"))?;
//! let limits = RunLimits::new(StepLimit::new(10), DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN);
//! let input = RuntimeInput::validate(b"a", DEFAULT_MAX_INPUT_LEN)?;
//! let execution = program.start_execution(
//!     &input,
//!     limits,
//! )?;
//!
//! let execution = match execution.step().map_err(|step| step.into_error())? {
//!     ExecutionTransition::Applied(applied) => {
//!         assert!(applied.state().bytes().eq(b"b".iter().copied()));
//!         applied.into_running()
//!     }
//!     ExecutionTransition::Stable(_) | ExecutionTransition::Returned(_) => {
//!         return Err("expected first applied step".into());
//!     }
//! };
//!
//! let execution = match execution.step().map_err(|step| step.into_error())? {
//!     ExecutionTransition::Applied(applied) => {
//!         assert!(applied.state().bytes().eq(b"c".iter().copied()));
//!         applied.into_running()
//!     }
//!     ExecutionTransition::Stable(_) | ExecutionTransition::Returned(_) => {
//!         return Err("expected second applied step".into());
//!     }
//! };
//!
//! match execution.step().map_err(|step| step.into_error())? {
//!     ExecutionTransition::Stable(stable) => {
//!         assert_eq!(stable.steps().get(), 2);
//!         assert!(stable.state().bytes().eq(b"c".iter().copied()));
//!     }
//!     ExecutionTransition::Applied(_) | ExecutionTransition::Returned(_) => {
//!         return Err("expected stable completion".into());
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Limits
//!
//! [`limits::RuntimeInputByteLimit`] bounds owned input classification before
//! allocation.
//! [`RunLimits`] carries the step budget and byte budgets for runtime states and
//! `(return)` outputs. Trace snapshot materialization uses an explicit
//! [`limits::TraceSnapshotByteLimit`]. Step limits are checked only when another
//! matching rule would apply after the configured number of completed steps:
//!
//! ```
//! use rsaeb::error::{LimitError, RunError};
//! use rsaeb::limits::{
//!     DEFAULT_MAX_INPUT_LEN, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, StepLimit,
//! };
//! use rsaeb::{Program, ProgramSource, RunLimits, RuntimeInput};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let limits = RunLimits::new(StepLimit::new(0), DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN);
//! let input = RuntimeInput::validate(b"a", DEFAULT_MAX_INPUT_LEN)?;
//! let result = Program::parse(ProgramSource::from_str("a=b"))?.run(&input, limits);
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
//! # Rule inspection
//!
//! Parsed rules are exposed as borrowed structured views, not as stored source
//! strings:
//!
//! ```
//! use rsaeb::inspect::{RuleActionView, RuleAnchor, RuleRepeat};
//! use rsaeb::{Program, ProgramSource};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::parse(ProgramSource::from_str("( once ) ( start ) a = ( end ) b # comment"))?;
//! let rule = program.rules().next().ok_or("missing parsed rule")?;
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
//! # Tracing
//!
//! Borrowed trace events allocate no snapshots. Snapshot tracing is layered on
//! top when a caller needs owned event bytes:
//!
//! ```
//! use rsaeb::limits::{
//!     DEFAULT_MAX_INPUT_LEN, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, StepLimit,
//! };
//! use rsaeb::trace::BorrowedTraceEvent;
//! use rsaeb::{Program, ProgramSource, RunLimits, RuntimeInput};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::parse(ProgramSource::from_str("a=b\nb=(return)ok"))?;
//! let mut byte_counts = Vec::new();
//! let limits = RunLimits::new(StepLimit::new(10), DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN);
//! let input = RuntimeInput::validate(b"a", DEFAULT_MAX_INPUT_LEN)?;
//!
//! program.run_with_borrowed_trace(
//!     &input,
//!     limits,
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
//! error types such as [`error::ParseError`], [`error::RuntimeInputError`],
//! [`error::RunError`],
//! [`error::TraceSnapshotError`], [`error::TraceSnapshotRunError`],
//! [`error::FallibleTraceSnapshotRunError`], and [`error::TracedRunError`].
//! [`error::AebError`] is available as a parse/input/run umbrella for callers
//! that want one top-level error type.

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
pub mod error;
pub mod execution;
pub mod inspect;
pub mod limits;
mod parser;
mod program;
mod rule;
mod runtime;
pub mod source;
mod syntax;
pub mod trace;

pub use program::{Program, ReturnOutput, RunLimits, RunOutcome, RunResult, RuntimeStateSnapshot};
pub use runtime::RuntimeInput;
pub use source::ProgramSource;
