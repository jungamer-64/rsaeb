//! Byte-oriented interpreter for A=B ordered rewrite programs.
//!
//! This page is the canonical API guide. The README is the package entry point
//! and language overview, while the project wiki is a short use-case navigation
//! layer. This page focuses on exact public Rust surfaces and the typed
//! boundaries a host program should use.
//!
//! `rsaeb` is a `no_std + alloc` library crate. It parses compact A=B source
//! into an immutable [`program::Program`], validates host bytes as
//! [`input::RuntimeInput`], admits that input into a one-run [`input::AdmittedRun`],
//! and executes under resource policies selected by types from [`policy`]. The
//! interpreter core does not read files, use process arguments, access
//! environment variables, write stdout/stderr, or perform lossy byte-to-text
//! display conversion.
//!
//! The public API is intentionally arranged as boundary types rather than root
//! re-exports. A host should move data through the domains in this order:
//! source bytes become a parsed program, raw input bytes become validated
//! runtime input, validated input becomes an admitted run, and only then
//! can execution or tracing start. That ordering keeps parse errors, input
//! validation errors, run-admission errors, runtime failures, and trace-sink
//! failures separate in both type signatures and diagnostics.
//!
//! # API map
//!
//! Use these public entry points according to the boundary being crossed:
//!
//! - [`source::ProgramSource::from_bytes`] and [`source::ProgramSource::from_text`] explicitly
//!   label host bytes or strings as A=B source before parsing.
//! - [`program::Program::parse`] validates source syntax under the program's
//!   [`policy::ParsePolicy`] and returns a reusable [`program::Program`].
//! - [`input::RuntimeInputSource::from_bytes`] labels host input bytes, and
//!   [`input::RuntimeInput::validate`] validates and owns them in the runtime input
//!   byte domain until execution consumes the value.
//! - [`policy::RuntimeInputPolicy`] bounds raw input validation,
//!   [`input::AdmittedRun`] admits validated input under a
//!   [`policy::ExecutionPolicy`], and [`policy::TraceSnapshotPolicy`] bounds
//!   trace snapshot materialization.
//! - [`program::Program::execute`] selects borrowed run-to-completion,
//!   borrowed stepwise, or borrowed rule-attempt execution by type.
//! - [`program::Program::into_execute`] selects owned stepwise or owned
//!   rule-attempt execution by type.
//! - [`program::Program::trace`] emits borrowed or materialized snapshot trace
//!   events from a typed trace request.
//! - [`inspect`] exposes borrowed structured rule views, and [`error`] exposes
//!   structured parse, input, runtime, and trace errors.
//!
//! # Compile-time API guards
//!
//! Runtime execution mode selectors and old method-shaped entrypoints are not
//! part of the public API:
//!
//! ```compile_fail
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{DefaultExecutionPolicy, DefaultParsePolicy, DefaultRuntimeInputPolicy};
//! use rsaeb::program::Program;
//! use rsaeb::source::ProgramSource;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let program = Program::<DefaultParsePolicy>::parse(ProgramSource::from_text("a=b"))?;
//!     let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
//!     let admitted = input.admit::<DefaultExecutionPolicy>()?;
//!     let _ = program.run(admitted)?;
//!     Ok(())
//! }
//! ```
//!
//! Borrowed and owned execution modes are different compile-time domains:
//!
//! ```compile_fail
//! use rsaeb::execution::BorrowedSteps;
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{DefaultExecutionPolicy, DefaultParsePolicy, DefaultRuntimeInputPolicy};
//! use rsaeb::program::Program;
//! use rsaeb::source::ProgramSource;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let program = Program::<DefaultParsePolicy>::parse(ProgramSource::from_text("a=b"))?;
//!     let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
//!     let admitted = input.admit::<DefaultExecutionPolicy>()?;
//!     let _ = program.into_execute::<BorrowedSteps, _>(admitted)?;
//!     Ok(())
//! }
//! ```
//!
//! Execution entrypoints require admitted input. Validated input cannot be run
//! directly:
//!
//! ```compile_fail
//! use rsaeb::execution::CompleteRun;
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{DefaultParsePolicy, DefaultRuntimeInputPolicy};
//! use rsaeb::program::Program;
//! use rsaeb::source::ProgramSource;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let program = Program::<DefaultParsePolicy>::parse(ProgramSource::from_text("a=b"))?;
//!     let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
//!     let _ = program.execute::<CompleteRun, _>(input)?;
//!     Ok(())
//! }
//! ```
//!
//! Terminal transitions do not carry a continuation session, so they cannot be
//! stepped again:
//!
//! ```compile_fail
//! use rsaeb::execution::{BorrowedStepTransition, BorrowedSteps};
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{DefaultExecutionPolicy, DefaultParsePolicy, DefaultRuntimeInputPolicy};
//! use rsaeb::program::Program;
//! use rsaeb::source::ProgramSource;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let program = Program::<DefaultParsePolicy>::parse(ProgramSource::from_text(""))?;
//!     let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
//!     let admitted = input.admit::<DefaultExecutionPolicy>()?;
//!     let terminal = match program.execute::<BorrowedSteps, _>(admitted)?.step() {
//!         BorrowedStepTransition::Stable(stable) => stable,
//!         _ => return Ok(()),
//!     };
//!     let _ = terminal.step();
//!     Ok(())
//! }
//! ```
//!
//! Rule-attempt execution starts with a typed start state. The start itself
//! cannot be stepped until the caller proves it is active:
//!
//! ```compile_fail
//! use rsaeb::execution::BorrowedRuleAttempts;
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{
//!     DefaultExecutionPolicy, DefaultParsePolicy, DefaultRuntimeInputPolicy,
//!     StaticRuleAttemptPolicy,
//! };
//! use rsaeb::program::Program;
//! use rsaeb::source::ProgramSource;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let program = Program::<DefaultParsePolicy>::parse(ProgramSource::from_text("a=b"))?;
//!     let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
//!     let admitted = input.admit::<DefaultExecutionPolicy>()?;
//!     let start = program.execute::<BorrowedRuleAttempts<StaticRuleAttemptPolicy<10>>, _>(admitted)?;
//!     let _ = start.step();
//!     Ok(())
//! }
//! ```
//!
//! Rule-attempt stable terminals expose a final miss directly. The old
//! stable-reason enum is not part of the public API:
//!
//! ```compile_fail
//! fn main() {
//!     let _ = rsaeb::execution::RuleAttemptStableReason::<()>::NoExecutableRules;
//! }
//! ```
//!
//! ```compile_fail
//! use rsaeb::execution::{
//!     BorrowedRuleAttemptStart, BorrowedRuleAttemptTransition, BorrowedRuleAttempts,
//! };
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{
//!     DefaultExecutionPolicy, DefaultParsePolicy, DefaultRuntimeInputPolicy,
//!     StaticRuleAttemptPolicy,
//! };
//! use rsaeb::program::Program;
//! use rsaeb::source::ProgramSource;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let program = Program::<DefaultParsePolicy>::parse(ProgramSource::from_text("a=b"))?;
//!     let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"z"))?;
//!     let admitted = input.admit::<DefaultExecutionPolicy>()?;
//!     let start = program.execute::<BorrowedRuleAttempts<StaticRuleAttemptPolicy<10>>, _>(admitted)?;
//!     let BorrowedRuleAttemptStart::Active(session) = start else {
//!         return Ok(());
//!     };
//!     let stable = match session.step() {
//!         BorrowedRuleAttemptTransition::Stable(stable) => stable,
//!         _ => return Ok(()),
//!     };
//!     let _ = stable.stable_reason();
//!     Ok(())
//! }
//! ```
//!
//! Policy domains are not interchangeable:
//!
//! ```compile_fail
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::DefaultExecutionPolicy;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let _ = RuntimeInput::<DefaultExecutionPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
//!     Ok(())
//! }
//! ```
//!
//! Caller-selected generic action and event enums are intentionally absent;
//! callers receive concrete domain views:
//!
//! ```compile_fail
//! fn main() {
//!     let _action: rsaeb::inspect::RuleAction<Vec<u8>>;
//!     let _event: rsaeb::trace::TraceEvent<'static, Vec<u8>, Vec<u8>>;
//! }
//! ```
//!
//! Runtime rule-state provenance errors are intentionally absent. Parsed rules
//! and their per-run repeat state are advanced together inside the runtime, so
//! callers cannot observe or match a fallback slot-mismatch error:
//!
//! ```compile_fail
//! fn main() {
//!     let _ = core::mem::size_of::<rsaeb::error::RuleRuntimeStateError>();
//! }
//! ```
//!
//! ```compile_fail
//! use rsaeb::error::RunStepError;
//!
//! fn main() {
//!     let _ = |error: RunStepError| match error {
//!         RunStepError::RuleRuntimeState(_) => true,
//!         _ => false,
//!     };
//! }
//! ```
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
//! Rule-attempt entrypoints carry the [`policy::RuleAttemptPolicy`] type when a
//! host needs rule-line attempt stepping.
//!
//! # Basic execution
//!
//! Parse [`source::ProgramSource`], validate [`input::RuntimeInput`], then
//! admit an [`input::AdmittedRun`] before running:
//!
//! ```
//! use rsaeb::execution::CompleteRun;
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{DefaultExecutionPolicy, DefaultParsePolicy, DefaultRuntimeInputPolicy};
//! use rsaeb::program::{Program, RunOutcome};
//! use rsaeb::source::ProgramSource;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::<DefaultParsePolicy>::parse(ProgramSource::from_text("a=b"))?;
//! let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
//! let admitted = input.admit::<DefaultExecutionPolicy>()?;
//! let result = program.execute::<CompleteRun, _>(admitted)?;
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
//! Parse [`program::Program`] once when the same rules should be reused. The
//! parser records which rules are `(once)`, and each runtime invocation owns
//! per-rule repeat state rather than mutating the parsed program:
//!
//! ```
//! use rsaeb::execution::CompleteRun;
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{DefaultParsePolicy, DefaultRuntimeInputPolicy, StaticExecutionPolicy};
//! use rsaeb::program::{Program, RunOutcome};
//! use rsaeb::source::ProgramSource;
//!
//! type ShortRun = StaticExecutionPolicy<10_000, 16_777_216, 16_777_216>;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::<DefaultParsePolicy>::parse(ProgramSource::from_text("(once)a=b\na=c"))?;
//!
//! let first_input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"aa"))?;
//! let second_input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"aa"))?;
//!
//! let first = program.execute::<CompleteRun, _>(first_input.admit::<ShortRun>()?)?;
//! let second = program.execute::<CompleteRun, _>(second_input.admit::<ShortRun>()?)?;
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
//! # Compile-time policies
//!
//! Resource policy is selected by types. The default policy preserves the crate
//! defaults; const-generic static policies let a host name tighter domains
//! without constructing runtime limit bags:
//!
//! ```
//! use rsaeb::error::{RunError, RunFinishError, RunStepError};
//! use rsaeb::execution::CompleteRun;
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{DefaultParsePolicy, StaticExecutionPolicy, StaticRuntimeInputPolicy};
//! use rsaeb::program::Program;
//! use rsaeb::source::ProgramSource;
//!
//! type TinyInput = StaticRuntimeInputPolicy<4>;
//! type NoSteps = StaticExecutionPolicy<0, 4, 4>;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::<DefaultParsePolicy>::parse(ProgramSource::from_text("a=b"))?;
//! let input = RuntimeInput::<TinyInput>::validate(RuntimeInputSource::from_bytes(b"a"))?;
//! let admitted = input.admit::<NoSteps>()?;
//! let result = program.execute::<CompleteRun, _>(admitted);
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
//! Use [`program::Program::execute`] with [`execution::BorrowedSteps`] when a
//! host wants to wait after each applied rule while keeping the parsed program reusable:
//!
//! ```
//! use rsaeb::execution::{BorrowedSteps, BorrowedStepTransition};
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{DefaultParsePolicy, DefaultRuntimeInputPolicy, StaticExecutionPolicy};
//! use rsaeb::program::Program;
//! use rsaeb::source::ProgramSource;
//!
//! type TenSteps = StaticExecutionPolicy<10, 16_777_216, 16_777_216>;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::<DefaultParsePolicy>::parse(ProgramSource::from_text("a=b\nb=c"))?;
//! let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
//! let admitted = input.admit::<TenSteps>()?;
//! let execution = program.execute::<BorrowedSteps, _>(admitted)?;
//!
//! let execution = match execution.step() {
//!     BorrowedStepTransition::Applied(applied) => {
//!         if applied.rule().position().number().get() != 1 {
//!             return Err("unexpected first applied rule".into());
//!         }
//!         if applied.state().materialize()?.as_slice() != b"b" {
//!             return Err("unexpected first applied state".into());
//!         }
//!         applied.into_session()
//!     }
//!     BorrowedStepTransition::Stable(_) | BorrowedStepTransition::Returned(_) | BorrowedStepTransition::Failed(_) => {
//!         return Err("expected first applied step".into());
//!     }
//! };
//!
//! let execution = match execution.step() {
//!     BorrowedStepTransition::Applied(applied) => {
//!         if applied.rule().position().number().get() != 2 {
//!             return Err("unexpected second applied rule".into());
//!         }
//!         if applied.state().materialize()?.as_slice() != b"c" {
//!             return Err("unexpected second applied state".into());
//!         }
//!         applied.into_session()
//!     }
//!     BorrowedStepTransition::Stable(_) | BorrowedStepTransition::Returned(_) | BorrowedStepTransition::Failed(_) => {
//!         return Err("expected second applied step".into());
//!     }
//! };
//!
//! match execution.step() {
//!     BorrowedStepTransition::Stable(stable) => {
//!         if stable.steps().get() != 2 {
//!             return Err("unexpected stable step count".into());
//!         }
//!         if stable.state().materialize()?.as_slice() != b"c" {
//!             return Err("unexpected stable state".into());
//!         }
//!     }
//!     BorrowedStepTransition::Applied(_) | BorrowedStepTransition::Returned(_) | BorrowedStepTransition::Failed(_) => {
//!         return Err("expected stable completion".into());
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! A [`execution::BorrowedStepTransition::Failed`] value is terminal. It exposes the
//! uncommitted state for diagnostics, then lets callers discard the failed run
//! into its [`error::RunError`]; it does not expose a retryable session.
//! [`execution::OwnedStepTransition::Failed`] carries the same error and
//! uncommitted-state diagnostics for owned sessions, and it can split into the
//! runtime error plus the parsed program when ownership matters. Failed
//! transitions are terminal; recovering the program never recovers a retryable
//! session. Borrowed applied and returned transitions carry
//! [`inspect::RuleView`] witnesses; owned transitions retain
//! [`execution::OwnedRuleWitness`] values so rule metadata remains available
//! after ownership moves. Owned non-terminal applied and missed transitions also
//! expose `into_parts` methods so callers can keep the owned witness and the
//! continuation session together.
//!
//! Use [`program::Program::execute`] with
//! [`execution::BorrowedRuleAttempts`] when the host needs to observe every
//! executable rule line, including lines that do not apply to the current runtime state:
//!
//! ```
//! use rsaeb::execution::{
//!     BorrowedRuleAttemptStart, BorrowedRuleAttempts, BorrowedRuleAttemptTransition,
//!     RuleMissReason,
//! };
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{
//!     DefaultParsePolicy, DefaultRuntimeInputPolicy, StaticExecutionPolicy,
//!     StaticRuleAttemptPolicy,
//! };
//! use rsaeb::program::Program;
//! use rsaeb::source::ProgramSource;
//!
//! type TenSteps = StaticExecutionPolicy<10, 16_777_216, 16_777_216>;
//! type TenAttempts = StaticRuleAttemptPolicy<10>;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::<DefaultParsePolicy>::parse(ProgramSource::from_text("z=x\na=b"))?;
//! let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
//! let admitted = input.admit::<TenSteps>()?;
//! let start = program.execute::<BorrowedRuleAttempts<TenAttempts>, _>(admitted)?;
//! let BorrowedRuleAttemptStart::Active(execution) = start else {
//!     return Err("expected executable rules".into());
//! };
//!
//! let execution = match execution.step() {
//!     BorrowedRuleAttemptTransition::Missed(missed) => {
//!         if missed.miss().reason() != RuleMissReason::StateMismatch {
//!             return Err("unexpected miss reason".into());
//!         }
//!         if missed.miss().rule().position().number().get() != 1 {
//!             return Err("unexpected missed rule".into());
//!         }
//!         missed.into_session()
//!     }
//!     BorrowedRuleAttemptTransition::Applied(_)
//!     | BorrowedRuleAttemptTransition::Stable(_)
//!     | BorrowedRuleAttemptTransition::Returned(_)
//!     | BorrowedRuleAttemptTransition::Failed(_) => return Err("expected first rule to miss".into()),
//! };
//!
//! match execution.step() {
//!     BorrowedRuleAttemptTransition::Applied(applied) => {
//!         if applied.step().get() != 1 || applied.rule().position().number().get() != 2 {
//!             return Err("unexpected applied rule attempt".into());
//!         }
//!     }
//!     BorrowedRuleAttemptTransition::Missed(_)
//!     | BorrowedRuleAttemptTransition::Stable(_)
//!     | BorrowedRuleAttemptTransition::Returned(_)
//!     | BorrowedRuleAttemptTransition::Failed(_) => return Err("expected second rule to apply".into()),
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Limits
//!
//! The [`limits`] module contains leaf domain values such as
//! [`limits::StepLimit`], [`limits::RuntimeStateByteLimit`], and
//! [`limits::RuleLimit`]. Policy traits expose those values as associated
//! constants, and structured errors echo the leaf value that rejected a measured
//! count.
//!
//! # Rule inspection
//!
//! Parsed rules are exposed as borrowed structured views, not as stored source
//! strings:
//!
//! ```
//! use rsaeb::inspect::{RuleActionView, RuleAnchor, RuleRepeat};
//! use rsaeb::policy::DefaultParsePolicy;
//! use rsaeb::program::Program;
//! use rsaeb::source::ProgramSource;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::<DefaultParsePolicy>::parse(ProgramSource::from_text("( once ) ( start ) a = ( end ) b # comment"))?;
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
//! use rsaeb::trace::{BorrowedTrace, BorrowedTraceEvent};
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{DefaultParsePolicy, DefaultRuntimeInputPolicy, StaticExecutionPolicy};
//! use rsaeb::program::Program;
//! use rsaeb::source::ProgramSource;
//!
//! type TenSteps = StaticExecutionPolicy<10, 16_777_216, 16_777_216>;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::<DefaultParsePolicy>::parse(ProgramSource::from_text("a=b\nb=(return)ok"))?;
//! let mut byte_counts = Vec::new();
//! let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
//! let admitted = input.admit::<TenSteps>()?;
//!
//! program.trace(
//!     admitted,
//!     BorrowedTrace::new(|event| {
//!         byte_counts.push(event.byte_count().get());
//!         if let BorrowedTraceEvent::Step { rule, .. } = event {
//!             let _line = rule.line_number();
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
//! Snapshot tracing materializes owned event bytes under an explicit snapshot
//! byte budget, which lets the caller retain events after each callback returns:
//!
//! ```
//! use rsaeb::trace::{SnapshotTrace, TraceSnapshotEffect, TraceSnapshotEvent};
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{
//!     DefaultParsePolicy, DefaultRuntimeInputPolicy, StaticExecutionPolicy,
//!     StaticTraceSnapshotPolicy,
//! };
//! use rsaeb::program::Program;
//! use rsaeb::source::ProgramSource;
//!
//! type TenSteps = StaticExecutionPolicy<10, 16_777_216, 16_777_216>;
//! type SnapshotBytes = StaticTraceSnapshotPolicy<16_777_216>;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::<DefaultParsePolicy>::parse(ProgramSource::from_text("a=b\nb=(return)ok"))?;
//! let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
//! let admitted = input.admit::<TenSteps>()?;
//! let mut states = Vec::new();
//! let mut returns = Vec::new();
//!
//! program.trace(
//!     admitted,
//!     SnapshotTrace::<SnapshotBytes, _>::new(|event| {
//!         match event {
//!             TraceSnapshotEvent::Initial { state } => states.push(state.into_raw_bytes()),
//!             TraceSnapshotEvent::Step {
//!                 effect: TraceSnapshotEffect::Continue { state },
//!                 ..
//!             } => states.push(state.into_raw_bytes()),
//!             TraceSnapshotEvent::Step {
//!                 effect: TraceSnapshotEffect::Return { output },
//!                 ..
//!             } => returns.push(output.into_raw_bytes()),
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
//! Allocation reservation failures include a typed
//! [`error::RequestedCapacity`] instead of only a formatted string.
//! Representation failures are distinct from allocation failures, and runtime
//! contradictions that public construction paths cannot express are eliminated
//! by typed witnesses instead of becoming hidden panics or display-only errors.
//!
//! ```
//! use rsaeb::error::RuntimeInputError;
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::StaticRuntimeInputPolicy;
//!
//! type HostInput = StaticRuntimeInputPolicy<4>;
//!
//! fn validate_host_input(bytes: &[u8]) -> Result<RuntimeInput<HostInput>, RuntimeInputError> {
//!     RuntimeInput::<HostInput>::validate(RuntimeInputSource::from_bytes(bytes))
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
