//! Structured error types for parsing, input validation, running, and tracing.
//!
//! The interpreter reports errors as structured data first. Human-readable text
//! is kept in formatting implementations, so parser and runtime code construct
//! typed reasons instead of scattering presentation strings across the core.
//!
//! The main domains are:
//!
//! - [`ParseError`] for source syntax and parser allocation failures.
//! - [`RuntimeInputError`] for raw input bytes rejected before execution.
//! - [`RunError`] for execution-time allocation, state-size, budget failures,
//!   and runtime invariant failures that public inputs should not be able to
//!   construct.
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
//! use rsaeb::limits::RuntimeInputByteLimit;
//! use rsaeb::{RuntimeInput, RuntimeInputSource};
//!
//! fn validate(bytes: &[u8]) -> Result<RuntimeInput, RuntimeInputError> {
//!     RuntimeInput::validate(
//!         RuntimeInputSource::from_bytes(bytes),
//!         RuntimeInputByteLimit::new(8),
//!     )
//! }
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let Err(error) = validate(&[b'a', 0xff]) else {
//!     return Err("expected invalid input".into());
//! };
//!
//! assert!(matches!(
//!     error,
//!     RuntimeInputError::NonAscii { column, byte }
//!         if column.get() == 2 && byte.get() == 0xff
//! ));
//! # Ok(())
//! # }
//! ```

mod fmt;
mod parse;
mod run;
mod traced;

pub use crate::allocation::{
    AllocationContext, AllocationError, AllocationErrorKind, RequestedCapacity,
};
pub use crate::bytes::{
    NonAsciiCodeByte, NonAsciiInputByte, NonPrintableCodeByte, ReservedSyntaxByte,
};
pub use parse::{
    LeftModifierKind, ParseError, ParseErrorKind, ParseErrorLocation, ParseLimitError, PayloadKind,
    RightActionKind,
};
pub use run::{
    InputColumn, InternalInvariantError, LimitError, RunError, RuntimeInputError,
    StateLimitContext, StateSizeError,
};
pub use traced::{TraceSnapshotError, TraceSnapshotRunError, TracedRunError};
