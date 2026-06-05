//! Public stepwise and rule-attempt run typestates.
//!
//! Run-to-completion, tracing, stepwise, and rule-attempt execution start only
//! from [`ExecutableProgram`](crate::program::ExecutableProgram), which is
//! produced by target-shape parse entrypoints such as
//! [`ExecutableProgram::parse_text`](crate::program::ExecutableProgram::parse_text).
//!
//! A step transition is a typestate value, not a status flag. Rewritten steps
//! carry the continuation session. Stable and returned states are terminal.
//! Failed states are also terminal for the borrowed API: they preserve the
//! uncommitted state for diagnostics and then let the caller discard the run
//! into its [`RunStepError`](crate::error::RunStepError).
//! Rule-attempt execution starts as a cursor that must be matched into a
//! continuing or final session before stepping. Continuing transitions can miss
//! and keep running; final transitions can stabilize. The two impossible
//! outcomes are absent from their transition types. Rule-attempt transitions
//! additionally expose typed miss variants through [`RuleMiss`]. Stable
//! rule-attempt terminals carry the final non-applying rule directly.
//!
//! ```
//! use rsaeb::error::RunStepError;
//! use rsaeb::execution::BorrowedStepTransition;
//! use rsaeb::input::{AdmittedRun, RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{DefaultParsePolicy, DefaultRuntimeInputPolicy, StaticExecutionPolicy};
//! use rsaeb::program::ExecutableProgram;
//!
//! type TinyState = StaticExecutionPolicy<10, 1, 16_777_216>;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let executable = ExecutableProgram::parse_text::<DefaultParsePolicy>("a=aaaa")?;
//! let input = RuntimeInput::validate::<DefaultRuntimeInputPolicy>(RuntimeInputSource::from_bytes(b"a"))?;
//! let session = executable.steps(input.admit::<TinyState>()?)?;
//!
//! let BorrowedStepTransition::Failed(failed) = session.step() else {
//!     return Err("expected oversized rewrite to fail before commit".into());
//! };
//!
//! if failed.completed_steps().get() != 0 {
//!     return Err("failed step must not commit progress".into());
//! }
//! if failed.state().materialize()?.as_slice() != b"a" {
//!     return Err("failed step must expose the uncommitted state".into());
//! }
//! if !matches!(
//!     failed.error(),
//!     RunStepError::RuntimeStateLimit(error)
//!         if error.attempted_len().get() == 4
//! ) {
//!     return Err("unexpected failed-step error".into());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Compile-time guards
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
//! Owned stepwise execution and its owned rule-witness surface have been
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
//!     let executable = ExecutableProgram::parse_text::<DefaultParsePolicy>("a=b")?;
//!     let input = RuntimeInput::validate::<DefaultRuntimeInputPolicy>(RuntimeInputSource::from_bytes(b"a"))?;
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
//!     let executable = ExecutableProgram::parse_text::<DefaultParsePolicy>("a=b")?;
//!     let input = RuntimeInput::validate::<DefaultRuntimeInputPolicy>(RuntimeInputSource::from_bytes(b"a"))?;
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
//!     let executable = ExecutableProgram::parse_text::<DefaultParsePolicy>("a=b")?;
//!     let input = RuntimeInput::validate::<DefaultRuntimeInputPolicy>(RuntimeInputSource::from_bytes(b"a"))?;
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
//! Successful rewrite and return outcomes no longer erase their action
//! provenance into [`crate::inspect::RuleView`]:
//!
//! ```compile_fail
//! use rsaeb::execution::BorrowedAppliedStep;
//! use rsaeb::inspect::RuleView;
//! use rsaeb::policy::ExecutionPolicy;
//!
//! fn erase_applied<'program, E: ExecutionPolicy>(
//!     applied: &BorrowedAppliedStep<'program, E>,
//! ) -> RuleView<'program> {
//!     applied.rule()
//! }
//! ```
//!
//! ```compile_fail
//! use rsaeb::execution::BorrowedReturnedRun;
//! use rsaeb::inspect::RuleView;
//!
//! fn erase_returned<'program>(
//!     returned: &BorrowedReturnedRun<'program>,
//! ) -> RuleView<'program> {
//!     returned.rule()
//! }
//! ```
//!
//! Old shape-erased step transition variants have been deleted:
//!
//! ```compile_fail
//! use rsaeb::execution::BorrowedStepTransition;
//! use rsaeb::policy::DefaultExecutionPolicy;
//!
//! fn invalid(transition: BorrowedStepTransition<'static, DefaultExecutionPolicy>) {
//!     match transition {
//!         BorrowedStepTransition::Applied(_) => {}
//!         _ => {}
//!     }
//! }
//! ```
//!
//! ```compile_fail
//! use rsaeb::execution::BorrowedStepTransition;
//! use rsaeb::policy::DefaultExecutionPolicy;
//!
//! fn invalid(transition: BorrowedStepTransition<'static, DefaultExecutionPolicy>) {
//!     match transition {
//!         BorrowedStepTransition::Returned(_) => {}
//!         _ => {}
//!     }
//! }
//! ```
//!
//! ```compile_fail
//! use rsaeb::execution::{
//!     BorrowedContinuingRuleAttemptTransition, BorrowedFinalRuleAttemptTransition,
//! };
//! use rsaeb::policy::{DefaultExecutionPolicy, DefaultRuleAttemptPolicy};
//!
//! fn invalid_continuing(
//!     transition: BorrowedContinuingRuleAttemptTransition<
//!         'static,
//!         DefaultExecutionPolicy,
//!         DefaultRuleAttemptPolicy,
//!     >,
//! ) {
//!     match transition {
//!         BorrowedContinuingRuleAttemptTransition::Applied(_) => {}
//!         _ => {}
//!     }
//! }
//!
//! fn invalid_final(
//!     transition: BorrowedFinalRuleAttemptTransition<
//!         'static,
//!         DefaultExecutionPolicy,
//!         DefaultRuleAttemptPolicy,
//!     >,
//! ) {
//!     match transition {
//!         BorrowedFinalRuleAttemptTransition::Returned(_) => {}
//!         _ => {}
//!     }
//! }
//! ```
//!
//! Old shape-erased miss carriers and miss variants have been deleted:
//!
//! ```compile_fail
//! use rsaeb::execution::{OnceConsumedRuleMiss, StateMismatchRuleMiss};
//!
//! fn main() {
//!     let _ = core::mem::size_of::<StateMismatchRuleMiss<'static>>();
//!     let _ = core::mem::size_of::<OnceConsumedRuleMiss<'static>>();
//! }
//! ```
//!
//! ```compile_fail
//! use rsaeb::execution::RuleMiss;
//!
//! fn invalid(miss: RuleMiss<'static>) {
//!     match miss {
//!         RuleMiss::StateMismatch(_) => {}
//!         _ => {}
//!     }
//! }
//! ```
//!
//! ```compile_fail
//! use rsaeb::execution::RuleMiss;
//!
//! fn invalid(miss: RuleMiss<'static>) {
//!     match miss {
//!         RuleMiss::OnceConsumed(_) => {}
//!         _ => {}
//!     }
//! }
//! ```

