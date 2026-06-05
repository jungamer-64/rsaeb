//! Compile-time resource policy types.
//!
//! Hosts choose parser, input, execution, rule-attempt, and trace snapshot
//! budgets by selecting policy types. Each policy domain has its own default
//! type, so a parser default cannot accidentally satisfy an execution or input
//! boundary. Runtime entrypoints do not accept policy value bags; the selected
//! type is the resource contract.
//!
//! Deleted runtime policy bags cannot be constructed:
//!
//! ```compile_fail
//! use rsaeb::limits::{ExecutionLimits, RuntimeInputLimits};
//! ```
//!
//! Policy domains are intentionally separate. The parser default cannot be used
//! to validate runtime input:
//!
//! ```compile_fail
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::DefaultParsePolicy;
//!
//! type ParseOnly = DefaultParsePolicy;
//!
//! let _input = RuntimeInput::validate::<ParseOnly>(RuntimeInputSource::from_bytes(b"a"));
//! ```
//!
//! The execution default cannot be used to validate runtime input:
//!
//! ```compile_fail
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::DefaultExecutionPolicy;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let _ = RuntimeInput::validate::<DefaultExecutionPolicy>(RuntimeInputSource::from_bytes(b"a"))?;
//!     Ok(())
//! }
//! ```
//!
//! The execution default cannot be used to parse programs:
//!
//! ```compile_fail
//! use rsaeb::policy::DefaultExecutionPolicy;
//! use rsaeb::program::ExecutableProgram;
//!
//! type ExecutionOnly = DefaultExecutionPolicy;
//!
//! let _program = ExecutableProgram::parse_text::<ExecutionOnly>("a=b");
//! ```
//!
//! Public boundary types no longer infer default policy domains. The policy
//! type must be named at construction:
//!
//! ```compile_fail
//! use rsaeb::program::ExecutableProgram;
//!
//! let _program: ExecutableProgram = ExecutableProgram::parse_text("a=b").unwrap();
//! ```
//!
//! ```compile_fail
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//!
//! let _input: RuntimeInput = RuntimeInput::validate(RuntimeInputSource::from_bytes(b"a")).unwrap();
//! ```
//!
//! Runtime witness values for selecting trace or rule-attempt policy cannot be
//! constructed:
//!
//! ```compile_fail
//! use rsaeb::policy::{DefaultTraceSnapshotPolicy, TraceSnapshotPolicyWitness};
//!
//! let _witness = TraceSnapshotPolicyWitness::<DefaultTraceSnapshotPolicy>::new();
//! ```
//!
//! Executable programs cannot use the empty-program stabilization boundary:
//!
//! ```compile_fail
//! use rsaeb::input::{RuntimeInput, RuntimeInputSource};
//! use rsaeb::policy::{DefaultExecutionPolicy, DefaultParsePolicy, DefaultRuntimeInputPolicy};
//! use rsaeb::program::ExecutableProgram;
//!
//! let program = ExecutableProgram::parse_text::<DefaultParsePolicy>("a=b").unwrap();
//! let input = RuntimeInput::validate::<DefaultRuntimeInputPolicy>(RuntimeInputSource::from_bytes(b"a")).unwrap();
//! let admitted = input.admit::<DefaultExecutionPolicy>().unwrap();
//! let _result = program.stabilize(admitted).unwrap();
//! ```

use crate::limits::{
    CodeLineByteLimit, PayloadByteLimit, ReturnByteLimit, RuleAttemptLimit, RuleLimit,
    RuntimeInputByteLimit, RuntimeStateByteLimit, SourceByteLimit, StepLimit,
    TraceSnapshotByteLimit,
};

/// Shared byte budget used by the default policy.
const DEFAULT_BYTE_BUDGET: usize = 16_777_216;
/// Shared count budget used by the default policy.
const DEFAULT_COUNT_BUDGET: usize = 1_000_000;

/// Parser resource policy selected at the type level.
pub trait ParsePolicy {
    /// Maximum source bytes accepted before line parsing starts.
    const SOURCE_BYTE_LIMIT: SourceByteLimit;
    /// Maximum executable bytes accepted in one source line.
    const CODE_LINE_BYTE_LIMIT: CodeLineByteLimit;
    /// Maximum bytes accepted in one parsed payload.
    const PAYLOAD_BYTE_LIMIT: PayloadByteLimit;
    /// Maximum executable rules accepted in one parsed program.
    const RULE_LIMIT: RuleLimit;
}

/// Runtime-input validation policy selected at the type level.
pub trait RuntimeInputPolicy {
    /// Maximum raw runtime-input bytes accepted before owned classification.
    const INPUT_BYTE_LIMIT: RuntimeInputByteLimit;
}

/// Execution resource policy selected at the type level.
pub trait ExecutionPolicy {
    /// Maximum committed execution steps.
    const STEP_LIMIT: StepLimit;
    /// Maximum runtime-state bytes for initial and rewritten states.
    const STATE_BYTE_LIMIT: RuntimeStateByteLimit;
    /// Maximum materialized `(return)` output bytes.
    const RETURN_BYTE_LIMIT: ReturnByteLimit;
}

/// Rule-attempt resource policy selected at the type level.
pub trait RuleAttemptPolicy {
    /// Maximum consumed executable rule-line attempts.
    const RULE_ATTEMPT_LIMIT: RuleAttemptLimit;
}

