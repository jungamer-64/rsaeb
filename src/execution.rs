//! Public stepwise run typestates.
//!
//! [`Program::start_run`](crate::program::Program::start_run) borrows a parsed
//! program into a [`BorrowedRunSession`]. [`Program::into_run`](crate::program::Program::into_run)
//! is the explicit owned variant for hosts that need a `'static` session.
//! [`Program::run`](crate::program::Program::run) is the borrowed
//! run-to-completion shortcut over the same admitted [`RunSeed`] boundary.
//! [`Program::start_rule_attempt_run`](crate::program::Program::start_rule_attempt_run)
//! and [`Program::into_rule_attempt_run`](crate::program::Program::into_rule_attempt_run)
//! use a separate rule-attempt typestate that can pause after non-matching
//! executable rule lines.
//!
//! A step transition is a typestate value, not a status flag. Applied steps
//! carry the continuation session. Stable and returned states are terminal.
//! Failed states are also terminal for the borrowed API: they preserve the
//! uncommitted state for diagnostics and then let the caller discard the run
//! into its [`RunError`]. Owned failed states additionally let the caller
//! recover the owned parsed program or split it from the error.
//! Rule-attempt transitions additionally expose typed miss reasons through
//! [`RuleMissReason`], expose stable reasons through
//! [`RuleAttemptStableReason`], and consume [`RuleAttemptLimit`] instead of
//! treating non-matches as rewrite steps.
//!
//! ```
//! use rsaeb::error::{LimitError, RunError};
//! use rsaeb::execution::BorrowedStepTransition;
//! use rsaeb::input::{RunSeed, RuntimeInput, RuntimeInputSource};
//! use rsaeb::limits::{
//!     DEFAULT_MAX_INPUT_LEN, DEFAULT_MAX_RETURN_LEN, DEFAULT_PARSE_LIMITS, ExecutionLimits,
//!     RuntimeInputLimits, RuntimeStateByteLimit, StepLimit,
//! };
//! use rsaeb::program::Program;
//! use rsaeb::source::ProgramSource;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::parse(ProgramSource::from_text("a=aaaa"), DEFAULT_PARSE_LIMITS)?;
//! let input_limits = RuntimeInputLimits::new(DEFAULT_MAX_INPUT_LEN);
//! let execution_limits = ExecutionLimits::new(
//!     StepLimit::new(10),
//!     RuntimeStateByteLimit::new(1),
//!     DEFAULT_MAX_RETURN_LEN,
//! );
//! let input = RuntimeInput::validate(RuntimeInputSource::from_bytes(b"a"), input_limits)?;
//! let session = program.start_run(RunSeed::admit(input, execution_limits)?)?;
//!
//! let BorrowedStepTransition::Failed(failed) = session.step() else {
//!     return Err("expected oversized rewrite to fail before commit".into());
//! };
//!
//! if failed.completed_steps().get() != 0 {
//!     return Err("failed step must not commit progress".into());
//! }
//! if failed.state().materialize()?.as_slice() != b"a" {
//!     return Err("failed step must expose the uncommitted state".into());
//! }
//! if !matches!(
//!     failed.error(),
//!     RunError::Limit(LimitError::State { attempted_len, .. })
//!         if attempted_len.get() == 4
//! ) {
//!     return Err("unexpected failed-step error".into());
//! }
//! # Ok(())
//! # }
//! ```

/// Shared mutable execution engine behind the public typestates.
mod engine;

use crate::error::{RunError, TracedRunError};
use crate::input::RunSeed;
use crate::inspect::RulePosition;
use crate::limits::{RuleAttemptCount, RuleAttemptLimit, StepCount};
use crate::program::{Program, ReturnOutput, RunResult};
use crate::runtime::action::{AppliedRule, CommittedReturnRule};
use crate::trace::{BorrowedTraceEvent, RuntimeStateView};

use engine::{
    AttemptSession, BorrowedProgram, CoreRuleAttempt, CoreStep, OwnedProgram, ProgramOwner,
    RunCore, Session,
};

pub use crate::runtime::matcher::RuleMissReason;

/// Stateful run session that borrows a reusable parsed program.
///
/// This is the stepwise form returned by
/// [`Program::start_run`](crate::program::Program::start_run). It consumes
/// itself on every step so callers must handle the returned [`BorrowedStepTransition`]
/// before they can continue.
pub struct BorrowedRunSession<'program> {
    /// Internal session using the public borrowed program boundary.
    session: Session<BorrowedProgram<'program>>,
}

/// Stateful run session that owns its parsed program.
///
/// This is the stepwise form returned by
/// [`Program::into_run`](crate::program::Program::into_run). It is useful when
/// the session must move independently of a borrowed [`Program`]. Owned
/// terminal and failed states retain a way to recover the parsed program
/// instead of leaking ownership through a parallel API.
pub struct OwnedRunSession {
    /// Internal session using the public owned program boundary.
    session: Session<OwnedProgram>,
}

/// Stateful run session that borrows a reusable parsed program and advances by rule attempt.
///
/// A rule-attempt step consumes one executable rule line even when that rule
/// does not apply. Ordinary rewrite steps still reset the rule cursor to the
/// first executable rule.
pub struct BorrowedRuleAttemptSession<'program> {
    /// Internal rule-attempt session using the public borrowed program boundary.
    session: AttemptSession<BorrowedProgram<'program>>,
}

/// Stateful run session that owns its parsed program and advances by rule attempt.
///
/// This is the owned counterpart to [`BorrowedRuleAttemptSession`].
pub struct OwnedRuleAttemptSession {
    /// Internal rule-attempt session using the public owned program boundary.
    session: AttemptSession<OwnedProgram>,
}

/// Result of advancing a borrowed run session once.
///
/// Only [`BorrowedStepTransition::Applied`] carries a continuation session. Stable,
/// returned, and failed transitions are terminal.
pub enum BorrowedStepTransition<'program> {
    /// One ordinary rewrite rule was applied and execution can continue.
    Applied(BorrowedAppliedStep<'program>),
    /// No rule matched the final runtime state.
    Stable(BorrowedStableRun<'program>),
    /// A matched rule executed `(return)`.
    Returned(BorrowedReturnedRun<'program>),
    /// A matching rule failed before committing.
    Failed(BorrowedFailedRun<'program>),
}

/// Completed non-applying rule attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleMiss {
    /// Program-local position of the consumed rule line.
    rule_position: RulePosition,
    /// Why the consumed rule did not apply.
    reason: RuleMissReason,
}

/// Why a rule-attempt run reached stability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleAttemptStableReason {
    /// The parsed program contains no executable rules.
    NoExecutableRules,
    /// The final executable rule line was consumed without applying.
    FinalMiss(RuleMiss),
}

/// One committed non-terminal rule application in a borrowed session.
pub struct BorrowedAppliedStep<'program> {
    /// Step number committed by this transition.
    step: StepCount,
    /// Program-local rewrite rule position committed by this transition.
    rule_position: RulePosition,
    /// Continuation session after the committed rewrite.
    session: BorrowedRunSession<'program>,
}

