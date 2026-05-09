use alloc::vec::Vec;
use core::convert::Infallible;

use crate::allocation::{AllocationContext, try_push, try_reserve_total_exact};
use crate::bytes::{Payload, RuntimeByte, copy_runtime_bytes, push_runtime_bytes};
use crate::error::{
    ReturnLimitError, RunError, StateLimitContext, StateLimitError, StateSizeError, StepLimitError,
    TracedRunError,
};
use crate::program::{Program, RunLimits, RunResult};
use crate::rule::{Action, PayloadView, Rule, RuleAnchor, RulePosition, RuntimeRuleState};
use crate::trace::{BorrowedTraceEffect, BorrowedTraceEvent, RuntimeStateView};

type NoTrace<'program> = for<'run> fn(BorrowedTraceEvent<'program, 'run>) -> Result<(), Infallible>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct State {
    pub(crate) bytes: Vec<RuntimeByte>,
}

impl State {
    pub(crate) fn parse_input(input: &[u8], limits: RunLimits) -> Result<Self, RunError> {
        if input.len() > limits.max_state_len() {
            return Err(StateLimitError::new(
                limits.max_state_len(),
                input.len(),
                StateLimitContext::Input,
            )
            .into());
        }

        let mut bytes = Vec::new();
        try_reserve_total_exact(&mut bytes, input.len(), AllocationContext::RuntimeInput)?;

        for (zero_based_column, byte) in input.iter().copied().enumerate() {
            try_push(
                &mut bytes,
                RuntimeByte::parse_input(byte, zero_based_column)?,
                AllocationContext::RuntimeInput,
            )?;
        }

        Ok(Self { bytes })
    }

    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn starts_with_payload(&self, payload: &Payload) -> Option<StateMatch> {
        self.matches_payload_at(0, payload)
    }

    pub(crate) fn ends_with_payload(&self, payload: &Payload) -> Option<StateMatch> {
        let start = self.len().checked_sub(payload.len())?;
        self.matches_payload_at(start, payload)
    }

    pub(crate) fn find_payload(&self, payload: &Payload) -> Option<StateMatch> {
        if payload.is_empty() {
            return StateMatch::checked(0, 0, self.len());
        }

        let last_start = self.len().checked_sub(payload.len())?;
        (0..=last_start).find_map(|position| self.matches_payload_at(position, payload))
    }

    pub(crate) fn matches_payload_at(
        &self,
        position: usize,
        payload: &Payload,
    ) -> Option<StateMatch> {
        let state_match = StateMatch::checked(position, payload.len(), self.len())?;
        let window = self.bytes.get(state_match.position()..state_match.end())?;

        window
            .iter()
            .copied()
            .zip(payload.bytes().iter().copied())
            .all(|(state_byte, code_byte)| state_byte.as_u8() == code_byte.as_u8())
            .then_some(state_match)
    }

    pub(crate) fn replace_at_into(
        &self,
        state_match: StateMatch,
        rhs: &Payload,
        output: &mut Vec<RuntimeByte>,
        limits: RunLimits,
    ) -> Result<(), RunError> {
        self.prepare_replacement_buffer(state_match, rhs, output, limits)?;
        self.push_prefix(output, state_match)?;
        push_runtime_bytes(output, rhs.runtime_bytes())?;
        self.push_suffix(output, state_match)?;
        Ok(())
    }

    pub(crate) fn move_start_at_into(
        &self,
        state_match: StateMatch,
        rhs: &Payload,
        output: &mut Vec<RuntimeByte>,
        limits: RunLimits,
    ) -> Result<(), RunError> {
        self.prepare_replacement_buffer(state_match, rhs, output, limits)?;
        push_runtime_bytes(output, rhs.runtime_bytes())?;
        self.push_prefix(output, state_match)?;
        self.push_suffix(output, state_match)?;
        Ok(())
    }

    pub(crate) fn move_end_at_into(
        &self,
        state_match: StateMatch,
        rhs: &Payload,
        output: &mut Vec<RuntimeByte>,
        limits: RunLimits,
    ) -> Result<(), RunError> {
        self.prepare_replacement_buffer(state_match, rhs, output, limits)?;
        self.push_prefix(output, state_match)?;
        self.push_suffix(output, state_match)?;
        push_runtime_bytes(output, rhs.runtime_bytes())?;
        Ok(())
    }

