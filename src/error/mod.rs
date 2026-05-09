//! Structured error types for parsing, running, and tracing A=B programs.
//!
//! The interpreter reports errors as structured data first. Human-readable text
//! is kept in `fmt`, so parser/runtime code constructs typed reasons instead of
//! scattering presentation strings across the core.

mod fmt;
mod parse;
mod run;
mod traced;

use core::error::Error;

pub use parse::{ParseError, ParseErrorKind, PayloadKind};
pub use run::{
    InputError, ReturnLimitError, RunError, StateLimitContext, StateLimitError, StateSizeError,
    StepLimitError, TraceLimitError,
};
pub use traced::TracedRunError;

/// Top-level interpreter error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AebError {
    /// Source program parse error.
    Parse(ParseError),
    /// Runtime execution error.
    Run(RunError),
}

impl Error for AebError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Parse(error) => Some(error),
            Self::Run(error) => Some(error),
        }
    }
}

impl From<ParseError> for AebError {
    fn from(value: ParseError) -> Self {
        Self::Parse(value)
    }
}

impl From<RunError> for AebError {
    fn from(value: RunError) -> Self {
        Self::Run(value)
    }
}
