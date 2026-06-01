//! Public stepwise and rule-attempt run typestates.
//!
//! [`Program::execute`](crate::program::Program::execute) runs to completion.
//! Stepwise and rule-attempt execution start only after
//! [`Program::as_executable`](crate::program::Program::as_executable) or
//! [`Program::into_executable`](crate::program::Program::into_executable)
//! proves that the parsed program has at least one executable rule.
//!
//! A step transition is a typestate value, not a status flag. Applied steps
//! carry the continuation session. Stable and returned states are terminal.
//! Failed states are also terminal for the borrowed API: they preserve the
//! uncommitted state for diagnostics and then let the caller discard the run
//! into its [`RunStepError`](crate::error::RunStepError). Owned failed states
//! additionally let the caller recover the owned parsed program or split it
//! from the error.
//! Rule-attempt transitions additionally expose typed miss reasons through
//! [`RuleMissReason`]. Stable rule-attempt terminals carry the final
//! non-applying rule directly.
//!
//! ```
//! use rsaeb::error::RunStepError;
//! use rsaeb::execution::BorrowedStepTransition;
//! use rsaeb::input::{AdmittedRun, RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{DefaultParsePolicy, DefaultRuntimeInputPolicy, StaticExecutionPolicy};
//! use rsaeb::program::Program;
//! use rsaeb::source::ProgramSource;
//!
//! type TinyState = StaticExecutionPolicy<10, 1, 16_777_216>;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::<DefaultParsePolicy>::parse(ProgramSource::from_text("a=aaaa"))?;
//! let input = RuntimeInput::<DefaultRuntimeInputPolicy>::validate(RuntimeInputSource::from_bytes(b"a"))?;
//! let executable = program.as_executable().map_err(|_| "expected executable rules")?;
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
/// Owned execution rule witnesses.
mod witness;

pub use attempt::{RuleMiss, RuleMissReason};
pub use session::{BorrowedRuleAttemptSession, BorrowedRunSession, OwnedRunSession};
pub use transition::{
    BorrowedAppliedStep, BorrowedFailedRun, BorrowedMissedRuleAttempt, BorrowedReturnedRun,
    BorrowedRuleAttemptAppliedStep, BorrowedRuleAttemptFailedRun, BorrowedRuleAttemptReturnedRun,
    BorrowedRuleAttemptStableRun, BorrowedRuleAttemptTransition, BorrowedStableRun,
    BorrowedStepTransition, OwnedAppliedStep, OwnedFailedRun, OwnedReturnedRun, OwnedStableRun,
    OwnedStepTransition,
};
pub use witness::{OwnedRuleAction, OwnedRulePayload, OwnedRuleWitness};

pub(crate) use session::{finish_borrowed_run, trace_events};
