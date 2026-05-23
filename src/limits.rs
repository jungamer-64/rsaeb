//! Runtime budgets and public byte-count value types.
//!
//! Parser limits, runtime limits, and trace snapshot limits are separate
//! domains. Parser limits bound source ingestion and parsed program size;
//! runtime limits bind input admission and decide whether execution may allocate
//! or continue; trace snapshot limits decide whether a borrowed trace event may
//! be materialized as owned bytes. Count types report measured lengths without
//! erasing those domains into plain `usize` values.
//!
//! ```
//! use rsaeb::limits::{
//!     ReturnByteLimit, RunLimits, RuntimeInputByteLimit, RuntimeStateByteLimit, StepLimit,
//!     TraceSnapshotByteLimit,
//! };
//!
//! let run_limits = RunLimits::new(
//!     RuntimeInputByteLimit::new(4096),
//!     StepLimit::new(100),
//!     RuntimeStateByteLimit::new(4096),
//!     ReturnByteLimit::new(1024),
//! );
//!
//! assert_eq!(run_limits.input_byte_limit().get(), 4096);
//! assert_eq!(run_limits.step_limit().get(), 100);
//! assert_eq!(TraceSnapshotByteLimit::new(2048).get(), 2048);
//! ```

pub use crate::bytes::{
    PayloadByteCount, ReturnOutputByteCount, RuntimeInputByteCount, RuntimeStateByteCount,
    TraceSnapshotByteCount,
};
pub use crate::program::limits::{
    CodeLineByteCount, CodeLineByteLimit, DEFAULT_MAX_CODE_LINE_LEN, DEFAULT_MAX_INPUT_LEN,
    DEFAULT_MAX_PAYLOAD_LEN, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_RULES, DEFAULT_MAX_SOURCE_LEN,
    DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_STEPS, DEFAULT_MAX_TRACE_SNAPSHOT_LEN, DEFAULT_PARSE_LIMITS,
    ParseLimits, PayloadByteLimit, ReturnByteLimit, RuleLimit, RunLimits, RuntimeInputByteLimit,
    RuntimeStateByteLimit, SourceByteCount, SourceByteLimit, StepCount, StepLimit,
    TraceSnapshotByteLimit,
};
