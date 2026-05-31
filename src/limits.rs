//! Runtime budgets and public byte-count value types.
//!
//! Parser, runtime-input, execution, rule-attempt, and trace snapshot policies
//! expose these leaf values as associated constants. Count types report
//! measured lengths without erasing those domains into plain `usize` values.
//!
//! Limit values describe one domain budget. Count values are observations
//! produced by parser, input, execution, or trace code. Keeping those roles in
//! distinct types prevents a source length, runtime input length, runtime state
//! length, return-output length, or trace-snapshot length from crossing into the
//! wrong budget by accident.
//!
//! ```
//! use rsaeb::limits::{
//!     ReturnByteLimit, RuntimeInputByteLimit, RuleAttemptLimit, RuntimeStateByteLimit,
//!     StepLimit, TraceSnapshotByteLimit,
//! };
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! if RuntimeInputByteLimit::new(4096).get() != 4096 {
//!     return Err("unexpected input limit".into());
//! }
//! if StepLimit::new(100).get() != 100 {
//!     return Err("unexpected step limit".into());
//! }
//! if RuntimeStateByteLimit::new(4096).get() != 4096 {
//!     return Err("unexpected state limit".into());
//! }
//! if ReturnByteLimit::new(1024).get() != 1024 {
//!     return Err("unexpected return limit".into());
//! }
//! if RuleAttemptLimit::new(500).get() != 500 {
//!     return Err("unexpected rule-attempt limit".into());
//! }
//! if TraceSnapshotByteLimit::new(2048).get() != 2048 {
//!     return Err("unexpected trace limit".into());
//! }
//! # Ok(())
//! # }
//! ```

pub use crate::bytes::{
    PayloadByteCount, ReturnOutputByteCount, RuntimeInputByteCount, RuntimeStateByteCount,
    TraceSnapshotByteCount,
};
pub use crate::program::limits::{
    CodeLineByteCount, CodeLineByteLimit, PayloadByteLimit, ReturnByteLimit, RuleAttemptCount,
    RuleAttemptLimit, RuleLimit, RuntimeInputByteLimit, RuntimeStateByteLimit, SourceByteCount,
    SourceByteLimit, StepCount, StepLimit, TraceSnapshotByteLimit,
};