/// Trace snapshot resource policy selected at the type level.
pub trait TraceSnapshotPolicy {
    /// Maximum materialized bytes in one trace snapshot event.
    const TRACE_SNAPSHOT_BYTE_LIMIT: TraceSnapshotByteLimit;
}

/// Crate default parser resource policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultParsePolicy;

/// Crate default runtime-input validation policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultRuntimeInputPolicy;

/// Crate default execution resource policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultExecutionPolicy;

/// Crate default rule-attempt resource policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultRuleAttemptPolicy;

/// Crate default trace snapshot resource policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultTraceSnapshotPolicy;

/// Const-generic parser policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticParsePolicy<
    const SOURCE_BYTES: usize,
    const CODE_LINE_BYTES: usize,
    const PAYLOAD_BYTES: usize,
    const RULES: usize,
>;

/// Const-generic runtime-input policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticRuntimeInputPolicy<const INPUT_BYTES: usize>;

/// Const-generic execution policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticExecutionPolicy<
    const STEPS: usize,
    const STATE_BYTES: usize,
    const RETURN_BYTES: usize,
>;

/// Const-generic rule-attempt policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticRuleAttemptPolicy<const ATTEMPTS: usize>;

/// Const-generic trace snapshot policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticTraceSnapshotPolicy<const SNAPSHOT_BYTES: usize>;

impl ParsePolicy for DefaultParsePolicy {
    const SOURCE_BYTE_LIMIT: SourceByteLimit = SourceByteLimit::new(DEFAULT_BYTE_BUDGET);
    const CODE_LINE_BYTE_LIMIT: CodeLineByteLimit = CodeLineByteLimit::new(DEFAULT_BYTE_BUDGET);
    const PAYLOAD_BYTE_LIMIT: PayloadByteLimit = PayloadByteLimit::new(DEFAULT_BYTE_BUDGET);
    const RULE_LIMIT: RuleLimit = RuleLimit::new(DEFAULT_COUNT_BUDGET);
}

impl RuntimeInputPolicy for DefaultRuntimeInputPolicy {
    const INPUT_BYTE_LIMIT: RuntimeInputByteLimit = RuntimeInputByteLimit::new(DEFAULT_BYTE_BUDGET);
}

impl ExecutionPolicy for DefaultExecutionPolicy {
    const STEP_LIMIT: StepLimit = StepLimit::new(DEFAULT_COUNT_BUDGET);
    const STATE_BYTE_LIMIT: RuntimeStateByteLimit = RuntimeStateByteLimit::new(DEFAULT_BYTE_BUDGET);
    const RETURN_BYTE_LIMIT: ReturnByteLimit = ReturnByteLimit::new(DEFAULT_BYTE_BUDGET);
}

impl RuleAttemptPolicy for DefaultRuleAttemptPolicy {
    const RULE_ATTEMPT_LIMIT: RuleAttemptLimit = RuleAttemptLimit::new(DEFAULT_COUNT_BUDGET);
}

impl TraceSnapshotPolicy for DefaultTraceSnapshotPolicy {
    const TRACE_SNAPSHOT_BYTE_LIMIT: TraceSnapshotByteLimit =
        TraceSnapshotByteLimit::new(DEFAULT_BYTE_BUDGET);
}

impl<
    const SOURCE_BYTES: usize,
    const CODE_LINE_BYTES: usize,
    const PAYLOAD_BYTES: usize,
    const RULES: usize,
> ParsePolicy for StaticParsePolicy<SOURCE_BYTES, CODE_LINE_BYTES, PAYLOAD_BYTES, RULES>
{
    const SOURCE_BYTE_LIMIT: SourceByteLimit = SourceByteLimit::new(SOURCE_BYTES);
    const CODE_LINE_BYTE_LIMIT: CodeLineByteLimit = CodeLineByteLimit::new(CODE_LINE_BYTES);
    const PAYLOAD_BYTE_LIMIT: PayloadByteLimit = PayloadByteLimit::new(PAYLOAD_BYTES);
    const RULE_LIMIT: RuleLimit = RuleLimit::new(RULES);
}

impl<const INPUT_BYTES: usize> RuntimeInputPolicy for StaticRuntimeInputPolicy<INPUT_BYTES> {
    const INPUT_BYTE_LIMIT: RuntimeInputByteLimit = RuntimeInputByteLimit::new(INPUT_BYTES);
}

impl<const STEPS: usize, const STATE_BYTES: usize, const RETURN_BYTES: usize> ExecutionPolicy
    for StaticExecutionPolicy<STEPS, STATE_BYTES, RETURN_BYTES>
{
    const STEP_LIMIT: StepLimit = StepLimit::new(STEPS);
    const STATE_BYTE_LIMIT: RuntimeStateByteLimit = RuntimeStateByteLimit::new(STATE_BYTES);
    const RETURN_BYTE_LIMIT: ReturnByteLimit = ReturnByteLimit::new(RETURN_BYTES);
}

impl<const ATTEMPTS: usize> RuleAttemptPolicy for StaticRuleAttemptPolicy<ATTEMPTS> {
    const RULE_ATTEMPT_LIMIT: RuleAttemptLimit = RuleAttemptLimit::new(ATTEMPTS);
}

impl<const SNAPSHOT_BYTES: usize> TraceSnapshotPolicy
    for StaticTraceSnapshotPolicy<SNAPSHOT_BYTES>
{
    const TRACE_SNAPSHOT_BYTE_LIMIT: TraceSnapshotByteLimit =
        TraceSnapshotByteLimit::new(SNAPSHOT_BYTES);
}