/// Terminal borrowed run state reached by no matching rule.
pub struct BorrowedStableRun<'program> {
    /// Number of committed steps before no rule matched.
    steps: StepCount,
    /// Parsed program borrowed by the terminal state.
    program: &'program Program,
    /// Terminal runtime core containing the stable state.
    core: RunCore,
}

/// Terminal borrowed run state reached by `(return)`.
pub struct BorrowedReturnedRun<'program> {
    /// Step number that executed the return action.
    step: StepCount,
    /// Program-local return rule position.
    rule_position: RulePosition,
    /// Parsed program borrowed by the terminal state.
    program: &'program Program,
    /// Materialized return output produced by the committed return rule.
    output: ReturnOutput,
}

/// Runtime failure that preserves uncommitted borrowed state for inspection.
pub struct BorrowedFailedRun<'program> {
    /// Runtime error that stopped the candidate step before commit.
    error: RunError,
    /// Uncommitted borrowed session retained for diagnostic inspection.
    session: BorrowedRunSession<'program>,
}

/// Result of advancing a borrowed rule-attempt session once.
///
/// Only [`BorrowedRuleAttemptTransition::Missed`] and [`BorrowedRuleAttemptTransition::Applied`]
/// carry continuation sessions. Stable, returned, and failed transitions are
/// terminal.
pub enum BorrowedRuleAttemptTransition<'program> {
    /// One executable rule line was consumed without applying.
    Missed(BorrowedMissedRuleAttempt<'program>),
    /// One ordinary rewrite rule was applied and execution can continue.
    Applied(BorrowedRuleAttemptAppliedStep<'program>),
    /// The rule pass completed without a match.
    Stable(BorrowedRuleAttemptStableRun<'program>),
    /// A matched rule executed `(return)`.
    Returned(BorrowedRuleAttemptReturnedRun<'program>),
    /// A matching rule failed before committing runtime state.
    Failed(BorrowedRuleAttemptFailedRun<'program>),
}

/// One consumed non-applying rule line in a borrowed rule-attempt session.
pub struct BorrowedMissedRuleAttempt<'program> {
    /// Rule-attempt count committed by this transition.
    attempt: RuleAttemptCount,
    /// Non-applying rule information.
    miss: RuleMiss,
    /// Continuation session after consuming the rule line.
    session: BorrowedRuleAttemptSession<'program>,
}

/// One committed non-terminal rule application in a borrowed rule-attempt session.
pub struct BorrowedRuleAttemptAppliedStep<'program> {
    /// Rule-attempt count committed by this transition.
    attempt: RuleAttemptCount,
    /// Step number committed by this transition.
    step: StepCount,
    /// Program-local rewrite rule position committed by this transition.
    rule_position: RulePosition,
    /// Continuation session after the committed rewrite.
    session: BorrowedRuleAttemptSession<'program>,
}

/// Terminal borrowed rule-attempt run state reached by no matching rule.
pub struct BorrowedRuleAttemptStableRun<'program> {
    /// Number of consumed rule attempts before stability.
    attempts: RuleAttemptCount,
    /// Number of committed rewrite steps before stability.
    steps: StepCount,
    /// Why the rule-attempt run reached stability.
    stable_reason: RuleAttemptStableReason,
    /// Parsed program borrowed by the terminal state.
    program: &'program Program,
    /// Terminal runtime core containing the stable state.
    core: RunCore,
}

/// Terminal borrowed rule-attempt run state reached by `(return)`.
pub struct BorrowedRuleAttemptReturnedRun<'program> {
    /// Rule-attempt count committed by this transition.
    attempt: RuleAttemptCount,
    /// Step number that executed the return action.
    step: StepCount,
    /// Program-local return rule position.
    rule_position: RulePosition,
    /// Parsed program borrowed by the terminal state.
    program: &'program Program,
    /// Materialized return output produced by the committed return rule.
    output: ReturnOutput,
}

/// Runtime failure that preserves uncommitted borrowed rule-attempt state for inspection.
pub struct BorrowedRuleAttemptFailedRun<'program> {
    /// Runtime error that stopped the candidate attempt before commit.
    error: RunError,
    /// Uncommitted borrowed session retained for diagnostic inspection.
    session: BorrowedRuleAttemptSession<'program>,
}

/// Result of advancing an owned run session once.
///
/// This mirrors [`BorrowedStepTransition`] while preserving ownership of the parsed
/// program through owned terminal and failed states.
pub enum OwnedStepTransition {
    /// One ordinary rewrite rule was applied and execution can continue.
    Applied(OwnedAppliedStep),
    /// No rule matched the final runtime state.
    Stable(OwnedStableRun),
    /// A matched rule executed `(return)`.
    Returned(OwnedReturnedRun),
    /// A matching rule failed before committing.
    Failed(OwnedFailedRun),
}

/// One committed non-terminal rule application.
pub struct OwnedAppliedStep {
    /// Step number committed by this transition.
    step: StepCount,
    /// Program-local rewrite rule position committed by this transition.
    rule_position: RulePosition,
    /// Continuation session after the committed rewrite.
    session: OwnedRunSession,
}

/// Terminal run state reached by no matching rule.
pub struct OwnedStableRun {
    /// Number of committed steps before no rule matched.
    steps: StepCount,
    /// Parsed program retained by the owned terminal state.
    program: Program,
    /// Terminal runtime core containing the stable state.
    core: RunCore,
}

/// Terminal run state reached by `(return)`.
pub struct OwnedReturnedRun {
    /// Step number that executed the return action.
    step: StepCount,
    /// Program-local return rule position.
    rule_position: RulePosition,
    /// Parsed program retained by the terminal state.
    program: Program,
    /// Materialized return output produced by the committed return rule.
    output: ReturnOutput,
}

/// Runtime failure that preserves uncommitted state for inspection.
pub struct OwnedFailedRun {
    /// Runtime error that stopped the candidate step before commit.
    error: RunError,
    /// Uncommitted owned session retained for diagnostic inspection.
    session: OwnedRunSession,
}

/// Result of advancing an owned rule-attempt session once.
///
/// This mirrors [`BorrowedRuleAttemptTransition`] while preserving ownership of the
/// parsed program through owned terminal and failed states.
pub enum OwnedRuleAttemptTransition {
    /// One executable rule line was consumed without applying.
    Missed(OwnedMissedRuleAttempt),
    /// One ordinary rewrite rule was applied and execution can continue.
    Applied(OwnedRuleAttemptAppliedStep),
    /// The rule pass completed without a match.
    Stable(OwnedRuleAttemptStableRun),
    /// A matched rule executed `(return)`.
    Returned(OwnedRuleAttemptReturnedRun),
    /// A matching rule failed before committing runtime state.
    Failed(OwnedRuleAttemptFailedRun),
}

