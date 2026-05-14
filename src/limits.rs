//! Runtime budgets and byte-count value types.
//!
//! Limits configure execution and trace materialization. Counts report measured
//! byte lengths from parsed payloads, runtime states, return outputs, and trace
//! snapshots.

pub use crate::bytes::{
    PayloadByteCount, ReturnOutputByteCount, RuntimeStateByteCount, TraceSnapshotByteCount,
};
pub use crate::program::{
    DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_STEPS,
    DEFAULT_MAX_TRACE_SNAPSHOT_LEN, ReturnByteLimit, RunLimits, StateByteLimit, StepCount,
    StepLimit, TraceSnapshotByteLimit, TraceSnapshotLimits,
};
