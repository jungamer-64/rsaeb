//! Parsed program and run-to-completion result types.
//!
//! A=B source parsing is split by target program shape. Hosts call
//! [`ExecutableProgram::parse_text`] / [`ExecutableProgram::parse_bytes`] or
//! [`EmptyProgram::parse_text`] / [`EmptyProgram::parse_bytes`] under a
//! [`ParsePolicy`](crate::policy::ParsePolicy).
//!
//! [`ExecutableProgram`] owns immutable syntax and rule metadata only. Per-run
//! `(once)` availability, runtime bytes, completed-step counts, and execution
//! budgets are created from an [`input::AdmittedRun`](crate::input::AdmittedRun)
//! each time execution starts. This keeps parsed source reuse separate from
//! mutable runtime progress.

/// Target-shape parsed program types.
mod executable;
/// Parser limit value types and defaults.
pub(crate) mod limits;
/// Run result and output byte domains.
mod result;
/// Parsed rule table storage.
mod rule_set;

pub(crate) use rule_set::{
    EmptyRuleSetBuilder, ExecutableRuleSet, ExecutableRuleSetBuilder, PositionedRule, RuleScan,
    RuleSink,
};

pub use executable::{EmptyProgram, ExecutableProgram};
pub use result::{ReturnOutput, ReturnOutputView, RunOutcome, RunResult, RuntimeStateSnapshot};