/// One consumed non-applying rule line.
pub struct OwnedMissedRuleAttempt {
    /// Rule-attempt count committed by this transition.
    attempt: RuleAttemptCount,
    /// Non-applying rule information.
    miss: RuleMiss,
    /// Continuation session after consuming the rule line.
    session: OwnedRuleAttemptSession,
}

/// One committed non-terminal rule application.
pub struct OwnedRuleAttemptAppliedStep {
    /// Rule-attempt count committed by this transition.
    attempt: RuleAttemptCount,
    /// Step number committed by this transition.
    step: StepCount,
    /// Program-local rewrite rule position committed by this transition.
    rule_position: RulePosition,
    /// Continuation session after the committed rewrite.
    session: OwnedRuleAttemptSession,
}

/// Terminal owned rule-attempt run state reached by no matching rule.
pub struct OwnedRuleAttemptStableRun {
    /// Number of consumed rule attempts before stability.
    attempts: RuleAttemptCount,
    /// Number of committed rewrite steps before stability.
    steps: StepCount,
    /// Why the rule-attempt run reached stability.
    stable_reason: RuleAttemptStableReason,
    /// Parsed program retained by the owned terminal state.
    program: Program,
    /// Terminal runtime core containing the stable state.
    core: RunCore,
}

/// Terminal owned rule-attempt run state reached by `(return)`.
pub struct OwnedRuleAttemptReturnedRun {
    /// Rule-attempt count committed by this transition.
    attempt: RuleAttemptCount,
    /// Step number that executed the return action.
    step: StepCount,
    /// Program-local return rule position.
    rule_position: RulePosition,
    /// Parsed program retained by the terminal state.
    program: Program,
    /// Materialized return output produced by the committed return rule.
    output: ReturnOutput,
}

/// Runtime failure that preserves uncommitted owned rule-attempt state for inspection.
pub struct OwnedRuleAttemptFailedRun {
    /// Runtime error that stopped the candidate attempt before commit.
    error: RunError,
    /// Uncommitted owned session retained for diagnostic inspection.
    session: OwnedRuleAttemptSession,
}

impl core::fmt::Debug for BorrowedRunSession<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedRunSession")
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for OwnedRunSession {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedRunSession")
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for BorrowedRuleAttemptSession<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedRuleAttemptSession")
            .field("completed_attempts", &self.completed_attempts())
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for OwnedRuleAttemptSession {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedRuleAttemptSession")
            .field("completed_attempts", &self.completed_attempts())
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for BorrowedStepTransition<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Applied(applied) => formatter.debug_tuple("Applied").field(applied).finish(),
            Self::Stable(stable) => formatter.debug_tuple("Stable").field(stable).finish(),
            Self::Returned(returned) => formatter.debug_tuple("Returned").field(returned).finish(),
            Self::Failed(failed) => formatter.debug_tuple("Failed").field(failed).finish(),
        }
    }
}

impl core::fmt::Debug for OwnedStepTransition {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Applied(applied) => formatter.debug_tuple("Applied").field(applied).finish(),
            Self::Stable(stable) => formatter.debug_tuple("Stable").field(stable).finish(),
            Self::Returned(returned) => formatter.debug_tuple("Returned").field(returned).finish(),
            Self::Failed(failed) => formatter.debug_tuple("Failed").field(failed).finish(),
        }
    }
}

impl core::fmt::Debug for BorrowedRuleAttemptTransition<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Missed(missed) => formatter.debug_tuple("Missed").field(missed).finish(),
            Self::Applied(applied) => formatter.debug_tuple("Applied").field(applied).finish(),
            Self::Stable(stable) => formatter.debug_tuple("Stable").field(stable).finish(),
            Self::Returned(returned) => formatter.debug_tuple("Returned").field(returned).finish(),
            Self::Failed(failed) => formatter.debug_tuple("Failed").field(failed).finish(),
        }
    }
}

impl core::fmt::Debug for OwnedRuleAttemptTransition {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Missed(missed) => formatter.debug_tuple("Missed").field(missed).finish(),
            Self::Applied(applied) => formatter.debug_tuple("Applied").field(applied).finish(),
            Self::Stable(stable) => formatter.debug_tuple("Stable").field(stable).finish(),
            Self::Returned(returned) => formatter.debug_tuple("Returned").field(returned).finish(),
            Self::Failed(failed) => formatter.debug_tuple("Failed").field(failed).finish(),
        }
    }
}

impl core::fmt::Debug for BorrowedAppliedStep<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedAppliedStep")
            .field("step", &self.step())
            .field("rule_position", &self.rule_position())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for OwnedAppliedStep {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedAppliedStep")
            .field("step", &self.step())
            .field("rule_position", &self.rule_position())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for BorrowedMissedRuleAttempt<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedMissedRuleAttempt")
            .field("attempt", &self.attempt())
            .field("miss", &self.miss())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for OwnedMissedRuleAttempt {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedMissedRuleAttempt")
            .field("attempt", &self.attempt())
            .field("miss", &self.miss())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for BorrowedStableRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedStableRun")
            .field("steps", &self.steps())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for OwnedStableRun {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedStableRun")
            .field("steps", &self.steps())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for BorrowedRuleAttemptAppliedStep<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedRuleAttemptAppliedStep")
            .field("attempt", &self.attempt())
            .field("step", &self.step())
            .field("rule_position", &self.rule_position())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for OwnedRuleAttemptAppliedStep {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedRuleAttemptAppliedStep")
            .field("attempt", &self.attempt())
            .field("step", &self.step())
            .field("rule_position", &self.rule_position())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for BorrowedRuleAttemptStableRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedRuleAttemptStableRun")
            .field("attempts", &self.attempts())
            .field("steps", &self.steps())
            .field("stable_reason", &self.stable_reason())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for OwnedRuleAttemptStableRun {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedRuleAttemptStableRun")
            .field("attempts", &self.attempts())
            .field("steps", &self.steps())
            .field("stable_reason", &self.stable_reason())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for BorrowedReturnedRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedReturnedRun")
            .field("step", &self.step())
            .field("rule_position", &self.rule_position())
            .field("output", &self.output())
            .finish()
    }
}

impl core::fmt::Debug for OwnedReturnedRun {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedReturnedRun")
            .field("step", &self.step())
            .field("rule_position", &self.rule_position())
            .field("output", &self.output())
            .finish()
    }
}

impl core::fmt::Debug for BorrowedRuleAttemptReturnedRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedRuleAttemptReturnedRun")
            .field("attempt", &self.attempt())
            .field("step", &self.step())
            .field("rule_position", &self.rule_position())
            .field("output", &self.output())
            .finish()
    }
}

impl core::fmt::Debug for OwnedRuleAttemptReturnedRun {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedRuleAttemptReturnedRun")
            .field("attempt", &self.attempt())
            .field("step", &self.step())
            .field("rule_position", &self.rule_position())
            .field("output", &self.output())
            .finish()
    }
}