    fn replaced_len(
        &self,
        state_match: StateMatch,
        rhs: &Payload,
    ) -> Result<usize, StateSizeError> {
        self.len()
            .checked_sub(state_match.lhs_len())
            .and_then(|base| base.checked_add(rhs.len()))
            .ok_or_else(|| StateSizeError::new(self.len(), state_match.lhs_len(), rhs.len()))
    }

    fn prepare_replacement_buffer(
        &self,
        state_match: StateMatch,
        rhs: &Payload,
        output: &mut Vec<RuntimeByte>,
        limits: RunLimits,
    ) -> Result<(), RunError> {
        let capacity = self.replaced_len(state_match, rhs)?;

        if capacity > limits.max_state_len() {
            return Err(StateLimitError::new(
                limits.max_state_len(),
                capacity,
                StateLimitContext::Rewrite,
            )
            .into());
        }

        output.clear();
        try_reserve_total_exact(output, capacity, AllocationContext::RuntimeState)?;
        Ok(())
    }

    fn push_prefix(
        &self,
        output: &mut Vec<RuntimeByte>,
        state_match: StateMatch,
    ) -> Result<(), crate::allocation::AllocationError> {
        push_runtime_bytes(
            output,
            self.bytes.iter().take(state_match.position()).copied(),
        )
    }

    fn push_suffix(
        &self,
        output: &mut Vec<RuntimeByte>,
        state_match: StateMatch,
    ) -> Result<(), crate::allocation::AllocationError> {
        push_runtime_bytes(output, self.bytes.iter().skip(state_match.end()).copied())
    }

    pub(crate) fn into_output(self) -> Result<Vec<u8>, crate::allocation::AllocationError> {
        copy_runtime_bytes(&self.bytes, AllocationContext::FinalOutput)
    }

    pub(crate) fn into_step_limit_state(
        self,
    ) -> Result<Vec<u8>, crate::allocation::AllocationError> {
        copy_runtime_bytes(&self.bytes, AllocationContext::StepLimitState)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StateMatch {
    position: usize,
    lhs_len: usize,
}

impl StateMatch {
    pub(crate) fn checked(position: usize, lhs_len: usize, state_len: usize) -> Option<Self> {
        let end = position.checked_add(lhs_len)?;
        (position <= state_len && end <= state_len).then_some(Self { position, lhs_len })
    }

    pub(crate) const fn position(self) -> usize {
        self.position
    }

    pub(crate) const fn lhs_len(self) -> usize {
        self.lhs_len
    }

    pub(crate) const fn end(self) -> usize {
        self.position + self.lhs_len
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RewriteEffect<'program> {
    Continue,
    Return(PayloadView<'program>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MatchedRule<'program> {
    position: RulePosition,
    rule: &'program Rule,
    state_match: StateMatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Runtime<'program> {
    program: &'program Program,
    state: State,
    scratch: Vec<RuntimeByte>,
    steps: usize,
    once_states: Vec<RuntimeRuleState>,
    limits: RunLimits,
}

impl<'program> Runtime<'program> {
    pub(crate) fn new(
        program: &'program Program,
        input: &[u8],
        limits: RunLimits,
    ) -> Result<Self, RunError> {
        let state = State::parse_input(input, limits)?;
        let mut once_states = Vec::new();
        try_reserve_total_exact(
            &mut once_states,
            program.runtime_once_rule_count(),
            AllocationContext::RuntimeRuleState,
        )?;

        for _ in 0..program.runtime_once_rule_count() {
            try_push(
                &mut once_states,
                RuntimeRuleState::Fresh,
                AllocationContext::RuntimeRuleState,
            )?;
        }

        let scratch = Vec::new();

        Ok(Self {
            program,
            state,
            scratch,
            steps: 0,
            once_states,
            limits,
        })
    }

    pub(crate) fn run(self) -> Result<RunResult, RunError> {
        match self.run_impl::<NoTrace<'program>, Infallible>(None) {
            Ok(result) => Ok(result),
            Err(TracedRunError::Run(error)) => Err(error),
            Err(TracedRunError::Trace(error)) => match error {},
        }
    }

    pub(crate) fn run_with_borrowed_trace<F, E>(
        self,
        trace: F,
    ) -> Result<RunResult, TracedRunError<E>>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), E>,
    {
        self.run_impl(Some(trace))
    }

    fn run_impl<F, E>(mut self, mut trace: Option<F>) -> Result<RunResult, TracedRunError<E>>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), E>,
    {
        self.emit_initial_trace(&mut trace)?;

        loop {
            let Some(matched) = self.find_next_match() else {
                return Ok(RunResult::stable(
                    self.state.into_output().map_err(RunError::from)?,
                    self.steps,
                ));
            };

            if self.steps >= self.limits.max_steps() {
                let state = self.state.into_step_limit_state().map_err(RunError::from)?;
                return Err(RunError::StepLimit(StepLimitError::new(
                    self.limits.max_steps(),
                    state,
                ))
                .into());
            }

            self.consume_once_rule(matched.rule);

            if let Some(result) = self.apply_rule(matched, &mut trace)? {
                return Ok(result);
            }
        }
    }

    fn emit_initial_trace<F, E>(&self, trace: &mut Option<F>) -> Result<(), TracedRunError<E>>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), E>,
    {
        if let Some(trace) = trace.as_mut() {
            trace(BorrowedTraceEvent::Initial {
                state: RuntimeStateView::new(&self.state.bytes),
            })
            .map_err(TracedRunError::Trace)?;
        }

        Ok(())
    }

    fn find_next_match(&self) -> Option<MatchedRule<'program>> {
        let program = self.program;

        for (index, rule) in program.rule_slice().iter().enumerate() {
            if !self.is_rule_available(rule) {
                continue;
            }

            let Some(state_match) = find_match(&self.state, rule) else {
                continue;
            };

            return Some(MatchedRule {
                position: RulePosition::new(index),
                rule,
                state_match,
            });
        }

        None
    }

