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
//! - [`RunError`] for execution-time allocation, state-size, and budget
//!   failures.
//! - [`TraceSnapshotError`] and the traced run wrappers for trace
//!   materialization or user callback failures.
//! - [`AebError`] for callers that want a parse/input/run umbrella while still
//!   preserving the structured inner error.
//!
//! ```
//! use rsaeb::error::{AebError, RuntimeInputError};
//! use rsaeb::limits::RuntimeInputByteLimit;
//! use rsaeb::RuntimeInput;
//!
//! fn validate(bytes: &[u8]) -> Result<RuntimeInput, AebError> {
//!     RuntimeInput::validate(bytes, RuntimeInputByteLimit::new(8)).map_err(AebError::from)
//! }
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let Err(error) = validate(&[b'a', 0xff]) else {
//!     return Err("expected invalid input".into());
//! };
//!
//! assert!(matches!(
//!     error,
//!     AebError::Input(RuntimeInputError::NonAscii { column, byte })
//!         if column.get() == 2 && byte.get() == 0xff
//! ));
//! # Ok(())
//! # }
//! ```

mod fmt;
mod parse;
mod run;
mod traced;

use core::error::Error;

pub use crate::allocation::{AllocationContext, AllocationError, AllocationErrorKind};
pub use crate::bytes::{
    NonAsciiCodeByte, NonAsciiInputByte, NonPrintableCodeByte, ReservedSyntaxByte,
};
pub use parse::{
    LeftModifierKind, ParseError, ParseErrorKind, ParseErrorLocation, ParseLimitError, PayloadKind,
    RightActionKind,
};
pub use run::{
    InputColumn, LimitError, RunError, RuntimeInputError, StateLimitContext, StateSizeError,
};
pub use traced::{
    FallibleTraceSnapshotRunError, TraceSnapshotError, TraceSnapshotRunError, TracedRunError,
};

/// Top-level source parsing, input validation, and runtime execution error.
///
/// This wrapper is useful at host boundaries that perform parse, input
/// validation, and execution in one operation. It does not erase the underlying
/// domain: callers can still match the inner [`ParseError`],
/// [`RuntimeInputError`], or [`RunError`] variant.
#[derive(Debug, PartialEq, Eq)]
pub enum AebError {
    /// Source program parse error.
    Parse(ParseError),
    /// Runtime input validation error.
    Input(RuntimeInputError),
    /// Runtime execution error.
    Run(RunError),
}

impl Error for AebError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Parse(error) => Some(error),
            Self::Input(error) => Some(error),
            Self::Run(error) => Some(error),
        }
    }
}

impl From<ParseError> for AebError {
    fn from(value: ParseError) -> Self {
        Self::Parse(value)
    }
}

impl From<RuntimeInputError> for AebError {
    fn from(value: RuntimeInputError) -> Self {
        Self::Input(value)
    }
}

impl From<RunError> for AebError {
    fn from(value: RunError) -> Self {
        Self::Run(value)
    }
}