impl core::fmt::Debug for BorrowedFailedRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedFailedRun")
            .field("error", &self.error())
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for OwnedFailedRun {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedFailedRun")
            .field("error", &self.error())
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for BorrowedRuleAttemptFailedRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BorrowedRuleAttemptFailedRun")
            .field("error", &self.error())
            .field("completed_attempts", &self.completed_attempts())
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl core::fmt::Debug for OwnedRuleAttemptFailedRun {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("OwnedRuleAttemptFailedRun")
            .field("error", &self.error())
            .field("completed_attempts", &self.completed_attempts())
            .field("completed_steps", &self.completed_steps())
            .field("state", &self.state())
            .finish()
    }
}

impl RuleMiss {
    /// Captures the rule and reason for one consumed non-applying rule line.
    const fn new(rule_position: RulePosition, reason: RuleMissReason) -> Self {
        Self {
            rule_position,
            reason,
        }
    }

    /// Program-local position of the consumed rule line.
    #[must_use]
    pub const fn rule_position(self) -> RulePosition {
        self.rule_position
    }

    /// Why the consumed rule did not apply.
    #[must_use]
    pub const fn reason(self) -> RuleMissReason {
        self.reason
    }
}

impl CommittedReturnRule {
    /// Materializes this returned run as a run result.
    fn into_result(self) -> RunResult {
        let step = self.step();
        RunResult::from_return(self.into_output(), step)
    }
}

/// Runs a borrowed program to completion.
///
/// # Errors
///
/// Returns `RunError` when execution setup fails or a later matching rule would
/// exceed configured limits.
pub(crate) fn finish_borrowed_run(program: &Program, seed: RunSeed) -> Result<RunResult, RunError> {
    Session::new(BorrowedProgram { program }, seed)?.finish()
}

/// Runs a borrowed program to completion while emitting borrowed trace events.
///
/// # Errors
///
/// Returns `TracedRunError::Run` for runtime failures and
/// `TracedRunError::Trace` for user callback failures.
pub(crate) fn run_with_borrowed_trace<'program, F, E>(
    program: &'program Program,
    seed: RunSeed,
    trace: F,
) -> Result<RunResult, TracedRunError<E>>
where
    F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), E>,
{
    Session::new(BorrowedProgram { program }, seed)
        .map_err(TracedRunError::Run)?
        .run_with_borrowed_trace(trace)
}

/// Shared transition construction for ordinary stepwise sessions.
trait StepwiseRunSession: Sized {
    /// Public transition produced by this session.
    type Transition;

    /// Mutable access to the private runtime session.
    fn session_mut(&mut self) -> &mut Session<Self::Owner>;

    /// Program ownership mode carried by this public session.
    type Owner: ProgramOwner;

    /// Builds a non-terminal applied transition.
    fn applied(self, step: StepCount, rule_position: RulePosition) -> Self::Transition;

    /// Builds a terminal return transition.
    fn returned(self, committed: CommittedReturnRule) -> Self::Transition;

    /// Builds a terminal stable transition.
    fn stable(self, steps: StepCount) -> Self::Transition;

    /// Builds a terminal failed transition.
    fn failed(self, error: RunError) -> Self::Transition;

    /// Advances by one matching rule and maps the core result into public typestates.
    fn step_transition(mut self) -> Self::Transition {
        match self.session_mut().step() {
            Ok(CoreStep::Applied(AppliedRule::Rewrite(committed))) => {
                self.applied(committed.step(), committed.rule_position())
            }
            Ok(CoreStep::Applied(AppliedRule::Return(committed))) => self.returned(committed),
            Ok(CoreStep::Stable(steps)) => self.stable(steps),
            Err(error) => self.failed(error),
        }
    }
}

/// Shared transition construction for rule-attempt stepwise sessions.
trait RuleAttemptRunSession: Sized {
    /// Public transition produced by this session.
    type Transition;

    /// Program ownership mode carried by this public session.
    type Owner: ProgramOwner;

    /// Mutable access to the private rule-attempt runtime session.
    fn session_mut(&mut self) -> &mut AttemptSession<Self::Owner>;

    /// Builds a non-terminal missed-attempt transition.
    fn missed(self, attempt: RuleAttemptCount, miss: RuleMiss) -> Self::Transition;

    /// Builds a non-terminal applied-attempt transition.
    fn applied(
        self,
        attempt: RuleAttemptCount,
        step: StepCount,
        rule_position: RulePosition,
    ) -> Self::Transition;

    /// Builds a terminal return transition.
    fn returned(
        self,
        attempt: RuleAttemptCount,
        committed: CommittedReturnRule,
    ) -> Self::Transition;

    /// Builds a terminal stable transition.
    fn stable(
        self,
        attempts: RuleAttemptCount,
        steps: StepCount,
        stable_reason: RuleAttemptStableReason,
    ) -> Self::Transition;

    /// Builds a terminal failed transition.
    fn failed(self, error: RunError) -> Self::Transition;

    /// Advances by one executable rule line and maps the core result into public typestates.
    fn step_transition(mut self) -> Self::Transition {
        match self.session_mut().step() {
            Ok(CoreRuleAttempt::Missed { attempt, miss }) => self.missed(attempt, miss),
            Ok(CoreRuleAttempt::Applied {
                attempt,
                applied: AppliedRule::Rewrite(committed),
            }) => self.applied(attempt, committed.step(), committed.rule_position()),
            Ok(CoreRuleAttempt::Applied {
                attempt,
                applied: AppliedRule::Return(committed),
            }) => self.returned(attempt, committed),
            Ok(CoreRuleAttempt::Stable {
                attempts,
                steps,
                stable_reason,
            }) => self.stable(attempts, steps, stable_reason),
            Err(error) => self.failed(error),
        }
    }
}

impl<'program> StepwiseRunSession for BorrowedRunSession<'program> {
    type Transition = BorrowedStepTransition<'program>;
    type Owner = BorrowedProgram<'program>;

    fn session_mut(&mut self) -> &mut Session<Self::Owner> {
        &mut self.session
    }

    fn applied(self, step: StepCount, rule_position: RulePosition) -> Self::Transition {
        BorrowedStepTransition::Applied(BorrowedAppliedStep {
            step,
            rule_position,
            session: self,
        })
    }

    fn returned(self, committed: CommittedReturnRule) -> Self::Transition {
        let step = committed.step();
        let rule_position = committed.rule_position();
        let output = committed.into_output();
        let Session { program, core: _ } = self.session;
        BorrowedStepTransition::Returned(BorrowedReturnedRun {
            step,
            rule_position,
            program: program.program,
            output,
        })
    }

    fn stable(self, steps: StepCount) -> Self::Transition {
        let Session { program, core } = self.session;
        BorrowedStepTransition::Stable(BorrowedStableRun {
            steps,
            program: program.program,
            core,
        })
    }

    fn failed(self, error: RunError) -> Self::Transition {
        BorrowedStepTransition::Failed(BorrowedFailedRun::new(error, self))
    }
}

