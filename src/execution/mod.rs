//! Public stepwise run typestates.
//!
//! [`Program::start_run`](crate::program::Program::start_run) borrows a parsed
//! program into a [`BorrowedRunSession`]. [`Program::into_run`](crate::program::Program::into_run)
//! is the explicit owned variant for hosts that need a `'static` session.
//! [`Program::run`](crate::program::Program::run) is the borrowed
//! run-to-completion shortcut over the same admitted
//! [`RunSeed`](crate::input::RunSeed) boundary.
//! [`Program::start_rule_attempt_run`](crate::program::Program::start_rule_attempt_run)
//! and [`Program::into_rule_attempt_run`](crate::program::Program::into_rule_attempt_run)
//! use a separate rule-attempt typestate that can pause after non-matching
//! executable rule lines.
//!
//! A step transition is a typestate value, not a status flag. Applied steps
//! carry the continuation session. Stable and returned states are terminal.
//! Failed states are also terminal for the borrowed API: they preserve the
//! uncommitted state for diagnostics and then let the caller discard the run
//! into its [`RunStepError`](crate::error::RunStepError). Owned failed states
//! additionally let the caller recover the owned parsed program or split it
//! from the error.
//! Rule-attempt transitions additionally expose typed miss reasons through
//! [`RuleMissReason`], expose stable reasons through
//! [`RuleAttemptStableReason`], and
//! consume [`RuleAttemptSeed`] instead of accepting a detached
//! [`RuleAttemptLimit`](crate::limits::RuleAttemptLimit).
//!
//! ```
//! use rsaeb::error::RunStepError;
//! use rsaeb::execution::BorrowedStepTransition;
//! use rsaeb::input::{RunSeed, RuntimeInput, RuntimeInputSource};
//! use rsaeb::limits::{
//!     DEFAULT_MAX_INPUT_LEN, DEFAULT_MAX_RETURN_LEN, DEFAULT_PARSE_LIMITS, ExecutionLimits,
//!     RuntimeInputLimits, RuntimeStateByteLimit, StepLimit,
//! };
//! use rsaeb::program::Program;
//! use rsaeb::source::ProgramSource;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::parse(ProgramSource::from_text("a=aaaa"), DEFAULT_PARSE_LIMITS)?;
//! let input_limits = RuntimeInputLimits::new(DEFAULT_MAX_INPUT_LEN);
//! let execution_limits = ExecutionLimits::new(
//!     StepLimit::new(10),
//!     RuntimeStateByteLimit::new(1),
//!     DEFAULT_MAX_RETURN_LEN,
//! );
//! let input = RuntimeInput::validate(RuntimeInputSource::from_bytes(b"a"), input_limits)?;
//! let session = program.start_run(RunSeed::admit(input, execution_limits)?)?;
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

/// Rule-attempt admission witness.
mod admission;
/// Rule miss and stable-reason values.
mod attempt;
/// Internal transition constructors from engine states.
mod construction;
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

pub use admission::RuleAttemptSeed;
pub use attempt::{RuleAttemptStableReason, RuleMiss, RuleMissReason};
pub use session::{
    BorrowedRuleAttemptSession, BorrowedRunSession, OwnedRuleAttemptSession, OwnedRunSession,
};
pub use transition::{
    BorrowedAppliedStep, BorrowedFailedRun, BorrowedMissedRuleAttempt, BorrowedReturnedRun,
    BorrowedRuleAttemptAppliedStep, BorrowedRuleAttemptFailedRun, BorrowedRuleAttemptReturnedRun,
    BorrowedRuleAttemptStableRun, BorrowedRuleAttemptTransition, BorrowedStableRun,
    BorrowedStepTransition, OwnedAppliedStep, OwnedFailedRun, OwnedMissedRuleAttempt,
    OwnedReturnedRun, OwnedRuleAttemptAppliedStep, OwnedRuleAttemptFailedRun,
    OwnedRuleAttemptReturnedRun, OwnedRuleAttemptStableRun, OwnedRuleAttemptTransition,
    OwnedStableRun, OwnedStepTransition,
};
pub use witness::{OwnedRulePayload, OwnedRuleWitness};

pub(crate) use session::{finish_borrowed_run, run_with_borrowed_trace};
