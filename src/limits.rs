//! Runtime budgets and public byte-count value types.
//!
//! Parser limits, runtime input limits, execution limits, and trace snapshot limits are separate
//! domains. Parser limits bound source ingestion and parsed program size;
//! runtime input limits bind raw input validation; execution limits decide
//! whether execution may allocate or continue; trace snapshot limits decide
//! whether a borrowed trace event may be materialized as owned bytes. Count
//! types report measured lengths without erasing those domains into plain
//! `usize` values.
//!
//! ```
//! use rsaeb::limits::{
//!     ExecutionLimits, ReturnByteLimit, RuntimeInputByteLimit, RuntimeInputLimits,
//!     RuntimeStateByteLimit, StepLimit, TraceSnapshotByteLimit,
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
    DEFAULT_MAX_PAYLOAD_LEN, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_RULES, DEFAULT_MAX_SOURCE_LEN,
    DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_STEPS, DEFAULT_MAX_TRACE_SNAPSHOT_LEN, DEFAULT_PARSE_LIMITS,
    ExecutionLimits, ParseLimits, PayloadByteLimit, ReturnByteLimit, RuleLimit,
    RuntimeInputByteLimit, RuntimeInputLimits, RuntimeStateByteLimit, SourceByteCount,
    SourceByteLimit, StepCount, StepLimit, TraceSnapshotByteLimit,
};
