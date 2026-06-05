//! Byte-oriented interpreter for A=B ordered rewrite programs.
//!
//! This page is the canonical API guide. The README is the package entry point
//! and language overview, while this module focuses on the exact public Rust
//! surfaces and the typed boundaries a host program should use.
//!
//! `rsaeb` is a `no_std + alloc` library crate. It parses compact A=B source
//! directly into either [`program::ExecutableProgram`] or
//! [`program::EmptyProgram`], validates host bytes as [`input::RuntimeInput`],
//! admits that input into a one-run [`input::AdmittedRun`], and executes under
//! resource policies selected by types from [`policy`]. The interpreter core
//! does not read files, use process arguments, access environment variables,
//! write stdout/stderr, or perform lossy byte-to-text display conversion.
//!
//! Program shape is selected at the parse boundary:
//!
//! - [`program::EmptyProgram::parse_text`] and
//!   [`program::EmptyProgram::parse_bytes`] accept syntactically valid source only
//!   when it contains no executable rules. Empty programs expose
//!   [`program::EmptyProgram::stabilize`] only.
//! - [`program::ExecutableProgram::parse_text`] and
//!   [`program::ExecutableProgram::parse_bytes`] accept syntactically valid source
//!   only when it contains at least one executable rule. Execution, tracing,
//!   stepwise execution, and rule-attempt execution exist only on this type.
//!
//! `(once)` repeat intent and right-side action shape are parsed into the rule
//! variant itself. Program topology assigns rule positions, while each run owns
//! fresh or consumed runtime rule cells for rule-local `(once)` availability.
//! Consumed once rules are filtered before ordinary matching and reported as
//! typed rule-attempt misses, so once-state mismatch is not a public runtime
//! error class.
//!
//! # API map
//!
//! Use these public entry points according to the boundary being crossed:
//!
//! - [`program::ExecutableProgram::parse_text`] /
//!   [`program::ExecutableProgram::parse_bytes`] and
//!   [`program::EmptyProgram::parse_text`] /
//!   [`program::EmptyProgram::parse_bytes`] validate source syntax under a
//!   [`policy::ParsePolicy`] and reject the wrong program shape with
//!   phase-specific parse errors.
//! - [`input::RuntimeInputSource::from_bytes`] labels host input bytes, and
//!   [`input::RuntimeInput::validate`] validates and owns them in the runtime
//!   input byte domain until execution consumes the value.
//! - [`input::AdmittedRun`] admits validated input under a
//!   [`policy::ExecutionPolicy`].
//! - [`program::ExecutableProgram::execute`], [`program::ExecutableProgram::trace`],
//!   [`program::ExecutableProgram::steps`], and
//!   [`program::ExecutableProgram::rule_attempts`] start executable runs.
//! - [`program::EmptyProgram::stabilize`] materializes admitted input as a
//!   zero-step stable result for empty source.
//! - [`inspect`] exposes borrowed structured rule views, and [`error`] exposes
//!   structured parse, input, runtime, and trace errors.
//!
//! # Compile-time contract map
//!
//! The compile-fail guards live beside the public boundary they protect:
//!
//! - [`source`] rejects the deleted public source carriers and marker types.
//! - [`program`] rejects shape-erased programs, old parse entrypoints, stored
//!   policy parameters, and empty/executable topology leakage.
//! - [`input`] rejects bypassing the admitted-run witness.
//! - [`execution`] rejects deleted execution modes, owned stepwise APIs,
//!   shape-erased rule-attempt APIs, and impossible transition variants.
//! - [`inspect`] rejects flat or shape-neutral rule inspection.
//! - [`trace`] rejects deleted trace effects and shape-erased trace events.
//! - [`policy`] rejects policy-domain mixing and runtime policy value bags.
//! - [`error`] rejects deleted parser and runtime error shapes.
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
//! [`input::AdmittedRun`] is the admission witness for one run. It consumes a
//! validated [`input::RuntimeInput`] under an [`policy::ExecutionPolicy`],
//! checks the initial runtime-state budget, and prevents a later execution API
//! from receiving raw bytes or detached budget values.
//!
//! # Basic execution
//!
//! Parse through the executable boundary, validate [`input::RuntimeInput`],
//! then run:
//!
//! ```
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{DefaultExecutionPolicy, DefaultParsePolicy, DefaultRuntimeInputPolicy};
//! use rsaeb::program::{ExecutableProgram, RunOutcome};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let executable = ExecutableProgram::parse_text::<DefaultParsePolicy>("a=b")?;
//! let input = RuntimeInput::validate::<DefaultRuntimeInputPolicy>(RuntimeInputSource::from_bytes(b"a"))?;
//! let admitted = input.admit::<DefaultExecutionPolicy>()?;
//! let result = executable.execute(admitted)?;
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
//! Empty source has its own type and can only stabilize admitted input:
//!
//! ```
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{DefaultExecutionPolicy, DefaultParsePolicy, DefaultRuntimeInputPolicy};
//! use rsaeb::program::{EmptyProgram, RunOutcome};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let empty = EmptyProgram::parse_text::<DefaultParsePolicy>("# empty")?;
//! let input = RuntimeInput::validate::<DefaultRuntimeInputPolicy>(RuntimeInputSource::from_bytes(b"unchanged"))?;
//! let admitted = input.admit::<DefaultExecutionPolicy>()?;
//! let result = empty.stabilize(admitted)?;
//!
//! if !matches!(
//!     result.outcome(),
//!     RunOutcome::Stable(output) if output.as_slice() == b"unchanged"
//! ) {
//!     return Err("unexpected stable output".into());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Parse once when the same executable rules should be reused. Each runtime
//! invocation owns fresh per-rule repeat state, so `(once)` consumption never
//! mutates the parsed program:
//!
//! ```
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{DefaultParsePolicy, DefaultRuntimeInputPolicy, StaticExecutionPolicy};
//! use rsaeb::program::{ExecutableProgram, RunOutcome};
//!
//! type ShortRun = StaticExecutionPolicy<10_000, 16_777_216, 16_777_216>;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let executable = ExecutableProgram::parse_text::<DefaultParsePolicy>("(once)a=b\na=c")?;
//!
//! let first_input = RuntimeInput::validate::<DefaultRuntimeInputPolicy>(RuntimeInputSource::from_bytes(b"aa"))?;
//! let second_input = RuntimeInput::validate::<DefaultRuntimeInputPolicy>(RuntimeInputSource::from_bytes(b"aa"))?;
//!
//! let first = executable.execute(first_input.admit::<ShortRun>()?)?;
//! let second = executable.execute(second_input.admit::<ShortRun>()?)?;
//!
//! if !matches!(first.outcome(), RunOutcome::Stable(output) if output.as_slice() == b"bc") {
//!     return Err("unexpected first output".into());
//! }
//! if !matches!(second.outcome(), RunOutcome::Stable(output) if output.as_slice() == b"bc") {
//!     return Err("unexpected second output".into());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Compile-time policies
//!
//! Resource policy is selected by types. Const-generic static policies let a
//! host name tighter domains without constructing runtime limit bags:
//!
//! ```
//! use rsaeb::error::{RunError, RunFinishError, RunStepError};
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{DefaultParsePolicy, StaticExecutionPolicy, StaticRuntimeInputPolicy};
//! use rsaeb::program::ExecutableProgram;
//!
//! type TinyInput = StaticRuntimeInputPolicy<4>;
//! type NoSteps = StaticExecutionPolicy<0, 4, 4>;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let executable = ExecutableProgram::parse_text::<DefaultParsePolicy>("a=b")?;
//! let input = RuntimeInput::validate::<TinyInput>(RuntimeInputSource::from_bytes(b"a"))?;
//! let admitted = input.admit::<NoSteps>()?;
//! let result = executable.execute(admitted);
//!
//! if !matches!(
//!     result,
//!     Err(RunError::Finish(RunFinishError::Step(RunStepError::StepLimit(error))))
//!         if error.max_steps().get() == 0
//! ) {
//!     return Err("unexpected step-limit error".into());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Step Execution
//!
//! Use [`program::ExecutableProgram::steps`] when a host wants to wait after
//! each applied rule while keeping the parsed program reusable:
//!
//! ```
//! use rsaeb::execution::BorrowedStepTransition;
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{DefaultParsePolicy, DefaultRuntimeInputPolicy, StaticExecutionPolicy};
//! use rsaeb::program::ExecutableProgram;
//!
//! type TenSteps = StaticExecutionPolicy<10, 16_777_216, 16_777_216>;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let executable = ExecutableProgram::parse_text::<DefaultParsePolicy>("a=b\nb=c")?;
//! let input = RuntimeInput::validate::<DefaultRuntimeInputPolicy>(RuntimeInputSource::from_bytes(b"a"))?;
//! let admitted = input.admit::<TenSteps>()?;
//! let execution = executable.steps(admitted)?;
//!
//! let execution = match execution.step() {
//!     BorrowedStepTransition::AlwaysRewritten(applied) => {
//!         if applied.rule().position().get() != 1 {
//!             return Err("unexpected first applied rule".into());
//!         }
//!         if applied.state().materialize()?.as_slice() != b"b" {
//!             return Err("unexpected first applied state".into());
//!         }
//!         applied.into_session()
//!     }
//!     BorrowedStepTransition::OnceRewritten(_)
//!     | BorrowedStepTransition::Stable(_)
//!     | BorrowedStepTransition::AlwaysReturned(_)
//!     | BorrowedStepTransition::OnceReturned(_)
//!     | BorrowedStepTransition::Failed(_) => {
//!         return Err("expected first applied step".into());
//!     }
//! };
//!
//! match execution.step() {
//!     BorrowedStepTransition::AlwaysRewritten(applied) => {
//!         if applied.rule().position().get() != 2 {
//!             return Err("unexpected second applied rule".into());
//!         }
//!     }
//!     BorrowedStepTransition::OnceRewritten(_)
//!     | BorrowedStepTransition::Stable(_)
//!     | BorrowedStepTransition::AlwaysReturned(_)
//!     | BorrowedStepTransition::OnceReturned(_)
//!     | BorrowedStepTransition::Failed(_) => {
//!         return Err("expected second applied step".into());
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Use [`program::ExecutableProgram::rule_attempts`] when the host needs to
//! observe every executable rule line, including lines that do not apply to the
//! current runtime state:
//!
//! ```
//! use rsaeb::execution::{
//!     BorrowedContinuingRuleAttemptTransition, BorrowedFinalRuleAttemptTransition,
//!     BorrowedRuleAttemptCursor, RuleMiss,
//! };
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{
//!     DefaultParsePolicy, DefaultRuntimeInputPolicy, StaticExecutionPolicy,
//!     StaticRuleAttemptPolicy,
//! };
//! use rsaeb::program::ExecutableProgram;
//!
//! type TenSteps = StaticExecutionPolicy<10, 16_777_216, 16_777_216>;
//! type TenAttempts = StaticRuleAttemptPolicy<10>;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let executable = ExecutableProgram::parse_text::<DefaultParsePolicy>("z=x\na=b")?;
//! let input = RuntimeInput::validate::<DefaultRuntimeInputPolicy>(RuntimeInputSource::from_bytes(b"a"))?;
//! let admitted = input.admit::<TenSteps>()?;
//! let execution = executable.rule_attempts::<TenAttempts, _>(admitted)?;
//!
//! let BorrowedRuleAttemptCursor::Continuing(execution) = execution else {
//!     return Err("expected first rule to have a successor".into());
//! };
//! let execution = match execution.step() {
//!     BorrowedContinuingRuleAttemptTransition::Missed(missed) => {
//!         if !matches!(missed.miss(), RuleMiss::AlwaysRewriteStateMismatch(_)) {
//!             return Err("unexpected miss shape".into());
//!         }
//!         missed.into_cursor()
//!     }
//!     BorrowedContinuingRuleAttemptTransition::AlwaysRewritten(_)
//!     | BorrowedContinuingRuleAttemptTransition::OnceRewritten(_)
//!     | BorrowedContinuingRuleAttemptTransition::AlwaysReturned(_)
//!     | BorrowedContinuingRuleAttemptTransition::OnceReturned(_)
//!     | BorrowedContinuingRuleAttemptTransition::Failed(_) => return Err("expected first rule to miss".into()),
//! };
//!
//! let BorrowedRuleAttemptCursor::Final(execution) = execution else {
//!     return Err("expected final cursor after first miss".into());
//! };
//! match execution.step() {
//!     BorrowedFinalRuleAttemptTransition::AlwaysRewritten(applied) => {
//!         if applied.step().get() != 1 || applied.rule().position().get() != 2 {
//!             return Err("unexpected applied rule attempt".into());
//!         }
//!     }
//!     BorrowedFinalRuleAttemptTransition::Stable(_)
//!     | BorrowedFinalRuleAttemptTransition::OnceRewritten(_)
//!     | BorrowedFinalRuleAttemptTransition::AlwaysReturned(_)
//!     | BorrowedFinalRuleAttemptTransition::OnceReturned(_)
//!     | BorrowedFinalRuleAttemptTransition::Failed(_) => return Err("expected second rule to apply".into()),
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
//! use rsaeb::inspect::{RewriteActionView, RuleAnchor, RuleView};
//! use rsaeb::policy::DefaultParsePolicy;
//! use rsaeb::program::ExecutableProgram;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let executable = ExecutableProgram::parse_text::<DefaultParsePolicy>("( once ) ( start ) a = ( end ) b # comment")?;
//! let rule = executable.rules().next().ok_or("missing parsed rule")?;
//! let RuleView::OnceRewrite(rule) = rule else {
//!     return Err("expected once rewrite rule".into());
//! };
//! if rule.anchor() != RuleAnchor::Start {
//!     return Err("unexpected anchor".into());
//! }
//! if rule.lhs().materialize()?.as_slice() != b"a" {
//!     return Err("unexpected left side".into());
//! }
//! match rule.rewrite_action() {
//!     RewriteActionView::MoveEnd(payload) => {
//!         if payload.materialize()?.as_slice() != b"b" {
//!             return Err("unexpected moved payload".into());
//!         }
//!     }
//!     RewriteActionView::Replace(_) | RewriteActionView::MoveStart(_) => {
//!         return Err("expected move-end rewrite action".into());
//!     }
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
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{DefaultParsePolicy, DefaultRuntimeInputPolicy, StaticExecutionPolicy};
//! use rsaeb::program::ExecutableProgram;
//! use rsaeb::trace::{BorrowedTrace, BorrowedTraceEvent};
//!
//! type TenSteps = StaticExecutionPolicy<10, 16_777_216, 16_777_216>;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let executable = ExecutableProgram::parse_text::<DefaultParsePolicy>("a=b\nb=(return)ok")?;
//! let mut byte_counts = Vec::new();
//! let input = RuntimeInput::validate::<DefaultRuntimeInputPolicy>(RuntimeInputSource::from_bytes(b"a"))?;
//! let admitted = input.admit::<TenSteps>()?;
//!
//! executable.trace(
//!     admitted,
//!     BorrowedTrace::new(|event| {
//!         byte_counts.push(event.byte_count().get());
//!         match event {
//!             BorrowedTraceEvent::Initial { .. } => {}
//!             BorrowedTraceEvent::AlwaysRewritten { rule, .. } => {
//!                 let _rewrite = rule.rewrite_action();
//!             }
//!             BorrowedTraceEvent::OnceRewritten { rule, .. } => {
//!                 let _rewrite = rule.rewrite_action();
//!             }
//!             BorrowedTraceEvent::AlwaysReturned { rule, .. } => {
//!                 let _output = rule.output();
//!             }
//!             BorrowedTraceEvent::OnceReturned { rule, .. } => {
//!                 let _output = rule.output();
//!             }
//!         }
//!         Ok::<(), Infallible>(())
//!     }),
//! )?;
//!
//! if byte_counts != [1, 1, 2] {
//!     return Err("unexpected trace byte counts".into());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ```
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{
//!     DefaultParsePolicy, DefaultRuntimeInputPolicy, StaticExecutionPolicy,
//!     StaticTraceSnapshotPolicy,
//! };
//! use rsaeb::program::ExecutableProgram;
//! use rsaeb::trace::{SnapshotTrace, TraceSnapshotEvent};
//!
//! type TenSteps = StaticExecutionPolicy<10, 16_777_216, 16_777_216>;
//! type SnapshotBytes = StaticTraceSnapshotPolicy<16_777_216>;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let executable = ExecutableProgram::parse_text::<DefaultParsePolicy>("a=b\nb=(return)ok")?;
//! let input = RuntimeInput::validate::<DefaultRuntimeInputPolicy>(RuntimeInputSource::from_bytes(b"a"))?;
//! let admitted = input.admit::<TenSteps>()?;
//! let mut states = Vec::new();
//! let mut returns = Vec::new();
//!
//! executable.trace(
//!     admitted,
//!     SnapshotTrace::<SnapshotBytes, _>::new(|event| {
//!         match event {
//!             TraceSnapshotEvent::Initial { state } => states.push(state.into_raw_bytes()),
//!             TraceSnapshotEvent::AlwaysRewritten { state, .. } => {
//!                 states.push(state.into_raw_bytes());
//!             }
//!             TraceSnapshotEvent::OnceRewritten { state, .. } => {
//!                 states.push(state.into_raw_bytes());
//!             }
//!             TraceSnapshotEvent::AlwaysReturned { output, .. } => {
//!                 returns.push(output.into_raw_bytes());
//!             }
//!             TraceSnapshotEvent::OnceReturned { output, .. } => {
//!                 returns.push(output.into_raw_bytes());
//!             }
//!         }
//!         Ok::<(), core::convert::Infallible>(())
//!     }),
//! )?;
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
//!
//! ```
//! use rsaeb::error::RuntimeInputError;
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::StaticRuntimeInputPolicy;
//!
//! type HostInput = StaticRuntimeInputPolicy<4>;
//!
//! fn validate_host_input(bytes: &[u8]) -> Result<RuntimeInput, RuntimeInputError> {
//!     RuntimeInput::validate::<HostInput>(RuntimeInputSource::from_bytes(bytes))
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
pub mod policy;
pub mod program;
/// Parsed rule domain model.
mod rule;
/// Runtime execution engine.
mod runtime;
pub mod source;
/// Reserved syntax token model.
mod syntax;
pub mod trace;