    fn is_rule_available(&self, rule: &Rule) -> bool {
        let Some(position) = rule.once_position() else {
            return true;
        };

        self.once_states
            .get(position.zero_based())
            .copied()
            .is_some_and(|state| !state.is_consumed())
    }

    fn consume_once_rule(&mut self, rule: &Rule) {
        if let Some(position) = rule.once_position()
            && let Some(state) = self.once_states.get_mut(position.zero_based())
        {
            *state = RuntimeRuleState::Consumed;
        }
    }

    fn apply_rule<F, E>(
        &mut self,
        matched: MatchedRule<'program>,
        trace: &mut Option<F>,
    ) -> Result<Option<RunResult>, TracedRunError<E>>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), E>,
    {
        let effect = self
            .apply_action_to_scratch(matched.state_match, &matched.rule.action)
            .map_err(TracedRunError::Run)?;

        self.steps += 1;

        match effect {
            RewriteEffect::Continue => {
                Self::emit_step_trace(
                    trace,
                    self.steps,
                    matched.position,
                    matched.rule,
                    BorrowedTraceEffect::Continue {
                        state: RuntimeStateView::new(&self.scratch),
                    },
                )?;
                core::mem::swap(&mut self.state.bytes, &mut self.scratch);
                Ok(None)
            }
            RewriteEffect::Return(output) => {
                Self::emit_step_trace(
                    trace,
                    self.steps,
                    matched.position,
                    matched.rule,
                    BorrowedTraceEffect::Return { output },
                )?;

                Ok(Some(RunResult::from_return(
                    output
                        .to_vec_with_context(AllocationContext::ReturnOutput)
                        .map_err(RunError::from)?,
                    self.steps,
                )))
            }
        }
    }

    fn apply_action_to_scratch(
        &mut self,
        state_match: StateMatch,
        action: &'program Action,
    ) -> Result<RewriteEffect<'program>, RunError> {
        match action {
            Action::Replace(rhs) => {
                self.state
                    .replace_at_into(state_match, rhs, &mut self.scratch, self.limits)?;
                Ok(RewriteEffect::Continue)
            }
            Action::MoveStart(rhs) => {
                self.state
                    .move_start_at_into(state_match, rhs, &mut self.scratch, self.limits)?;
                Ok(RewriteEffect::Continue)
            }
            Action::MoveEnd(rhs) => {
                self.state
                    .move_end_at_into(state_match, rhs, &mut self.scratch, self.limits)?;
                Ok(RewriteEffect::Continue)
            }
            Action::Return(output) => {
                if output.len() > self.limits.max_return_len() {
                    return Err(
                        ReturnLimitError::new(self.limits.max_return_len(), output.len()).into(),
                    );
                }

                Ok(RewriteEffect::Return(PayloadView::new(output)))
            }
        }
    }

    fn emit_step_trace<F, E>(
        trace: &mut Option<F>,
        step: usize,
        position: RulePosition,
        rule: &'program Rule,
        effect: BorrowedTraceEffect<'program, '_>,
    ) -> Result<(), TracedRunError<E>>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), E>,
    {
        if let Some(trace) = trace.as_mut() {
            trace(BorrowedTraceEvent::Step {
                step,
                rule: rule.view(position),
                effect,
            })
            .map_err(TracedRunError::Trace)?;
        }

        Ok(())
    }
}

