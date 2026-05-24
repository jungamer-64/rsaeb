//! Runtime budgets and public byte-count value types.
//!
//! Parser limits, runtime input limits, execution limits, and trace snapshot limits are separate
//! domains. Parser limits bound source ingestion and parsed program size;
//! runtime input limits bind raw input validation; execution limits decide
//! whether execution may allocate or continue; rule-attempt limits decide how
//! many executable rule lines a rule-attempt session may consume; trace
//! snapshot limits decide whether a borrowed trace event may be materialized as
//! owned bytes. Count types report measured lengths without erasing those
//! domains into plain `usize` values.
//!
//! Limits are policy values supplied by the host. Count values are observations
//! produced by parser, input, execution, or trace code. Keeping those roles in
//! distinct types prevents a source length, runtime input length, runtime state
//! length, return-output length, or trace-snapshot length from crossing into
//! the wrong budget by accident.
//!
//! ```
//! use rsaeb::limits::{
//!     ExecutionLimits, ReturnByteLimit, RuntimeInputByteLimit, RuntimeInputLimits,
//!     RuleAttemptLimit, RuntimeStateByteLimit, StepLimit, TraceSnapshotByteLimit,
//! };
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let input_limits = RuntimeInputLimits::new(RuntimeInputByteLimit::new(4096));
//! let execution_limits = ExecutionLimits::new(
//!     StepLimit::new(100),
//!     RuntimeStateByteLimit::new(4096),
//!     ReturnByteLimit::new(1024),
//! );
//!
//! if input_limits.input_byte_limit().get() != 4096 {
//!     return Err("unexpected input limit".into());
//! }
//! if execution_limits.step_limit().get() != 100 {
//!     return Err("unexpected step limit".into());
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
    CodeLineByteCount, CodeLineByteLimit, DEFAULT_MAX_CODE_LINE_LEN, DEFAULT_MAX_INPUT_LEN,
    DEFAULT_MAX_PAYLOAD_LEN, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_RULE_ATTEMPTS, DEFAULT_MAX_RULES,
    DEFAULT_MAX_SOURCE_LEN, DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_STEPS,
    DEFAULT_MAX_TRACE_SNAPSHOT_LEN, DEFAULT_PARSE_LIMITS, ExecutionLimits, ParseLimits,
    PayloadByteLimit, ReturnByteLimit, RuleAttemptCount, RuleAttemptLimit, RuleLimit,
    RuntimeInputByteLimit, RuntimeInputLimits, RuntimeStateByteLimit, SourceByteCount,
    SourceByteLimit, StepCount, StepLimit, TraceSnapshotByteLimit,
};
