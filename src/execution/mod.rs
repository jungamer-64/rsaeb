//! Public stepwise and rule-attempt run typestates.
//!
//! Run-to-completion, tracing, stepwise, and rule-attempt execution start only
//! from [`ExecutableProgram`](crate::program::ExecutableProgram), which is
//! produced by [`ExecutableProgram::parse`](crate::program::ExecutableProgram::parse).
//!
//! A step transition is a typestate value, not a status flag. Applied steps
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
//! use rsaeb::source::ExecutableProgramSource;
//!
//! type TinyState = StaticExecutionPolicy<10, 1, 16_777_216>;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let executable = ExecutableProgram::<DefaultParsePolicy>::parse(ExecutableProgramSource::from_text("a=aaaa"))?;
//! let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
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
pub use attempt::{OnceConsumedRuleMiss, RuleMiss, StateMismatchRuleMiss};
pub use session::{
    BorrowedContinuingRuleAttemptSession, BorrowedFinalRuleAttemptSession,
    BorrowedRuleAttemptCursor, BorrowedRunSession,
};
pub use transition::{
    BorrowedAppliedStep, BorrowedContinuingRuleAttemptTransition, BorrowedFailedRun,
    BorrowedFinalRuleAttemptTransition, BorrowedMissedRuleAttempt, BorrowedReturnedRun,
    BorrowedRuleAttemptAppliedStep, BorrowedRuleAttemptFailedRun, BorrowedRuleAttemptReturnedRun,
    BorrowedRuleAttemptStableRun, BorrowedStableRun, BorrowedStepTransition,
};

pub(crate) use session::{finish_borrowed_run, trace_events};
