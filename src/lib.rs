//! Byte-oriented interpreter for A=B ordered rewrite programs.
//!
//! `rsaeb` is a `no_std + alloc` library crate. It parses compact A=B source
//! into an immutable [`Program`] and runs that program against typed
//! [`RuntimeInput`] validated before execution. Files, stdout, stderr,
//! arguments, and lossy display formatting are outside the interpreter core.
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
//! Parse [`ProgramSource`] and [`RuntimeInput`] explicitly before running:
//!
//! ```
//! use rsaeb::{
//!     DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_STEPS, Program, ProgramSource,
//!     RunLimits, RunOutcome, RuntimeInput, RuntimeInputLimits,
//! };
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::parse(ProgramSource::from_str("a=b"))?;
//! let input = RuntimeInput::validate(b"a", RuntimeInputLimits::new(DEFAULT_MAX_STATE_LEN))?;
//! let result = program.run(&input, RunLimits::new(DEFAULT_MAX_STEPS, DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN))?;
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
//! `(once)` state is owned by each runtime invocation, not by the program.
//! Each execution owns runtime rule state derived from the parsed rule list, so
//! `(once)` state cannot drift away from rule order while scanning:
//!
//! ```
//! use rsaeb::limits::StepLimit;
//! use rsaeb::{
//!     DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, Program, ProgramSource, RunLimits,
//!     RunOutcome, RuntimeInput, RuntimeInputLimits,
//! };
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::parse(ProgramSource::from_str("(once)a=b\na=c"))?;
//! let limits = RunLimits::new(StepLimit::new(10_000), DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN);
//! let input = RuntimeInput::validate(
//!     b"aa",
//!     RuntimeInputLimits::new(limits.state_byte_limit()),
//! )?;
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
//! use rsaeb::{
//!     DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, ExecutionTransition, Program, ProgramSource,
//!     RunLimits, RuntimeInput, RuntimeInputLimits,
//! };
//! use rsaeb::limits::StepLimit;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::parse(ProgramSource::from_str("a=b\nb=c"))?;
//! let input = RuntimeInput::validate(b"a", RuntimeInputLimits::new(DEFAULT_MAX_STATE_LEN))?;
//! let execution = program.start_execution(
//!     &input,
//!     RunLimits::new(StepLimit::new(10), DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN),
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
//! [`RuntimeInputLimits`] bounds owned input classification before allocation.
//! [`RunLimits`] carries the step budget and byte budgets for runtime states and
//! `(return)` outputs. Trace snapshot materialization uses an explicit
//! [`limits::TraceSnapshotByteLimit`]. Step limits are checked only when another
//! matching rule would apply after the configured number of completed steps:
//!
//! ```
//! use rsaeb::error::{LimitError, RunError};
//! use rsaeb::limits::StepLimit;
//! use rsaeb::{
//!     DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, Program, ProgramSource, RunLimits,
//!     RuntimeInput, RuntimeInputLimits,
//! };
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let limits = RunLimits::new(StepLimit::new(0), DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN);
//! let input = RuntimeInput::validate(b"a", RuntimeInputLimits::new(limits.state_byte_limit()))?;
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
//! # Rule inspection and tracing
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
//! Borrowed trace events allocate no snapshots. Snapshot tracing is layered on
//! top when a caller needs owned event bytes:
//!
//! ```
//! use rsaeb::limits::StepLimit;
//! use rsaeb::trace::BorrowedTraceEvent;
//! use rsaeb::{
//!     DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, Program, ProgramSource, RunLimits,
//!     RuntimeInput, RuntimeInputLimits,
//! };
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::parse(ProgramSource::from_str("a=b\nb=(return)ok"))?;
//! let mut byte_counts = Vec::new();
//! let limits = RunLimits::new(StepLimit::new(10), DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN);
//! let input = RuntimeInput::validate(b"a", RuntimeInputLimits::new(limits.state_byte_limit()))?;
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
pub mod inspect;
pub mod limits;
mod parser;
mod program;
mod rule;
mod runtime;
pub mod source;
mod syntax;
pub mod trace;

pub use program::{
    DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_STEPS, Program, ReturnOutput,
    RunLimits, RunOutcome, RunResult, RuntimeStateSnapshot,
};
pub use runtime::{
    AppliedExecution, ExecutionStepError, ExecutionTransition, ReturnedExecution, RunningExecution,
    RuntimeInput, RuntimeInputLimits, StableExecution,
};
pub use source::ProgramSource;
