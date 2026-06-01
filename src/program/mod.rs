//! Parsed program and run-to-completion result types.
//!
//! [`Program`] is the immutable parsed A=B rule table. Hosts parse typed
//! [`ProgramSource`] under a [`ParsePolicy`], then
//! run with an admitted [`input::AdmittedRun`](crate::input::AdmittedRun). Runtime budget and byte-count types
//! live in [`limits`](crate::limits); runtime input lives in [`input`](crate::input).
//!
//! A parsed program owns syntax and rule metadata only. Per-run `(once)` state,
//! runtime bytes, completed-step counts, and execution budgets are created from
//! an [`input::AdmittedRun`](crate::input::AdmittedRun) each time execution starts. This keeps parsed source
//! reuse separate from mutable runtime progress.

/// Parser limit value types and defaults.
pub(crate) mod limits;
/// Run result and output byte domains.
mod result;
/// Parsed rule table storage.
mod rule_set;

use crate::error::ParseError;
use crate::inspect::{OnceRuleCount, RuleCount, RuleView};
use crate::parser::parse_rules_impl;
use crate::policy::ParsePolicy;
use crate::source::ProgramSource;

pub(crate) use rule_set::{ActiveRuleCursor, RuleCursorAfterMiss, RuleScan};
pub(crate) use rule_set::{RuleSet, RuleSetBuilder};

pub use result::{ReturnOutput, ReturnOutputView, RunOutcome, RunResult, RuntimeStateSnapshot};
