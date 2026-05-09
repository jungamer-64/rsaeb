use alloc::vec::Vec;

use crate::rule::RuleView;

#[derive(Debug, Clone, PartialEq, Eq)]
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
/// Step events carry a borrowed structured rule view and a structured effect. Return
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
        /// Structured view of the applied rule.
        rule: RuleView<'program>,
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

#[cfg(test)]
mod tests {
    use crate::test_support::{TestFailure, TestResult, expect_event};
    use crate::{Program, RuleActionView, RunOptions, TraceEffect, TraceEvent, TracedRunError};
    use std::vec::Vec;
    #[test]
    fn trace_events_are_emitted_without_core_stderr() -> TestResult {
        let program = Program::parse("a=b\nb=(return)ok")?;
        let mut events = Vec::new();
        let result = program.run_with_trace(b"a", RunOptions::new(10_000), |event| {
            events.push(event);
        })?;

        assert_eq!(result.output(), b"ok");
        assert!(result.returned());
        assert_eq!(events.len(), 3);

        let initial = expect_event(&events, 0)?;
        let first_step = expect_event(&events, 1)?;
        let second_step = expect_event(&events, 2)?;

        assert!(matches!(initial, TraceEvent::Initial { .. }));
        assert_eq!(initial.bytes(), b"a");
        assert_eq!(first_step.bytes(), b"b");
        assert_eq!(second_step.bytes(), b"ok");
        assert!(!first_step.is_return_step());
        assert!(second_step.is_return_step());

        match first_step {
            TraceEvent::Step {
                rule,
                effect: TraceEffect::Continue { state },
                ..
            } => {
                assert_eq!(state.as_slice(), b"b");
                assert_eq!(rule.position().zero_based(), 0);
                assert_eq!(rule.line_number(), 1);
                assert!(rule.lhs().eq_bytes(b"a"));
                assert!(matches!(
                    rule.action(),
                    RuleActionView::Replace(payload) if payload.eq_bytes(b"b")
                ));
                assert_eq!(rule.compact_source(), b"a=b");
            }
            TraceEvent::Initial { .. } | TraceEvent::Step { .. } => {
                return Err(TestFailure::Message("expected continuing step event"));
            }
        }

        Ok(())
    }

    #[test]
    fn fallible_trace_callback_can_abort_execution() -> TestResult {
        let program = Program::parse("a=b\nb=c")?;
        let result = program.try_run_with_trace(b"a", RunOptions::new(10_000), |_event| {
            Err::<(), _>("trace sink full")
        });

        assert_eq!(result, Err(TracedRunError::Trace("trace sink full")));
        Ok(())
    }

    #[test]
    fn traced_final_event_matches_run_result() -> TestResult {
        let program = Program::parse("a=b\nb=(return)c")?;
        let mut events = Vec::new();

        let result = program.run_with_trace(b"a", RunOptions::new(10), |event| {
            events.push(event);
        })?;

        let last = events
            .last()
            .ok_or(TestFailure::Message("expected final trace event"))?;
        assert_eq!(last.bytes(), result.output());
        assert_eq!(events.len(), result.steps() + 1);
        assert!(last.is_return_step());
        Ok(())
    }
}
