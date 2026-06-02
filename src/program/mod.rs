//! Parsed program and run-to-completion result types.
//!
//! A=B source parsing is split by expected program shape. Hosts parse
//! [`ExecutableProgramSource`](crate::source::ExecutableProgramSource) or
//! [`EmptyProgramSource`](crate::source::EmptyProgramSource) under a
//! [`ParsePolicy`](crate::policy::ParsePolicy) directly into the matching
//! [`ExecutableProgram`] or [`EmptyProgram`].
//!
//! [`ExecutableProgram`] owns immutable syntax and rule metadata only. Per-run
//! `(once)` availability, runtime bytes, completed-step counts, and execution
//! budgets are created from an [`input::AdmittedRun`](crate::input::AdmittedRun)
//! each time execution starts. This keeps parsed source reuse separate from
//! mutable runtime progress.

/// Classified parsed-program shapes.
mod executable;
/// Parser limit value types and defaults.
pub(crate) mod limits;
/// Run result and output byte domains.
mod result;
/// Parsed rule table storage.
mod rule_set;

pub(crate) use rule_set::RuleSetShape;
pub(crate) use rule_set::{ExecutableRuleSet, RuleScan};
pub(crate) use rule_set::{RuleSet, RuleSetBuilder};

pub use executable::{EmptyProgram, ExecutableProgram, ExecutableProgramRef};
pub use result::{ReturnOutput, ReturnOutputView, RunOutcome, RunResult, RuntimeStateSnapshot};