/// Type-selected execution advance kernel.
mod advance;
/// Rule miss values.
mod attempt;
/// Manual debug formatting for public typestates.
mod debug;
/// Shared mutable execution engine behind the public typestates.
mod engine;
/// Public run-session typestates.
mod session;
/// Public step and terminal transition typestates.
mod transition;
pub use attempt::RuleMiss;
pub use session::{
    BorrowedContinuingRuleAttemptSession, BorrowedFinalRuleAttemptSession,
    BorrowedRuleAttemptCursor, BorrowedRunSession,
};
pub use transition::{
    BorrowedAlwaysReturnRun, BorrowedAlwaysRewriteStep, BorrowedContinuingRuleAttemptTransition,
    BorrowedFailedRun, BorrowedFinalRuleAttemptTransition, BorrowedMissedRuleAttempt,
    BorrowedOnceReturnRun, BorrowedOnceRewriteStep, BorrowedRuleAttemptAlwaysReturnRun,
    BorrowedRuleAttemptAlwaysRewriteStep, BorrowedRuleAttemptFailedRun,
    BorrowedRuleAttemptOnceReturnRun, BorrowedRuleAttemptOnceRewriteStep,
    BorrowedRuleAttemptStableRun, BorrowedStableRun, BorrowedStepTransition,
};

pub(crate) use session::{finish_borrowed_run, trace_events};
