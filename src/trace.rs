use alloc::vec::Vec;

use crate::rule::RuleInfo;

pub enum TraceEffect {
    /// The step produced the next runtime state and execution may continue.
    Continue { state: Vec<u8> },
    /// The step executed `(return)` and produced final output bytes.
    Return { output: Vec<u8> },
}

impl TraceEffect {
    /// State/output bytes carried by this effect.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::Continue { state } => state,
            Self::Return { output } => output,
        }
    }

    /// Whether this effect stopped execution by `(return)`.
    #[must_use]
    pub const fn is_return(&self) -> bool {
        matches!(self, Self::Return { .. })
    }
}

/// Trace event emitted by [`Program::run_with_trace`] and
/// [`Program::try_run_with_trace`].
///
/// Step events carry borrowed rule metadata and a structured effect. Return
/// steps cannot be confused with ordinary continuation steps by forgetting to
/// inspect a boolean flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceEvent<'program> {
    /// Initial runtime state before any rewrite step.
    Initial { state: Vec<u8> },
    /// One applied rule.
    Step {
        /// One-based applied step count.
        step: usize,
        /// Metadata for the applied rule.
        rule: RuleInfo<'program>,
        /// Structured result of the rewrite step.
        effect: TraceEffect,
    },
}

impl TraceEvent<'_> {
    /// State/output bytes carried by this event.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::Initial { state } => state,
            Self::Step { effect, .. } => effect.bytes(),
        }
    }

    /// Whether this event is a step that stopped execution by `(return)`.
    #[must_use]
    pub const fn is_return_step(&self) -> bool {
        match self {
            Self::Initial { .. } => false,
            Self::Step { effect, .. } => effect.is_return(),
        }
    }
}

/// Combined one-shot error used by [`run`].
