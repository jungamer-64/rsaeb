//! Structured error types for parsing, input validation, running, and tracing.
//!
//! The interpreter reports errors as structured data first. Human-readable text
//! is kept in formatting implementations, so parser and runtime code construct
//! typed reasons instead of scattering presentation strings across the core.
//! Each public error type belongs to one phase boundary; callers should not
//! collapse them into one catch-all type unless their own boundary no longer
//! needs to distinguish user source, user input, run admission, runtime
//! execution, snapshot materialization, and callback failures.
//!
//! The main domains are:
//!
//! - [`ParseError`] for source syntax, parser allocation, parser
//!   representation, and parser-invariant failures.
//! - [`RuntimeInputError`] for raw input bytes rejected before execution and
//!   runtime-input witness contradictions.
//! - [`RunAdmissionError`] for validated input rejected as an initial runtime state.
//! - [`RunStartError`] for per-run setup failures, [`RunStepError`] and
//!   rule-attempt step errors for uncommitted step failures, [`RunFinishError`]
//!   for finishing an already-started run, and [`RunError`] for
//!   run-to-completion composition.
//! - [`AllocationError`] for explicit allocation boundaries such as view
//!   materialization, canonical source construction, final output conversion,
//!   and trace snapshots. [`AllocationContext`] names the failing boundary, and
//!   [`RequestedCapacity`] carries the requested vector capacity for reservation
//!   failures.
//! - [`TraceSnapshotError`] and traced run errors for trace materialization or
//!   user callback failures.
//!
//! ```
//! use rsaeb::error::RuntimeInputError;
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::StaticRuntimeInputPolicy;
//!
//! type Input8 = StaticRuntimeInputPolicy<8>;
//!
//! fn validate(bytes: &[u8]) -> Result<RuntimeInput<Input8>, RuntimeInputError> {
//!     RuntimeInput::<Input8>::validate(RuntimeInputSource::from_bytes(bytes))
//! }
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let Err(error) = validate(&[b'a', 0xff]) else {
//!     return Err("expected invalid input".into());
//! };
//!
//! if !matches!(
//!     error,
//!     RuntimeInputError::NonAscii { column, byte }
//!         if column.get() == 2 && byte.get() == 0xff
//! ) {
//!     return Err("unexpected input error".into());
//! }
//! # Ok(())
//! # }
//! ```

/// Display implementations for public error domains.
mod fmt;
/// Parse error model.
mod parse;
/// Runtime and admission error model.
mod run;
/// Trace error model.
mod traced;

pub use crate::allocation::{
    AllocationContext, AllocationError, AllocationErrorKind, RequestedCapacity,
};
pub use crate::bytes::{
    NonAsciiCodeByte, NonAsciiInputByte, NonPrintableCodeByte, ReservedSyntaxByte,
};
pub use parse::{
    LeftModifierKind, ParseError, ParseErrorKind, ParseErrorLocation, ParseLimitError,
    ParseRepresentationError, PayloadKind, RightActionKind,
};
pub use run::{
    InputColumn, OwnedRuleAttemptStepError, OwnedRunStepError, ReturnOutputLimitError,
    RewriteSizeError, RuleAttemptLimitError, RuleAttemptStepError, RuleRuntimeStateError,
    RunAdmissionError, RunError, RunFinishError, RunStartError, RunStepError, RuntimeInputError,
    RuntimeStateLimitError, StepLimitError,
};
pub use traced::{TraceSnapshotError, TraceSnapshotRunError, TracedRunError};