fn find_match(state: &State, rule: &Rule) -> Option<StateMatch> {
    match rule.anchor {
        RuleAnchor::Anywhere => state.find_payload(&rule.lhs),
        RuleAnchor::Start => state.starts_with_payload(&rule.lhs),
        RuleAnchor::End => state.ends_with_payload(&rule.lhs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes::{CodeByte, CompactByte, Payload};
    use crate::test_support::{
        TestFailure, TestResult, expect_input_error, expect_run_error, expect_step_limit,
        run_source,
    };
    use crate::{PayloadKind, Program, RunLimits};
    use std::string::String;
    use std::vec::Vec;

    fn expect_runtime_byte(state: &State, index: usize) -> Result<RuntimeByte, TestFailure> {
        state
            .bytes
            .get(index)
            .copied()
            .ok_or(TestFailure::Message("expected runtime byte"))
    }

    fn expect_payload_byte(payload: &Payload, index: usize) -> Result<u8, TestFailure> {
        payload
            .bytes()
            .get(index)
            .copied()
            .map(CodeByte::as_u8)
            .ok_or(TestFailure::Message("expected payload byte"))
    }

    #[test]
    fn normal_replacement_is_ordered_and_leftmost() -> TestResult {
        let source = "aa=x\na=y";
        assert_eq!(run_source(source, "aaaa")?, "xx");
        Ok(())
    }

    #[test]
    fn anchors_match_only_at_their_edges() -> TestResult {
        assert_eq!(run_source("(start)a=x", "aba")?, "xba");
        assert_eq!(run_source("(start)a=x", "ba")?, "ba");
        assert_eq!(run_source("(end)a=x", "aba")?, "abx");
        assert_eq!(run_source("(end)a=x", "ab")?, "ab");
        Ok(())
    }

    #[test]
    fn move_actions_work() -> TestResult {
        assert_eq!(run_source("a=(start)x", "ba")?, "xb");
        assert_eq!(run_source("a=(end)x", "ba")?, "bx");
        Ok(())
    }

    #[test]
    fn empty_lhs_anywhere_matches_at_start() -> TestResult {
        let source = "(once)=x\n(start)x=(return)ok";
        let result = Program::parse_str(source)?.run(b"ab", RunLimits::new(2))?;

        assert_eq!(result.output(), b"ok");
        assert_eq!(result.steps(), 2);
        assert!(result.returned());
        Ok(())
    }

    #[test]
    fn empty_lhs_start_and_end_anchors_pick_different_edges() -> TestResult {
        let start_result = Program::parse_str("(once)(start)=x\nxab=(return)start")?
            .run(b"ab", RunLimits::new(2))?;
        let end_result =
            Program::parse_str("(once)(end)=x\nabx=(return)end")?.run(b"ab", RunLimits::new(2))?;

        assert_eq!(start_result.output(), b"start");
        assert_eq!(end_result.output(), b"end");
        Ok(())
    }

    #[test]
    fn once_rule_is_used_at_most_once() -> TestResult {
        let source = "(once)a=b\na=c";
        assert_eq!(run_source(source, "aa")?, "bc");
        Ok(())
    }

    #[test]
    fn return_discards_current_state() -> TestResult {
        let source = "aa=(return)ok\na=x";
        assert_eq!(run_source(source, "aabb")?, "ok");
        Ok(())
    }

    #[test]
    fn runtime_only_bytes_are_preserved_until_return_discards_them() -> TestResult {
        assert_eq!(run_source("a=b", "a=()#c")?, "b=()#c");
        let result = Program::parse_str("a=(return)x")?.run(b"a=()#c", RunLimits::new(1))?;
        assert_eq!(result.output(), b"x");
        assert!(result.returned());
        Ok(())
    }

    #[test]
    fn input_spaces_are_preserved_and_do_not_bridge_matches() -> TestResult {
        assert_eq!(run_source("a= b", "a bc")?, "b bc");
        assert_eq!(run_source("a b=bb", "a bc")?, "a bc");
        assert_eq!(run_source("ab=bb", "a bc")?, "a bc");
        Ok(())
    }

    #[test]
    fn runtime_input_error_is_structured() -> TestResult {
        let error = expect_run_error(
            Program::parse_str("a=b")?.run("aあ".as_bytes(), RunLimits::default()),
        )?;
        let error = expect_input_error(error)?;

        assert_eq!(error.column(), 2);
        Ok(())
    }

    #[test]
    fn runtime_state_can_hold_reserved_bytes_that_program_payloads_cannot_construct() -> TestResult
    {
        let program = Program::parse_str("a=b")?;
        assert!(Program::parse_str("a=(return)(").is_err());
        assert!(Program::parse_str("a=b)").is_err());

        let result = program.run(b"a=#()", RunLimits::new(10_000))?;
        assert_eq!(String::from_utf8(result.into_output())?, "b=#()");
        Ok(())
    }

    #[test]
    fn step_limit_allows_exact_limit_but_blocks_next_match() -> TestResult {
        let exact = Program::parse_str("a=b")?.run(b"a", RunLimits::new(1))?;
        assert_eq!(exact.output(), b"b");
        assert_eq!(exact.steps(), 1);

        let no_match = Program::parse_str("a=b")?.run(b"x", RunLimits::new(0))?;
        assert_eq!(no_match.output(), b"x");
        assert_eq!(no_match.steps(), 0);

        let error = expect_run_error(Program::parse_str("a=b")?.run(b"a", RunLimits::new(0)))?;
        let error = expect_step_limit(error)?;
        assert_eq!(error.max_steps(), 0);
        assert_eq!(error.state(), b"a");
        Ok(())
    }

    #[test]
    fn step_limit_error_keeps_state_as_bytes() -> TestResult {
        let error = expect_run_error(Program::parse_str("=a")?.run(b"", RunLimits::new(3)))?;
        let error = expect_step_limit(error)?;

        assert_eq!(error.max_steps(), 3);
        assert_eq!(error.state(), b"aaa");
        Ok(())
    }

    #[test]
    fn palindrome_example_returns_true_or_false() -> TestResult {
        let source = "\
b=a|a|
c=a|aa|
a|-=
--=(return)false
(start)a|=(end)-
(start)a=(end)|-
=(return)true";

        assert_eq!(run_source(source, "aba")?, "true");
        assert_eq!(run_source(source, "ab")?, "false");
        Ok(())
    }

    #[test]
    fn runtime_accepts_every_ascii_input_byte() -> TestResult {
        let input: Vec<u8> = (0x00..=0x7f).collect();
        let result =
            Program::parse_str("# no executable rules")?.run(&input, RunLimits::default())?;

        assert_eq!(result.output(), input.as_slice());
        assert_eq!(result.steps(), 0);
        Ok(())
    }

    #[test]
    fn runtime_rejects_every_non_ascii_input_byte() -> TestResult {
        let program = Program::parse_str("# no executable rules")?;

        for byte in 0x80..=0xff {
            assert!(
                program.run(&[byte], RunLimits::default()).is_err(),
                "byte should be rejected: {byte:#04x}",
            );
        }

        Ok(())
    }

    #[test]
    fn internal_code_and_runtime_bytes_are_distinct_domains() -> TestResult {
        let compact = [CompactByte::new(b'a', 1)];
        let payload = Payload::parse(&compact, 1, PayloadKind::LeftSideData)?;
        let state = State::parse_input(b"a=()# ", RunLimits::default())?;

        assert_eq!(expect_payload_byte(&payload, 0)?, b'a');
        assert_eq!(expect_runtime_byte(&state, 0)?.as_u8(), b'a');
        assert_eq!(expect_runtime_byte(&state, 1)?.as_u8(), b'=');
        assert_eq!(expect_runtime_byte(&state, 2)?.as_u8(), b'(');
        assert_eq!(expect_runtime_byte(&state, 5)?.as_u8(), b' ');
        Ok(())
    }
}
