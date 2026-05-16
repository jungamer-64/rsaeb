//! Runtime budgets and public byte-count value types.
//!
//! Runtime limits and trace snapshot limits are separate domains. Runtime limits
//! decide whether execution may allocate or continue; trace snapshot limits
//! decide whether a borrowed trace event may be materialized as owned bytes.
//! Count types report measured byte lengths from parsed payloads, runtime
//! input, runtime states, return outputs, and trace snapshots without erasing
//! those domains into plain `usize` values.
//!
//! ```
//! use rsaeb::limits::{
//!     ReturnByteLimit, RunLimits, RuntimeStateByteLimit, StepLimit,
//!     TraceSnapshotByteLimit, TraceSnapshotLimits,
//! };
//!
//! let run_limits = RunLimits::new(
//!     StepLimit::new(100),
//!     RuntimeStateByteLimit::new(4096),
//!     ReturnByteLimit::new(1024),
//! );
//! let trace_limits = TraceSnapshotLimits::new(
//!     run_limits,
//!     TraceSnapshotByteLimit::new(2048),
//! );
//!
//! assert_eq!(trace_limits.run_limits().step_limit().get(), 100);
//! assert_eq!(trace_limits.snapshot_byte_limit().get(), 2048);
//! ```

pub use crate::bytes::{
    PayloadByteCount, ReturnOutputByteCount, RuntimeInputByteCount, RuntimeStateByteCount,
    TraceSnapshotByteCount,
};
pub use crate::program::{
    DEFAULT_MAX_INPUT_LEN, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_STEPS,
    DEFAULT_MAX_TRACE_SNAPSHOT_LEN, ReturnByteLimit, RunLimits, RuntimeInputByteLimit,
    RuntimeStateByteLimit, StepCount, StepLimit, TraceSnapshotByteLimit, TraceSnapshotLimits,
};