impl StepwiseRunSession for OwnedRunSession {
    type Transition = OwnedStepTransition;
    type Owner = OwnedProgram;

    fn session_mut(&mut self) -> &mut Session<Self::Owner> {
        &mut self.session
    }

    fn applied(self, step: StepCount, rule_position: RulePosition) -> Self::Transition {
        OwnedStepTransition::Applied(OwnedAppliedStep {
            step,
            rule_position,
            session: self,
        })
    }

    fn returned(self, committed: CommittedReturnRule) -> Self::Transition {
        let step = committed.step();
        let rule_position = committed.rule_position();
        let output = committed.into_output();
        let (program, _core) = self.session.into_program_core();
        OwnedStepTransition::Returned(OwnedReturnedRun {
            step,
            rule_position,
            program,
            output,
        })
    }

    fn stable(self, steps: StepCount) -> Self::Transition {
        let (program, core) = self.session.into_program_core();
        OwnedStepTransition::Stable(OwnedStableRun {
            steps,
            program,
            core,
        })
    }

    fn failed(self, error: RunError) -> Self::Transition {
        OwnedStepTransition::Failed(OwnedFailedRun::new(error, self))
    }
}

impl<'program> RuleAttemptRunSession for BorrowedRuleAttemptSession<'program> {
    type Transition = BorrowedRuleAttemptTransition<'program>;
    type Owner = BorrowedProgram<'program>;

    fn session_mut(&mut self) -> &mut AttemptSession<Self::Owner> {
        &mut self.session
    }

    fn missed(self, attempt: RuleAttemptCount, miss: RuleMiss) -> Self::Transition {
        BorrowedRuleAttemptTransition::Missed(BorrowedMissedRuleAttempt {
            attempt,
            miss,
            session: self,
        })
    }

    fn applied(
        self,
        attempt: RuleAttemptCount,
        step: StepCount,
        rule_position: RulePosition,
    ) -> Self::Transition {
        BorrowedRuleAttemptTransition::Applied(BorrowedRuleAttemptAppliedStep {
            attempt,
            step,
            rule_position,
            session: self,
        })
    }

    fn returned(
        self,
        attempt: RuleAttemptCount,
        committed: CommittedReturnRule,
    ) -> Self::Transition {
        let step = committed.step();
        let rule_position = committed.rule_position();
        let output = committed.into_output();
        let AttemptSession {
            program,
            core: _,
            cursor: _,
            attempt_budget: _,
        } = self.session;
        BorrowedRuleAttemptTransition::Returned(BorrowedRuleAttemptReturnedRun {
            attempt,
            step,
            rule_position,
            program: program.program,
            output,
        })
    }

    fn stable(
        self,
        attempts: RuleAttemptCount,
        steps: StepCount,
        stable_reason: RuleAttemptStableReason,
    ) -> Self::Transition {
        let AttemptSession {
            program,
            core,
            cursor: _,
            attempt_budget: _,
        } = self.session;
        BorrowedRuleAttemptTransition::Stable(BorrowedRuleAttemptStableRun {
            attempts,
            steps,
            stable_reason,
            program: program.program,
            core,
        })
    }

    fn failed(self, error: RunError) -> Self::Transition {
        BorrowedRuleAttemptTransition::Failed(BorrowedRuleAttemptFailedRun::new(error, self))
    }
}

impl RuleAttemptRunSession for OwnedRuleAttemptSession {
    type Transition = OwnedRuleAttemptTransition;
    type Owner = OwnedProgram;

    fn session_mut(&mut self) -> &mut AttemptSession<Self::Owner> {
        &mut self.session
    }

    fn missed(self, attempt: RuleAttemptCount, miss: RuleMiss) -> Self::Transition {
        OwnedRuleAttemptTransition::Missed(OwnedMissedRuleAttempt {
            attempt,
            miss,
            session: self,
        })
    }

    fn applied(
        self,
        attempt: RuleAttemptCount,
        step: StepCount,
        rule_position: RulePosition,
    ) -> Self::Transition {
        OwnedRuleAttemptTransition::Applied(OwnedRuleAttemptAppliedStep {
            attempt,
            step,
            rule_position,
            session: self,
        })
    }

    fn returned(
        self,
        attempt: RuleAttemptCount,
        committed: CommittedReturnRule,
    ) -> Self::Transition {
        let step = committed.step();
        let rule_position = committed.rule_position();
        let output = committed.into_output();
        let (program, _core) = self.session.into_program_core();
        OwnedRuleAttemptTransition::Returned(OwnedRuleAttemptReturnedRun {
            attempt,
            step,
            rule_position,
            program,
            output,
        })
    }

    fn stable(
        self,
        attempts: RuleAttemptCount,
        steps: StepCount,
        stable_reason: RuleAttemptStableReason,
    ) -> Self::Transition {
        let (program, core) = self.session.into_program_core();
        OwnedRuleAttemptTransition::Stable(OwnedRuleAttemptStableRun {
            attempts,
            steps,
            stable_reason,
            program,
            core,
        })
    }

    fn failed(self, error: RunError) -> Self::Transition {
        OwnedRuleAttemptTransition::Failed(OwnedRuleAttemptFailedRun::new(error, self))
    }
}

impl<'program> BorrowedRunSession<'program> {
    /// Starts a new borrowed run session for a parsed program and admitted run
    /// seed.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if allocating per-run rule state fails.
    pub(crate) fn new(program: &'program Program, seed: RunSeed) -> Result<Self, RunError> {
        Ok(Self {
            session: Session::new(BorrowedProgram { program }, seed)?,
        })
    }

    /// Number of rewrite steps that have already completed in this run.
    ///
    /// Failed candidate steps are not counted because they never commit.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.session.completed_steps()
    }

    /// Borrow the parsed program used by this session.
    #[must_use]
    pub fn program(&self) -> &'program Program {
        self.session.program.program
    }

    /// Borrow the current runtime state.
    ///
    /// The returned view borrows only for this observation. Materializing it is
    /// an explicit allocation boundary.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Advances this run by exactly one matching rule when possible.
    ///
    /// Applying an ordinary rewrite returns [`BorrowedStepTransition::Applied`] with a
    /// continuation session. No match, `(return)`, and runtime failure all
    /// consume the session into terminal typestates.
    #[must_use]
    pub fn step(self) -> BorrowedStepTransition<'program> {
        self.step_transition()
    }

    /// Runs this session to completion.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when applying a later matching rule would exceed the
    /// configured limits, allocation fails, or state-size arithmetic overflows.
    pub fn finish(self) -> Result<RunResult, RunError> {
        self.session.finish()
    }
}

impl<'program> BorrowedRuleAttemptSession<'program> {
    /// Starts a new borrowed rule-attempt run session for a parsed program and admitted run seed.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if allocating per-run rule state fails.
    pub(crate) fn new(
        program: &'program Program,
        seed: RunSeed,
        limit: RuleAttemptLimit,
    ) -> Result<Self, RunError> {
        Ok(Self {
            session: AttemptSession::new(BorrowedProgram { program }, seed, limit)?,
        })
    }

