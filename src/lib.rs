//! Byte-oriented interpreter for A=B ordered rewrite programs.
//!
//! This page is the docs.rs API guide. The README carries the longer language
//! reference; this page focuses on the public Rust surface and the typed
//! boundaries a host program should use.
//!
//! `rsaeb` is a `no_std + alloc` library crate. It parses compact A=B source
//! into an immutable [`program::Program`], validates host bytes as
//! [`input::RuntimeInput`], admits that input into a one-run [`input::RunSeed`],
//! and executes only after [`limits::ExecutionLimits`] are attached. The
//! interpreter core does not read files, use process arguments, access
//! environment variables, write stdout/stderr, or perform lossy byte-to-text
//! display conversion.
//!
//! # API map
//!
//! Use these public entry points according to the boundary being crossed:
//!
//! - [`source::ProgramSource::from_bytes`] and [`source::ProgramSource::from_text`] explicitly
//!   label host bytes or strings as A=B source before parsing.
//! - [`program::Program::parse`] validates source syntax under [`limits::ParseLimits`] and
//!   returns a reusable [`program::Program`].
//! - [`input::RuntimeInputSource::from_bytes`] labels host input bytes, and
//!   [`input::RuntimeInput::validate`] validates and owns them in the runtime input
//!   byte domain until execution consumes the value.
//! - [`limits::RuntimeInputLimits`] bounds raw input validation,
//!   [`input::RunSeed`] admits validated input under [`limits::ExecutionLimits`],
//!   and [`limits::TraceSnapshotByteLimit`] bounds trace snapshot materialization.
//! - [`program::Program::run`] runs to completion while borrowing the parsed
//!   program, [`program::Program::start_run`] returns a borrowed typestate
//!   execution, and [`program::Program::into_run`] returns the explicit owned
//!   typestate execution.
//! - [`program::Program::run_with_borrowed_trace`] observes borrowed trace events without
//!   per-event allocation; [`program::Program::run_with_trace_snapshots`] materializes
//!   bounded owned trace events.
//! - [`inspect`] exposes borrowed structured rule views, and [`error`] exposes
//!   structured parse, input, runtime, and trace errors.
//!
//! # Typed boundaries
//!
//! Program source and runtime input are different byte domains. Program payload
//! bytes are printable executable syntax bytes accepted by the parser. Runtime
//! input accepts any ASCII byte, including whitespace, control bytes, and bytes
//! that are reserved syntax in program source. Construct both explicitly before
//! execution so parsing, input validation, and runtime failures remain
//! distinguishable in the type system.
//!
//! # Basic execution
//!
//! Parse [`source::ProgramSource`], validate [`input::RuntimeInput`], then
//! admit an [`input::RunSeed`] before running:
//!
//! ```
//! use rsaeb::limits::{
//!     DEFAULT_MAX_INPUT_LEN, DEFAULT_PARSE_LIMITS, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_STEPS,
//! };
//! use rsaeb::input::{RunSeed, RuntimeInput, RuntimeInputSource};
//! use rsaeb::limits::{ExecutionLimits, RuntimeInputLimits};
//! use rsaeb::program::{Program, RunOutcome};
//! use rsaeb::source::ProgramSource;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::parse(ProgramSource::from_text("a=b"), DEFAULT_PARSE_LIMITS)?;
//! let input_limits = RuntimeInputLimits::new(DEFAULT_MAX_INPUT_LEN);
//! let execution_limits = ExecutionLimits::new(
//!     DEFAULT_MAX_STEPS,
//!     DEFAULT_MAX_STATE_LEN,
//!     DEFAULT_MAX_RETURN_LEN,
//! );
//! let input = RuntimeInput::validate(RuntimeInputSource::from_bytes(b"a"), input_limits)?;
//! let seed = RunSeed::admit(input, execution_limits)?;
//! let result = program.run(seed)?;
//!
//! if !matches!(
//!     result.outcome(),
//!     RunOutcome::Stable(output) if output.as_slice() == b"b"
//! ) {
//!     return Err("unexpected stable output".into());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Parse [`program::Program`] once when the same rules should be reused. Per-run
//! `(once)` state is owned by each runtime invocation, not by the parsed
//! program:
//!
//! ```
//! use rsaeb::limits::{
//!     DEFAULT_MAX_INPUT_LEN, DEFAULT_PARSE_LIMITS, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN,
//!     ExecutionLimits, RuntimeInputLimits, StepLimit,
//! };
//! use rsaeb::input::{RunSeed, RuntimeInput, RuntimeInputSource};
//! use rsaeb::program::{Program, RunOutcome};
//! use rsaeb::source::ProgramSource;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::parse(ProgramSource::from_text("(once)a=b\na=c"), DEFAULT_PARSE_LIMITS)?;
//! let input_limits = RuntimeInputLimits::new(DEFAULT_MAX_INPUT_LEN);
//! let execution_limits = ExecutionLimits::new(
//!     StepLimit::new(10_000),
//!     DEFAULT_MAX_STATE_LEN,
//!     DEFAULT_MAX_RETURN_LEN,
//! );
//!
//! let first_input = RuntimeInput::validate(RuntimeInputSource::from_bytes(b"aa"), input_limits)?;
//! let second_input = RuntimeInput::validate(RuntimeInputSource::from_bytes(b"aa"), input_limits)?;
//!
//! let first = program.run(RunSeed::admit(first_input, execution_limits)?)?;
//! let second = program.run(RunSeed::admit(second_input, execution_limits)?)?;
//!
//! if !matches!(
//!     first.outcome(),
//!     RunOutcome::Stable(output) if output.as_slice() == b"bc"
//! ) {
//!     return Err("unexpected first output".into());
//! }
//! if !matches!(
//!     second.outcome(),
//!     RunOutcome::Stable(output) if output.as_slice() == b"bc"
//! ) {
//!     return Err("unexpected second output".into());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Stepwise execution
//!
//! Use [`program::Program::start_run`] when a host wants to wait after each
//! applied rule while keeping the parsed program reusable:
//!
//! ```
//! use rsaeb::execution::StepTransition;
//! use rsaeb::limits::{
//!     DEFAULT_MAX_INPUT_LEN, DEFAULT_PARSE_LIMITS, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, StepLimit,
//! };
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::input::RunSeed;
//! use rsaeb::limits::{ExecutionLimits, RuntimeInputLimits};
//! use rsaeb::program::Program;
//! use rsaeb::source::ProgramSource;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::parse(ProgramSource::from_text("a=b\nb=c"), DEFAULT_PARSE_LIMITS)?;
//! let input_limits = RuntimeInputLimits::new(DEFAULT_MAX_INPUT_LEN);
//! let execution_limits = ExecutionLimits::new(
//!     StepLimit::new(10),
//!     DEFAULT_MAX_STATE_LEN,
//!     DEFAULT_MAX_RETURN_LEN,
//! );
//! let input = RuntimeInput::validate(RuntimeInputSource::from_bytes(b"a"), input_limits)?;
//! let seed = RunSeed::admit(input, execution_limits)?;
//! let execution = program.start_run(seed)?;
//!
//! let execution = match execution.step() {
//!     StepTransition::Applied(applied) => {
//!         if applied.state().materialize()?.as_slice() != b"b" {
//!             return Err("unexpected first applied state".into());
//!         }
//!         applied.into_session()
//!     }
//!     StepTransition::Stable(_) | StepTransition::Returned(_) | StepTransition::Failed(_) => {
//!         return Err("expected first applied step".into());
//!     }
//! };
//!
//! let execution = match execution.step() {
//!     StepTransition::Applied(applied) => {
//!         if applied.state().materialize()?.as_slice() != b"c" {
//!             return Err("unexpected second applied state".into());
//!         }
//!         applied.into_session()
//!     }
//!     StepTransition::Stable(_) | StepTransition::Returned(_) | StepTransition::Failed(_) => {
//!         return Err("expected second applied step".into());
//!     }
//! };
//!
//! match execution.step() {
//!     StepTransition::Stable(stable) => {
//!         if stable.steps().get() != 2 {
//!             return Err("unexpected stable step count".into());
//!         }
//!         if stable.state().materialize()?.as_slice() != b"c" {
//!             return Err("unexpected stable state".into());
//!         }
//!     }
//!     StepTransition::Applied(_) | StepTransition::Returned(_) | StepTransition::Failed(_) => {
//!         return Err("expected stable completion".into());
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! A [`execution::StepTransition::Failed`] value is terminal. It exposes the
//! uncommitted state for diagnostics, then lets callers discard the failed run
//! into its [`error::RunError`]; it does not expose a retryable session.
//!
//! # Limits
//!
//! [`limits::RuntimeInputLimits`] carries input-byte validation policy.
//! [`limits::ExecutionLimits`] carries initial runtime-state admission, the step
//! budget, and byte budgets for rewrite states and `(return)` outputs. Trace
//! snapshot materialization uses an explicit
//! [`limits::TraceSnapshotByteLimit`]. Step limits are checked only when another
//! matching rule would apply after the configured number of completed steps:
//!
//! ```
//! use rsaeb::error::{LimitError, RunError};
//! use rsaeb::limits::{
//!     DEFAULT_MAX_INPUT_LEN, DEFAULT_PARSE_LIMITS, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, StepLimit,
//! };
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::input::RunSeed;
//! use rsaeb::limits::{ExecutionLimits, RuntimeInputLimits};
//! use rsaeb::program::Program;
//! use rsaeb::source::ProgramSource;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let input_limits = RuntimeInputLimits::new(DEFAULT_MAX_INPUT_LEN);
//! let execution_limits = ExecutionLimits::new(
//!     StepLimit::new(0),
//!     DEFAULT_MAX_STATE_LEN,
//!     DEFAULT_MAX_RETURN_LEN,
//! );
//! let input = RuntimeInput::validate(RuntimeInputSource::from_bytes(b"a"), input_limits)?;
//! let seed = RunSeed::admit(input, execution_limits)?;
//! let result = Program::parse(ProgramSource::from_text("a=b"), DEFAULT_PARSE_LIMITS)?.run(seed);
//!
//! if !matches!(
//!     result,
//!     Err(RunError::Limit(LimitError::Step { completed_steps, .. }))
//!         if completed_steps.get() == 0
//! ) {
//!     return Err("unexpected step-limit error".into());
//! }
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
//! use rsaeb::limits::DEFAULT_PARSE_LIMITS;
//! use rsaeb::inspect::{RuleActionView, RuleAnchor, RuleRepeat};
//! use rsaeb::program::Program;
//! use rsaeb::source::ProgramSource;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::parse(ProgramSource::from_text("( once ) ( start ) a = ( end ) b # comment"), DEFAULT_PARSE_LIMITS)?;
//! let rule = program.rules().next().ok_or("missing parsed rule")?;
//!
//! if rule.repeat() != RuleRepeat::Once {
//!     return Err("unexpected repeat".into());
//! }
//! if rule.anchor() != RuleAnchor::Start {
//!     return Err("unexpected anchor".into());
//! }
//! if rule.lhs().materialize()?.as_slice() != b"a" {
//!     return Err("unexpected left side".into());
//! }
//! match rule.action() {
//!     RuleActionView::MoveEnd(payload) => {
//!         if payload.materialize()?.as_slice() != b"b" {
//!             return Err("unexpected moved payload".into());
//!         }
//!     }
//!     RuleActionView::Replace(_) | RuleActionView::MoveStart(_) | RuleActionView::Return(_) => {
//!         return Err("expected move-end action".into());
//!     }
//! }
//! if rule.canonical_source()?.as_slice() != b"(once)(start)a=(end)b" {
//!     return Err("unexpected canonical source".into());
//! }
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
//! use core::convert::Infallible;
//! use rsaeb::limits::{
//!     DEFAULT_MAX_INPUT_LEN, DEFAULT_PARSE_LIMITS, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, StepLimit,
//! };
//! use rsaeb::trace::BorrowedTraceEvent;
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::input::RunSeed;
//! use rsaeb::limits::{ExecutionLimits, RuntimeInputLimits};
//! use rsaeb::program::Program;
//! use rsaeb::source::ProgramSource;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::parse(ProgramSource::from_text("a=b\nb=(return)ok"), DEFAULT_PARSE_LIMITS)?;
//! let mut byte_counts = Vec::new();
//! let input_limits = RuntimeInputLimits::new(DEFAULT_MAX_INPUT_LEN);
//! let execution_limits = ExecutionLimits::new(
//!     StepLimit::new(10),
//!     DEFAULT_MAX_STATE_LEN,
//!     DEFAULT_MAX_RETURN_LEN,
//! );
//! let input = RuntimeInput::validate(RuntimeInputSource::from_bytes(b"a"), input_limits)?;
//! let seed = RunSeed::admit(input, execution_limits)?;
//!
//! program.run_with_borrowed_trace(
//!     seed,
//!     |event| {
//!         byte_counts.push(event.byte_count().get());
//!         if let BorrowedTraceEvent::Step { rule, .. } = event {
//!             let _line = rule.line_number();
//!         }
//!         Ok::<(), Infallible>(())
//!     },
//! )?;
//!
//! if byte_counts != [1, 1, 2] {
//!     return Err("unexpected trace byte counts".into());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Snapshot tracing materializes owned event bytes under an explicit snapshot
//! byte budget, which lets the caller retain events after each callback returns:
//!
//! ```
//! use rsaeb::limits::{
//!     DEFAULT_MAX_INPUT_LEN, DEFAULT_PARSE_LIMITS, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN,
//!     DEFAULT_MAX_TRACE_SNAPSHOT_LEN, StepLimit,
//! };
//! use rsaeb::trace::{TraceSnapshotEffect, TraceSnapshotEvent};
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::input::RunSeed;
//! use rsaeb::limits::{ExecutionLimits, RuntimeInputLimits};
//! use rsaeb::program::Program;
//! use rsaeb::source::ProgramSource;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::parse(ProgramSource::from_text("a=b\nb=(return)ok"), DEFAULT_PARSE_LIMITS)?;
//! let input_limits = RuntimeInputLimits::new(DEFAULT_MAX_INPUT_LEN);
//! let execution_limits = ExecutionLimits::new(
//!     StepLimit::new(10),
//!     DEFAULT_MAX_STATE_LEN,
//!     DEFAULT_MAX_RETURN_LEN,
//! );
//! let input = RuntimeInput::validate(RuntimeInputSource::from_bytes(b"a"), input_limits)?;
//! let seed = RunSeed::admit(input, execution_limits)?;
//! let mut states = Vec::new();
//! let mut returns = Vec::new();
//!
//! program.run_with_trace_snapshots(seed, DEFAULT_MAX_TRACE_SNAPSHOT_LEN, |event| {
//!     match event {
//!         TraceSnapshotEvent::Initial { state } => states.push(state.into_raw_bytes()),
//!         TraceSnapshotEvent::Step {
//!             effect: TraceSnapshotEffect::Continue { state },
//!             ..
//!         } => states.push(state.into_raw_bytes()),
//!         TraceSnapshotEvent::Step {
//!             effect: TraceSnapshotEffect::Return { output },
//!             ..
//!         } => returns.push(output.into_raw_bytes()),
//!     }
//!     Ok::<(), core::convert::Infallible>(())
//! })?;
//!
//! if states != [b"a".to_vec(), b"b".to_vec()] {
//!     return Err("unexpected trace states".into());
//! }
//! if returns != [b"ok".to_vec()] {
//!     return Err("unexpected trace returns".into());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Error model
//!
//! Source parsing, runtime input validation, runtime execution, trace snapshot
//! materialization, and user trace-sink failures are reported with structured
//! error types such as [`error::ParseError`], [`error::RuntimeInputError`],
//! [`error::RunError`], [`error::AllocationError`],
//! [`error::TraceSnapshotError`], [`error::TraceSnapshotRunError`], and
//! [`error::TracedRunError`].
//! Allocation reservation failures include a typed
//! [`error::RequestedCapacity`] instead of only a formatted string.
//! Representation and internal-invariant failures are distinct structured
//! domains, so parser/runtime witness contradictions do not become hidden
//! panics or allocation errors.
//!
//! ```
//! use rsaeb::error::RuntimeInputError;
//! use rsaeb::limits::{RuntimeInputByteLimit, RuntimeInputLimits};
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//!
//! fn validate_host_input(bytes: &[u8]) -> Result<RuntimeInput, RuntimeInputError> {
//!     let limits = RuntimeInputLimits::new(RuntimeInputByteLimit::new(4));
//!     RuntimeInput::validate(RuntimeInputSource::from_bytes(bytes), limits)
//! }
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let Err(error) = validate_host_input(&[0xff]) else {
//!     return Err("expected non-ASCII input to fail".into());
//! };
//!
//! if !matches!(
//!     error,
//!     RuntimeInputError::NonAscii { column, byte }
//!         if column.get() == 1 && byte.get() == 0xff
//! ) {
//!     return Err("unexpected input error".into());
//! }
//! # Ok(())
//! # }
//! ```

#![no_std]

extern crate alloc;

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod test_support;

/// Allocation boundary error model and fallible Vec helpers.
mod allocation;
/// Byte-domain model shared by parser and runtime.
mod bytes;
pub mod error;
pub mod execution;
pub mod input;
pub mod inspect;
pub mod limits;
/// Domain-tagged owned byte buffers.
mod materialized;
/// Program source parser.
mod parser;
pub mod program;
/// Parsed rule domain model.
mod rule;
/// Runtime execution engine.
mod runtime;
pub mod source;
/// Reserved syntax token model.
mod syntax;
pub mod trace;
