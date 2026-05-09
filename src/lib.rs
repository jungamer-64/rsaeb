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

#[cfg(test)]
mod test_support;

mod allocation;
mod bytes;
mod error;
mod parser;
mod program;
mod rule;
mod runtime;
mod source;
mod syntax;
mod trace;

pub use allocation::{AllocationContext, AllocationError, AllocationErrorKind};
pub use error::{
    AebError, InputError, LeftModifierKind, LimitError, ParseError, ParseErrorKind, PayloadKind,
    RightActionKind, RunError, StateLimitContext, StateSizeError, TracedRunError,
};
pub use program::{
    DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_STEPS,
    DEFAULT_MAX_TRACE_SNAPSHOT_LEN, Program, ReturnByteLimit, RunLimits, RunResult, RunTermination,
    StateByteLimit, StepCount, StepLimit, TraceSnapshotByteLimit, run_bytes, run_str,
};
pub use rule::{PayloadView, RuleActionView, RuleAnchor, RulePosition, RuleRepeat, RuleView};
pub use source::{SourceColumn, SourceLineNumber, SourcePosition};
pub use trace::{
    BorrowedTraceEffect, BorrowedTraceEvent, RuntimeStateView, TraceSnapshotEffect,
    TraceSnapshotEvent,
};