    /// Number of rewrite steps that have already completed in this run.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.session.completed_steps()
    }

    /// Number of executable rule-line attempts consumed so far.
    #[must_use]
    pub const fn completed_attempts(&self) -> RuleAttemptCount {
        self.session.completed_attempts()
    }

    /// Borrow the parsed program used by this session.
    #[must_use]
    pub fn program(&self) -> &'program Program {
        self.session.program.program
    }

    /// Borrow the current runtime state.
    ///
    /// The returned view borrows only for this observation. Materializing it is
    /// an explicit allocation boundary.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Advances this run by exactly one executable rule line when possible.
    ///
    /// Non-matching rules return [`BorrowedRuleAttemptTransition::Missed`] with a
    /// continuation session. Matching rewrites return
    /// [`BorrowedRuleAttemptTransition::Applied`] and reset the next attempt to the
    /// first executable rule. No match across the whole pass, `(return)`, and
    /// runtime failure consume the session into terminal typestates.
    #[must_use]
    pub fn step(self) -> BorrowedRuleAttemptTransition<'program> {
        self.step_transition()
    }
}

impl OwnedRunSession {
    /// Starts a new owned run session for a parsed program and admitted run seed.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if allocating per-run rule state fails.
    pub(crate) fn new(program: Program, seed: RunSeed) -> Result<Self, RunError> {
        Ok(Self {
            session: Session::new(OwnedProgram { program }, seed)?,
        })
    }

    /// Number of rewrite steps that have already completed in this run.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.session.completed_steps()
    }

    /// Borrow the parsed program owned by this session.
    #[must_use]
    pub fn program(&self) -> &Program {
        self.session.program()
    }

    /// Discards the current run state and recovers the owned parsed program.
    ///
    /// This intentionally drops the in-progress runtime state; it is for
    /// ownership recovery, not for retrying the same admitted run.
    #[must_use]
    pub fn into_program(self) -> Program {
        let (program, _core) = self.session.into_program_core();
        program
    }

    /// Borrow the current runtime state.
    ///
    /// The returned view borrows only for this observation. Materializing it is
    /// an explicit allocation boundary.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Advances this run by exactly one matching rule when possible.
    ///
    /// Applying an ordinary rewrite returns [`OwnedStepTransition::Applied`]
    /// with a continuation session. Owned terminal and failed states keep the
    /// parsed program recoverable.
    #[must_use]
    pub fn step(self) -> OwnedStepTransition {
        self.step_transition()
    }

    /// Runs this session to completion.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when applying a later matching rule would exceed the
    /// configured limits, allocation fails, or state-size arithmetic overflows.
    pub fn finish(self) -> Result<RunResult, RunError> {
        self.session.finish()
    }
}

impl OwnedRuleAttemptSession {
    /// Starts a new owned rule-attempt run session for a parsed program and admitted run seed.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if allocating per-run rule state fails.
    pub(crate) fn new(
        program: Program,
        seed: RunSeed,
        limit: RuleAttemptLimit,
    ) -> Result<Self, RunError> {
        Ok(Self {
            session: AttemptSession::new(OwnedProgram { program }, seed, limit)?,
        })
    }

    /// Number of rewrite steps that have already completed in this run.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.session.completed_steps()
    }

    /// Number of executable rule-line attempts consumed so far.
    #[must_use]
    pub const fn completed_attempts(&self) -> RuleAttemptCount {
        self.session.completed_attempts()
    }

    /// Borrow the parsed program owned by this session.
    #[must_use]
    pub fn program(&self) -> &Program {
        self.session.program()
    }

    /// Discards the current run state and recovers the owned parsed program.
    ///
    /// This intentionally drops the in-progress runtime state; it is for
    /// ownership recovery, not for retrying the same admitted run.
    #[must_use]
    pub fn into_program(self) -> Program {
        let (program, _core) = self.session.into_program_core();
        program
    }

    /// Borrow the current runtime state.
    ///
    /// The returned view borrows only for this observation. Materializing it is
    /// an explicit allocation boundary.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Advances this run by exactly one executable rule line when possible.
    ///
    /// Non-matching rules return [`OwnedRuleAttemptTransition::Missed`] with a
    /// continuation session. Matching rewrites return
    /// [`OwnedRuleAttemptTransition::Applied`] and reset the next attempt to
    /// the first executable rule. Owned terminal and failed states keep the
    /// parsed program recoverable.
    #[must_use]
    pub fn step(self) -> OwnedRuleAttemptTransition {
        self.step_transition()
    }
}

impl<'program> BorrowedAppliedStep<'program> {
    /// One-based applied step count.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Program-local position of the applied rule.
    #[must_use]
    pub const fn rule_position(&self) -> RulePosition {
        self.rule_position
    }

    /// Runtime state after the applied rewrite step.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Continue running after observing this applied step.
    ///
    /// This is the only borrowed transition that can resume execution.
    #[must_use]
    pub fn into_session(self) -> BorrowedRunSession<'program> {
        self.session
    }
}

impl OwnedAppliedStep {
    /// One-based applied step count.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Program-local position of the applied rule.
    #[must_use]
    pub const fn rule_position(&self) -> RulePosition {
        self.rule_position
    }

    /// Runtime state after the applied rewrite step.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Continue running after observing this applied step.
    ///
    /// This is the only owned transition that can resume execution.
    #[must_use]
    pub fn into_session(self) -> OwnedRunSession {
        self.session
    }
}

impl<'program> BorrowedMissedRuleAttempt<'program> {
    /// One-based consumed rule-attempt count.
    #[must_use]
    pub const fn attempt(&self) -> RuleAttemptCount {
        self.attempt
    }

    /// Non-applying rule information.
    #[must_use]
    pub const fn miss(&self) -> RuleMiss {
        self.miss
    }

    /// Program-local position of the consumed non-applying rule.
    #[must_use]
    pub const fn rule_position(&self) -> RulePosition {
        self.miss.rule_position()
    }

    /// Why the consumed rule did not apply.
    #[must_use]
    pub const fn reason(&self) -> RuleMissReason {
        self.miss.reason()
    }

    /// Runtime state after the non-applying rule attempt.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Continue running after observing this missed rule attempt.
    #[must_use]
    pub fn into_session(self) -> BorrowedRuleAttemptSession<'program> {
        self.session
    }
}

impl OwnedMissedRuleAttempt {
    /// One-based consumed rule-attempt count.
    #[must_use]
    pub const fn attempt(&self) -> RuleAttemptCount {
        self.attempt
    }

    /// Non-applying rule information.
    #[must_use]
    pub const fn miss(&self) -> RuleMiss {
        self.miss
    }

    /// Program-local position of the consumed non-applying rule.
    #[must_use]
    pub const fn rule_position(&self) -> RulePosition {
        self.miss.rule_position()
    }

