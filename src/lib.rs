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
//!   when it contains no executable rules. Empty programs expose inspection and
//!   [`program::EmptyProgram::stabilize`] only.
//! - [`program::ExecutableProgram::parse_text`] and
//!   [`program::ExecutableProgram::parse_bytes`] accept syntactically valid source
//!   only when it contains at least one executable rule. Execution, tracing,
//!   stepwise execution, and rule-attempt execution exist only on this type.
//!
//! `(once)` repeat intent and right-side action shape are parsed into the rule
//! variant itself. Every run builds its own per-rule runtime availability cells,
//! so consumed once rules are filtered before matching and once-state mismatch
//! is not a public runtime error class.
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
//! # Compile-time API guards
//!
//! The shape-erased public `Program` type has been deleted:
//!
//! ```compile_fail
//! use rsaeb::program::Program;
//!
//! fn main() {}
//! ```
//!
//! The shape-erased public `ParsedProgram` classifier has been deleted:
//!
//! ```compile_fail
//! use rsaeb::program::ParsedProgram;
//!
//! fn main() {}
//! ```
//!
//! The shape-neutral public `ProgramSource` boundary has been deleted:
//!
//! ```compile_fail
//! use rsaeb::source::ProgramSource;
//!
//! fn main() {}
//! ```
//!
//! Public source-shape marker types have been deleted:
//!
//! ```compile_fail
//! use rsaeb::source::{EmptyProgramSource, ExecutableProgramSource};
//!
//! fn main() {}
//! ```
//!
//! The old source-marker parse method has been deleted:
//!
//! ```compile_fail
//! use rsaeb::policy::DefaultParsePolicy;
//! use rsaeb::program::ExecutableProgram;
//!
//! fn main() {
//!     let _ = ExecutableProgram::<DefaultParsePolicy>::parse(b"a=b");
//! }
//! ```
//!
//! Target-shape parse entrypoints still require an explicit parse policy:
//!
//! ```compile_fail
//! use rsaeb::program::ExecutableProgram;
//!
//! fn main() {
//!     let _ = ExecutableProgram::parse_text("a=b");
//! }
//! ```
//!
//! Old executable and empty wrapper types have been deleted:
//!
//! ```compile_fail
//! use rsaeb::program::{
//!     BorrowedEmptyProgram, BorrowedExecutableProgram, OwnedEmptyProgram,
//!     OwnedExecutableProgram,
//! };
//!
//! fn main() {}
//! ```
//!
//! Flat rule repeat/action inspection and nested repeat/action inspection have
//! been deleted. Each [`inspect::RuleView`] variant now names the complete rule
//! shape:
//!
//! ```compile_fail
//! use rsaeb::inspect::{RepeatRuleView, RuleActionView, RuleRepeat};
//!
//! fn main() {}
//! ```
//!
//! ```compile_fail
//! use rsaeb::inspect::RuleView;
//!
//! fn main() {
//!     let _ = |rule: RuleView<'_>| rule.repeat();
//! }
//! ```
//!
//! ```compile_fail
//! use rsaeb::inspect::RuleView;
//!
//! fn main() {
//!     let _ = |rule: RuleView<'_>| rule.action();
//! }
//! ```
//!
//! ```compile_fail
//! use rsaeb::inspect::RuleView;
//!
//! fn main() {
//!     let _ = |rule: RuleView<'_>| matches!(rule, RuleView::Always(_));
//! }
//! ```
//!
//! Rule-attempt misses no longer expose a loosely paired reason enum:
//!
//! ```compile_fail
//! use rsaeb::execution::RuleMissReason;
//!
//! fn main() {}
//! ```
//!
//! ```compile_fail
//! use rsaeb::execution::RuleMiss;
//!
//! fn main() {
//!     let _ = |miss: RuleMiss<'_>| miss.reason();
//! }
//! ```
//!
//! Once-state mismatch is no longer a reportable execution error:
//!
//! ```compile_fail
//! use rsaeb::error::OnceRuleStateError;
//!
//! fn main() {}
//! ```
//!
//! ```compile_fail
//! use rsaeb::error::RunStepError;
//!
//! fn main() {
//!     let _ = |error: RunStepError| match error {
//!         RunStepError::OnceRuleState(_) => true,
//!         _ => false,
//!     };
//! }
//! ```
//!
//! Runtime execution mode selectors and old method-shaped entrypoints are not
//! part of the public API:
//!
//! ```compile_fail
//! use rsaeb::execution::{
//!     BorrowedExecutionMode, BorrowedRuleAttempts, BorrowedSteps, CompleteRun,
//!     OwnedExecutionMode, OwnedRuleAttempts, OwnedSteps,
//! };
//!
//! fn main() {}
//! ```
//!
//! Owned stepwise execution and its owned rule-witness/error surface have been
//! deleted. Stepwise execution borrows an executable program so the runtime rule
//! table can stay tied to the parsed rule table:
//!
//! ```compile_fail
//! use rsaeb::execution::{
//!     OwnedAppliedStep, OwnedFailedRun, OwnedReturnedRun, OwnedRunSession,
//!     OwnedStableRun, OwnedStepTransition,
//! };
//!
//! fn main() {}
//! ```
//!
//! ```compile_fail
//! use rsaeb::execution::{OwnedRuleAction, OwnedRulePayload, OwnedRuleWitness};
//!
//! fn main() {}
//! ```
//!
//! ```compile_fail
//! use rsaeb::error::OwnedRunStepError;
//!
//! fn main() {}
//! ```
//!
//! Shape-erased rule-attempt session and transition types have been deleted:
//!
//! ```compile_fail
//! use rsaeb::execution::{BorrowedRuleAttemptSession, BorrowedRuleAttemptTransition};
//!
//! fn main() {}
//! ```
//!
//! ```compile_fail
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{
//!     DefaultExecutionPolicy, DefaultParsePolicy, DefaultRuleAttemptPolicy,
//!     DefaultRuntimeInputPolicy,
//! };
//! use rsaeb::program::ExecutableProgram;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let executable = ExecutableProgram::<DefaultParsePolicy>::parse_text("a=b")?;
//!     let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
//!     let admitted = input.admit::<DefaultExecutionPolicy>()?;
//!     let _ = executable.into_steps(admitted)?;
//!     Ok(())
//! }
//! ```
//!
//! Rule-attempt cursors must be matched before stepping:
//!
//! ```compile_fail
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{
//!     DefaultExecutionPolicy, DefaultParsePolicy, DefaultRuleAttemptPolicy,
//!     DefaultRuntimeInputPolicy,
//! };
//! use rsaeb::program::ExecutableProgram;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let executable = ExecutableProgram::<DefaultParsePolicy>::parse_text("a=b")?;
//!     let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
//!     let admitted = input.admit::<DefaultExecutionPolicy>()?;
//!     let cursor = executable.rule_attempts::<DefaultRuleAttemptPolicy, _>(admitted)?;
//!     let _ = cursor.step();
//!     Ok(())
//! }
//! ```
//!
//! Continuing rule-attempt transitions cannot report stable terminals:
//!
//! ```compile_fail
//! use rsaeb::execution::BorrowedContinuingRuleAttemptTransition;
//! use rsaeb::policy::{DefaultExecutionPolicy, DefaultParsePolicy, DefaultRuleAttemptPolicy};
//!
//! fn main() {
//!     let _ = |transition: BorrowedContinuingRuleAttemptTransition<
//!         'static,
//!         DefaultParsePolicy,
//!         DefaultExecutionPolicy,
//!         DefaultRuleAttemptPolicy,
//!     >| match transition {
//!         BorrowedContinuingRuleAttemptTransition::Stable(_) => true,
//!         _ => false,
//!     };
//! }
//! ```
//!
//! Final rule-attempt transitions cannot report missed continuations:
//!
//! ```compile_fail
//! use rsaeb::execution::BorrowedFinalRuleAttemptTransition;
//! use rsaeb::policy::{DefaultExecutionPolicy, DefaultParsePolicy, DefaultRuleAttemptPolicy};
//!
//! fn main() {
//!     let _ = |transition: BorrowedFinalRuleAttemptTransition<
//!         'static,
//!         DefaultParsePolicy,
//!         DefaultExecutionPolicy,
//!         DefaultRuleAttemptPolicy,
//!     >| match transition {
//!         BorrowedFinalRuleAttemptTransition::Missed(_) => true,
//!         _ => false,
//!     };
//! }
//! ```
//!
//! Rule-attempt continuations return cursors, not old shape-erased sessions:
//!
//! ```compile_fail
//! use rsaeb::execution::BorrowedRuleAttemptAppliedStep;
//! use rsaeb::policy::{ExecutionPolicy, ParsePolicy, RuleAttemptPolicy};
//!
//! fn old_continuation<'program, P, E, A>(
//!     applied: BorrowedRuleAttemptAppliedStep<'program, P, E, A>,
//! )
//! where
//!     P: ParsePolicy,
//!     E: ExecutionPolicy,
//!     A: RuleAttemptPolicy,
//! {
//!     let _ = applied.into_session();
//! }
//! ```
//!
//! Empty-program witnesses expose stabilization only; they cannot start
//! execution, tracing, stepwise execution, or rule-attempt execution:
//!
//! ```compile_fail
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{DefaultExecutionPolicy, DefaultParsePolicy, DefaultRuntimeInputPolicy};
//! use rsaeb::program::EmptyProgram;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let empty = EmptyProgram::<DefaultParsePolicy>::parse_text("# empty")?;
//!     let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
//!     let admitted = input.admit::<DefaultExecutionPolicy>()?;
//!     let _ = empty.execute(admitted)?;
//!     Ok(())
//! }
//! ```
//!
//! Execution entrypoints require admitted input. Validated input cannot be run
//! directly:
//!
//! ```compile_fail
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{DefaultParsePolicy, DefaultRuntimeInputPolicy};
//! use rsaeb::program::ExecutableProgram;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let executable = ExecutableProgram::<DefaultParsePolicy>::parse_text("a=b")?;
//!     let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
//!     let _ = executable.execute(input)?;
//!     Ok(())
//! }
//! ```
//!
//! Terminal transitions do not carry a continuation session, so they cannot be
//! stepped again:
//!
//! ```compile_fail
//! use rsaeb::execution::BorrowedStepTransition;
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{DefaultExecutionPolicy, DefaultParsePolicy, DefaultRuntimeInputPolicy};
//! use rsaeb::program::ExecutableProgram;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let executable = ExecutableProgram::<DefaultParsePolicy>::parse_text("a=b")?;
//!     let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
//!     let admitted = input.admit::<DefaultExecutionPolicy>()?;
//!     let terminal = match executable.steps(admitted)?.step() {
//!         BorrowedStepTransition::Stable(stable) => stable,
//!         _ => return Ok(()),
//!     };
//!     let _ = terminal.step();
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
//! let executable = ExecutableProgram::<DefaultParsePolicy>::parse_text("a=b")?;
//! let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
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
//! let empty = EmptyProgram::<DefaultParsePolicy>::parse_text("# empty")?;
//! let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"unchanged"))?;
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
//! let executable = ExecutableProgram::<DefaultParsePolicy>::parse_text("(once)a=b\na=c")?;
//!
//! let first_input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"aa"))?;
//! let second_input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"aa"))?;
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
//! let executable = ExecutableProgram::<DefaultParsePolicy>::parse_text("a=b")?;
//! let input = RuntimeInput::<TinyInput>::validate(RuntimeInputSource::from_bytes(b"a"))?;
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
//! let executable = ExecutableProgram::<DefaultParsePolicy>::parse_text("a=b\nb=c")?;
//! let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
//! let admitted = input.admit::<TenSteps>()?;
//! let execution = executable.steps(admitted)?;
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
//! match execution.step() {
//!     BorrowedStepTransition::Applied(applied) => {
//!         if applied.rule().position().number().get() != 2 {
//!             return Err("unexpected second applied rule".into());
//!         }
//!     }
//!     BorrowedStepTransition::Stable(_) | BorrowedStepTransition::Returned(_) | BorrowedStepTransition::Failed(_) => {
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
//! let executable = ExecutableProgram::<DefaultParsePolicy>::parse_text("z=x\na=b")?;
//! let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
//! let admitted = input.admit::<TenSteps>()?;
//! let execution = executable.rule_attempts::<TenAttempts, _>(admitted)?;
//!
//! let BorrowedRuleAttemptCursor::Continuing(execution) = execution else {
//!     return Err("expected first rule to have a successor".into());
//! };
//! let execution = match execution.step() {
//!     BorrowedContinuingRuleAttemptTransition::Missed(missed) => {
//!         if !matches!(missed.miss(), RuleMiss::StateMismatch(_)) {
//!             return Err("unexpected miss shape".into());
//!         }
//!         missed.into_cursor()
//!     }
//!     BorrowedContinuingRuleAttemptTransition::Applied(_)
//!     | BorrowedContinuingRuleAttemptTransition::Returned(_)
//!     | BorrowedContinuingRuleAttemptTransition::Failed(_) => return Err("expected first rule to miss".into()),
//! };
//!
//! let BorrowedRuleAttemptCursor::Final(execution) = execution else {
//!     return Err("expected final cursor after first miss".into());
//! };
//! match execution.step() {
//!     BorrowedFinalRuleAttemptTransition::Applied(applied) => {
//!         if applied.step().get() != 1 || applied.rule().position().number().get() != 2 {
//!             return Err("unexpected applied rule attempt".into());
//!         }
//!     }
//!     BorrowedFinalRuleAttemptTransition::Stable(_)
//!     | BorrowedFinalRuleAttemptTransition::Returned(_)
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
//! let executable = ExecutableProgram::<DefaultParsePolicy>::parse_text("( once ) ( start ) a = ( end ) b # comment")?;
//! let rule = executable.rules().next().ok_or("missing parsed rule")?;
//!
//! if rule.anchor() != RuleAnchor::Start {
//!     return Err("unexpected anchor".into());
//! }
//! if rule.lhs().materialize()?.as_slice() != b"a" {
//!     return Err("unexpected left side".into());
//! }
//! match rule {
//!     RuleView::OnceRewrite(rewrite) => match rewrite.rewrite_action() {
//!         RewriteActionView::MoveEnd(payload) => {
//!             if payload.materialize()?.as_slice() != b"b" {
//!                 return Err("unexpected moved payload".into());
//!             }
//!         }
//!         RewriteActionView::Replace(_) | RewriteActionView::MoveStart(_) => {
//!             return Err("expected move-end rewrite action".into());
//!         }
//!     },
//!     RuleView::AlwaysRewrite(_)
//!     | RuleView::AlwaysReturn(_)
//!     | RuleView::OnceReturn(_) => return Err("expected once rewrite rule".into()),
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
//! let executable = ExecutableProgram::<DefaultParsePolicy>::parse_text("a=b\nb=(return)ok")?;
//! let mut byte_counts = Vec::new();
//! let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
//! let admitted = input.admit::<TenSteps>()?;
//!
//! executable.trace(
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
//! ```
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{
//!     DefaultParsePolicy, DefaultRuntimeInputPolicy, StaticExecutionPolicy,
//!     StaticTraceSnapshotPolicy,
//! };
//! use rsaeb::program::ExecutableProgram;
//! use rsaeb::trace::{SnapshotTrace, TraceSnapshotEffect, TraceSnapshotEvent};
//!
//! type TenSteps = StaticExecutionPolicy<10, 16_777_216, 16_777_216>;
//! type SnapshotBytes = StaticTraceSnapshotPolicy<16_777_216>;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let executable = ExecutableProgram::<DefaultParsePolicy>::parse_text("a=b\nb=(return)ok")?;
//! let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
//! let admitted = input.admit::<TenSteps>()?;
//! let mut states = Vec::new();
//! let mut returns = Vec::new();
//!
//! executable.trace(
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
