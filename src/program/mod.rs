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
//!
//! # Compile-time guards
//!
//! The shape-erased public `Program` type has been deleted:
//!
//! ```compile_fail
//! use rsaeb::program::Program;
//!
//! fn main() {}
//! ```
//!
//! The shape-erased public `ParsedProgram` classifier has been deleted:
//!
//! ```compile_fail
//! use rsaeb::program::ParsedProgram;
//!
//! fn main() {}
//! ```
//!
//! The old source-marker parse method has been deleted:
//!
//! ```compile_fail
//! use rsaeb::policy::DefaultParsePolicy;
//! use rsaeb::program::ExecutableProgram;
//!
//! fn main() {
//!     let _ = ExecutableProgram::<DefaultParsePolicy>::parse(b"a=b");
//! }
//! ```
//!
//! Target-shape parse entrypoints still require an explicit parse policy:
//!
//! ```compile_fail
//! use rsaeb::program::ExecutableProgram;
//!
//! fn main() {
//!     let _ = ExecutableProgram::parse_text("a=b");
//! }
//! ```
//!
//! Construction policies no longer parameterize stored program values:
//!
//! ```compile_fail
//! use rsaeb::policy::DefaultParsePolicy;
//! use rsaeb::program::{EmptyProgram, ExecutableProgram};
//!
//! fn old_shapes(
//!     _executable: ExecutableProgram<DefaultParsePolicy>,
//!     _empty: EmptyProgram<DefaultParsePolicy>,
//! ) {}
//! ```
//!
//! The old executable-program reference wrapper has been deleted:
//!
//! ```compile_fail
//! use rsaeb::program::ExecutableProgramRef;
//!
//! fn main() {}
//! ```
//!
//! Old executable and empty wrapper types have been deleted:
//!
//! ```compile_fail
//! use rsaeb::program::{
//!     BorrowedEmptyProgram, BorrowedExecutableProgram, OwnedEmptyProgram,
//!     OwnedExecutableProgram,
//! };
//!
//! fn main() {}
//! ```
//!
//! Empty programs do not expose zero-valued executable-rule counts. Empty
//! topology is represented by [`EmptyProgram`] itself:
//!
//! ```compile_fail
//! use rsaeb::policy::DefaultParsePolicy;
//! use rsaeb::program::EmptyProgram;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let empty = EmptyProgram::parse_text::<DefaultParsePolicy>("# empty")?;
//!     let _ = empty.rule_count();
//!     Ok(())
//! }
//! ```
//!
//! ```compile_fail
//! use rsaeb::policy::DefaultParsePolicy;
//! use rsaeb::program::EmptyProgram;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let empty = EmptyProgram::parse_text::<DefaultParsePolicy>("# empty")?;
//!     let _ = empty.rules();
//!     Ok(())
//! }
//! ```
//!
//! ```compile_fail
//! use rsaeb::policy::DefaultParsePolicy;
//! use rsaeb::program::EmptyProgram;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let empty = EmptyProgram::parse_text::<DefaultParsePolicy>("# empty")?;
//!     let _ = empty.once_rule_count();
//!     Ok(())
//! }
//! ```
//!
//! Executable programs expose a non-zero executable-rule count, not the old
//! zero-capable [`crate::inspect::RuleCount`]:
//!
//! ```compile_fail
//! use rsaeb::inspect::RuleCount;
//! use rsaeb::program::ExecutableProgram;
//!
//! fn invalid(program: &ExecutableProgram) -> RuleCount {
//!     program.rule_count()
//! }
//! ```
//!
//! Empty-program witnesses expose stabilization only; they cannot start
//! execution, tracing, stepwise execution, or rule-attempt execution:
//!
//! ```compile_fail
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{DefaultExecutionPolicy, DefaultParsePolicy, DefaultRuntimeInputPolicy};
//! use rsaeb::program::EmptyProgram;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let empty = EmptyProgram::parse_text::<DefaultParsePolicy>("# empty")?;
//!     let input = RuntimeInput::validate::<DefaultRuntimeInputPolicy>(RuntimeInputSource::from_bytes(b"a"))?;
//!     let admitted = input.admit::<DefaultExecutionPolicy>()?;
//!     let _ = empty.execute(admitted)?;
//!     Ok(())
//! }
//! ```

/// Target-shape parsed program types.
mod executable;
/// Parser limit value types and defaults.
pub(crate) mod limits;
/// Run result and output byte domains.
mod result;
/// Parsed rule table storage.
mod rule_set;

pub(crate) use rule_set::{
    EmptyRuleSetBuilder, ExecutableRuleSet, ExecutableRuleSetBuilder, RuleScan, RuleSink,
    RuntimeStoredRule, StoredRuleRef,
};

pub use executable::{EmptyProgram, ExecutableProgram};
pub use result::{ReturnOutput, ReturnOutputView, RunOutcome, RunResult, RuntimeStateSnapshot};