    /// Why the consumed rule did not apply.
    #[must_use]
    pub const fn reason(&self) -> RuleMissReason {
        self.miss.reason()
    }

    /// Runtime state after the non-applying rule attempt.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Continue running after observing this missed rule attempt.
    #[must_use]
    pub fn into_session(self) -> OwnedRuleAttemptSession {
        self.session
    }
}

impl<'program> BorrowedRuleAttemptAppliedStep<'program> {
    /// One-based consumed rule-attempt count.
    #[must_use]
    pub const fn attempt(&self) -> RuleAttemptCount {
        self.attempt
    }

    /// One-based applied step count.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Program-local position of the applied rule.
    #[must_use]
    pub const fn rule_position(&self) -> RulePosition {
        self.rule_position
    }

    /// Runtime state after the applied rewrite step.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Continue running after observing this applied rule attempt.
    #[must_use]
    pub fn into_session(self) -> BorrowedRuleAttemptSession<'program> {
        self.session
    }
}

impl OwnedRuleAttemptAppliedStep {
    /// One-based consumed rule-attempt count.
    #[must_use]
    pub const fn attempt(&self) -> RuleAttemptCount {
        self.attempt
    }

    /// One-based applied step count.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Program-local position of the applied rule.
    #[must_use]
    pub const fn rule_position(&self) -> RulePosition {
        self.rule_position
    }

    /// Runtime state after the applied rewrite step.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Continue running after observing this applied rule attempt.
    #[must_use]
    pub fn into_session(self) -> OwnedRuleAttemptSession {
        self.session
    }
}

impl<'program> BorrowedStableRun<'program> {
    /// Number of rewrite steps applied before reaching the stable state.
    #[must_use]
    pub const fn steps(&self) -> StepCount {
        self.steps
    }

    /// Borrow the parsed program used by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &'program Program {
        self.program
    }

    /// Borrowed final runtime state.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }

    /// Materializes this stable run as a run result.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if final state materialization cannot allocate.
    pub fn into_result(self) -> Result<RunResult, RunError> {
        self.core.into_stable_result(self.steps)
    }
}

impl OwnedStableRun {
    /// Number of rewrite steps applied before reaching the stable state.
    #[must_use]
    pub const fn steps(&self) -> StepCount {
        self.steps
    }

    /// Borrow the parsed program owned by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &Program {
        &self.program
    }

    /// Discards the terminal state and recovers the owned parsed program.
    ///
    /// This drops the stable runtime state. Use [`OwnedStableRun::into_result`]
    /// when the final state bytes are the desired output.
    #[must_use]
    pub fn into_program(self) -> Program {
        self.program
    }

    /// Borrowed final runtime state.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }

    /// Materializes this stable run as a run result.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if final state materialization cannot allocate.
    pub fn into_result(self) -> Result<RunResult, RunError> {
        self.core.into_stable_result(self.steps)
    }
}

impl<'program> BorrowedRuleAttemptStableRun<'program> {
    /// Number of rule attempts consumed before reaching the stable state.
    #[must_use]
    pub const fn attempts(&self) -> RuleAttemptCount {
        self.attempts
    }

    /// Number of rewrite steps applied before reaching the stable state.
    #[must_use]
    pub const fn steps(&self) -> StepCount {
        self.steps
    }

    /// Why this rule-attempt pass reached stability.
    #[must_use]
    pub const fn stable_reason(&self) -> RuleAttemptStableReason {
        self.stable_reason
    }

    /// Borrow the parsed program used by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &'program Program {
        self.program
    }

    /// Borrowed final runtime state.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }

    /// Materializes this stable run as a run result.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if final state materialization cannot allocate.
    pub fn into_result(self) -> Result<RunResult, RunError> {
        self.core.into_stable_result(self.steps)
    }
}

impl OwnedRuleAttemptStableRun {
    /// Number of rule attempts consumed before reaching the stable state.
    #[must_use]
    pub const fn attempts(&self) -> RuleAttemptCount {
        self.attempts
    }

    /// Number of rewrite steps applied before reaching the stable state.
    #[must_use]
    pub const fn steps(&self) -> StepCount {
        self.steps
    }

    /// Why this rule-attempt pass reached stability.
    #[must_use]
    pub const fn stable_reason(&self) -> RuleAttemptStableReason {
        self.stable_reason
    }

    /// Borrow the parsed program owned by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &Program {
        &self.program
    }

    /// Discards the terminal state and recovers the owned parsed program.
    ///
    /// This drops the stable runtime state. Use
    /// [`OwnedRuleAttemptStableRun::into_result`] when the final state bytes
    /// are the desired output.
    #[must_use]
    pub fn into_program(self) -> Program {
        self.program
    }

    /// Borrowed final runtime state.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.core.state()
    }

    /// Materializes this stable run as a run result.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if final state materialization cannot allocate.
    pub fn into_result(self) -> Result<RunResult, RunError> {
        self.core.into_stable_result(self.steps)
    }
}

impl<'program> BorrowedReturnedRun<'program> {
    /// One-based applied step count for the return rule.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Borrow the parsed program used by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &'program Program {
        self.program
    }

    /// Program-local position of the return rule.
    #[must_use]
    pub const fn rule_position(&self) -> RulePosition {
        self.rule_position
    }

    /// Materialized return output from runtime execution.
    #[must_use]
    pub const fn output(&self) -> &ReturnOutput {
        &self.output
    }

    /// Materializes this returned run as a run result.
    #[must_use]
    pub fn into_result(self) -> RunResult {
        RunResult::from_return(self.output, self.step)
    }
}

impl OwnedReturnedRun {
    /// One-based applied step count for the return rule.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Borrow the parsed program owned by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &Program {
        &self.program
    }

    /// Discards the return output and recovers the owned parsed program.
    ///
    /// This drops the terminal `(return)` output. Use
    /// [`OwnedReturnedRun::into_result`] when the output bytes are the desired
    /// result.
    #[must_use]
    pub fn into_program(self) -> Program {
        self.program
    }

    /// Program-local position of the return rule.
    #[must_use]
    pub const fn rule_position(&self) -> RulePosition {
        self.rule_position
    }

    /// Materialized return output from runtime execution.
    #[must_use]
    pub const fn output(&self) -> &ReturnOutput {
        &self.output
    }

    /// Materializes this returned run as a run result.
    #[must_use]
    pub fn into_result(self) -> RunResult {
        RunResult::from_return(self.output, self.step)
    }
}

impl<'program> BorrowedRuleAttemptReturnedRun<'program> {
    /// One-based consumed rule-attempt count.
    #[must_use]
    pub const fn attempt(&self) -> RuleAttemptCount {
        self.attempt
    }

    /// One-based applied step count for the return rule.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Borrow the parsed program used by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &'program Program {
        self.program
    }

    /// Program-local position of the return rule.
    #[must_use]
    pub const fn rule_position(&self) -> RulePosition {
        self.rule_position
    }

    /// Materialized return output from runtime execution.
    #[must_use]
    pub const fn output(&self) -> &ReturnOutput {
        &self.output
    }

    /// Materializes this returned run as a run result.
    #[must_use]
    pub fn into_result(self) -> RunResult {
        RunResult::from_return(self.output, self.step)
    }
}

