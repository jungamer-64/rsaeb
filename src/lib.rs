//! Library API for the A=B rewrite interpreter.
//!
//! The crate exposes a byte-oriented parser and runtime. Program syntax and
//! runtime input are separate domains:
//!
//! - program code is compact printable ASCII syntax;
//! - comments are ignored bytes after `#`;
//! - runtime input is ASCII data and may contain whitespace/reserved bytes;
//! - program payloads cannot contain whitespace, reserved syntax characters, or
//!   non-ASCII/control bytes.
//!
//! Files, stdout, stderr, argument parsing, and lossy display formatting are
//! intentionally outside this library. The command-line binary can do command-
//! line concerns without coupling them to the interpreter core.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

#[cfg(test)]
extern crate std;

mod allocation;
mod bytes;
mod error;
mod parser;
mod program;
mod rule;
mod runtime;
mod trace;

pub use allocation::{AllocationContext, AllocationError};
pub use error::{
    AebError, InputError, ParseError, ParseErrorKind, PayloadKind, RunError, StateSizeError,
    StepLimitError, TracedRunError,
};
pub use program::{run, Program, RunOptions, RunResult, RunTermination, DEFAULT_MAX_STEPS};
pub use rule::{RuleInfo, RulePosition};
pub use trace::{TraceEffect, TraceEvent};