impl OwnedRuleAttemptReturnedRun {
    /// One-based consumed rule-attempt count.
    #[must_use]
    pub const fn attempt(&self) -> RuleAttemptCount {
        self.attempt
    }

    /// One-based applied step count for the return rule.
    #[must_use]
    pub const fn step(&self) -> StepCount {
        self.step
    }

    /// Borrow the parsed program owned by this terminal state.
    #[must_use]
    pub const fn program(&self) -> &Program {
        &self.program
    }

    /// Discards the return output and recovers the owned parsed program.
    ///
    /// This drops the terminal `(return)` output. Use
    /// [`OwnedRuleAttemptReturnedRun::into_result`] when the output bytes are
    /// the desired result.
    #[must_use]
    pub fn into_program(self) -> Program {
        self.program
    }

    /// Program-local position of the return rule.
    #[must_use]
    pub const fn rule_position(&self) -> RulePosition {
        self.rule_position
    }

    /// Materialized return output from runtime execution.
    #[must_use]
    pub const fn output(&self) -> &ReturnOutput {
        &self.output
    }

    /// Materializes this returned run as a run result.
    #[must_use]
    pub fn into_result(self) -> RunResult {
        RunResult::from_return(self.output, self.step)
    }
}

impl<'program> BorrowedFailedRun<'program> {
    /// Captures a failed borrowed session without committing the attempted step.
    fn new(error: RunError, session: BorrowedRunSession<'program>) -> Self {
        Self { error, session }
    }

    /// Runtime error that prevented the step from committing.
    #[must_use]
    pub const fn error(&self) -> &RunError {
        &self.error
    }

    /// Number of rewrite steps that completed before the failed step attempt.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.session.completed_steps()
    }

    /// Borrow the parsed program used by this failed session.
    #[must_use]
    pub fn program(&self) -> &'program Program {
        self.session.program()
    }

    /// Borrow the uncommitted runtime state preserved by this error.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Discard the uncommitted run session and return the runtime error.
    ///
    /// Borrowed failed runs are terminal; there is no retryable borrowed
    /// continuation after an uncommitted failure.
    #[must_use]
    pub fn into_error(self) -> RunError {
        self.error
    }
}

impl core::fmt::Display for BorrowedFailedRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl core::error::Error for BorrowedFailedRun<'_> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl<'program> BorrowedRuleAttemptFailedRun<'program> {
    /// Captures a failed borrowed rule-attempt session without committing runtime state.
    fn new(error: RunError, session: BorrowedRuleAttemptSession<'program>) -> Self {
        Self { error, session }
    }

    /// Runtime error that prevented the rule attempt from completing.
    #[must_use]
    pub const fn error(&self) -> &RunError {
        &self.error
    }

    /// Number of rule attempts consumed before the failure was reported.
    #[must_use]
    pub const fn completed_attempts(&self) -> RuleAttemptCount {
        self.session.completed_attempts()
    }

    /// Number of rewrite steps that completed before the failed rule attempt.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.session.completed_steps()
    }

    /// Borrow the parsed program used by this failed session.
    #[must_use]
    pub fn program(&self) -> &'program Program {
        self.session.program()
    }

    /// Borrow the uncommitted runtime state preserved by this error.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Discard the uncommitted run session and return the runtime error.
    #[must_use]
    pub fn into_error(self) -> RunError {
        self.error
    }
}

impl core::fmt::Display for BorrowedRuleAttemptFailedRun<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl core::error::Error for BorrowedRuleAttemptFailedRun<'_> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl OwnedFailedRun {
    /// Captures a failed owned session without committing the attempted step.
    fn new(error: RunError, session: OwnedRunSession) -> Self {
        Self { error, session }
    }

    /// Runtime error that prevented the step from committing.
    #[must_use]
    pub const fn error(&self) -> &RunError {
        &self.error
    }

    /// Number of rewrite steps that completed before the failed step attempt.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.session.completed_steps()
    }

    /// Borrow the parsed program owned by this failed session.
    #[must_use]
    pub fn program(&self) -> &Program {
        self.session.program()
    }

    /// Borrow the uncommitted runtime state preserved by this error.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Discard the uncommitted run session and return the runtime error.
    #[must_use]
    pub fn into_error(self) -> RunError {
        self.error
    }

    /// Discards the runtime error and recovers the owned parsed program.
    ///
    /// This drops the failed runtime state. Failed transitions are terminal;
    /// callers cannot resume the failed step by recovering a session.
    #[must_use]
    pub fn into_program(self) -> Program {
        self.session.into_program()
    }

    /// Splits this failed transition into its runtime error and parsed program.
    #[must_use]
    pub fn into_parts(self) -> (RunError, Program) {
        let program = self.session.into_program();
        (self.error, program)
    }
}

impl OwnedRuleAttemptFailedRun {
    /// Captures a failed owned rule-attempt session without committing runtime state.
    fn new(error: RunError, session: OwnedRuleAttemptSession) -> Self {
        Self { error, session }
    }

    /// Runtime error that prevented the rule attempt from completing.
    #[must_use]
    pub const fn error(&self) -> &RunError {
        &self.error
    }

    /// Number of rule attempts consumed before the failure was reported.
    #[must_use]
    pub const fn completed_attempts(&self) -> RuleAttemptCount {
        self.session.completed_attempts()
    }

    /// Number of rewrite steps that completed before the failed rule attempt.
    #[must_use]
    pub const fn completed_steps(&self) -> StepCount {
        self.session.completed_steps()
    }

    /// Borrow the parsed program owned by this failed session.
    #[must_use]
    pub fn program(&self) -> &Program {
        self.session.program()
    }

    /// Borrow the uncommitted runtime state preserved by this error.
    #[must_use]
    pub fn state(&self) -> RuntimeStateView<'_> {
        self.session.state()
    }

    /// Discard the uncommitted run session and return the runtime error.
    #[must_use]
    pub fn into_error(self) -> RunError {
        self.error
    }

    /// Discards the runtime error and recovers the owned parsed program.
    ///
    /// This drops the failed runtime state. Failed transitions are terminal;
    /// callers cannot resume the failed rule-attempt step by recovering a
    /// session.
    #[must_use]
    pub fn into_program(self) -> Program {
        self.session.into_program()
    }

    /// Splits this failed transition into its runtime error and parsed program.
    #[must_use]
    pub fn into_parts(self) -> (RunError, Program) {
        let program = self.session.into_program();
        (self.error, program)
    }
}

impl core::fmt::Display for OwnedRuleAttemptFailedRun {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl core::error::Error for OwnedRuleAttemptFailedRun {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl core::fmt::Display for OwnedFailedRun {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl core::error::Error for OwnedFailedRun {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.error)
    }
}
