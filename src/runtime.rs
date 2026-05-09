use alloc::vec::Vec;
use core::convert::Infallible;

use crate::allocation::{AllocationContext, AllocationError, copy_bytes, try_reserve_total_exact};
use crate::bytes::{Payload, RuntimeByte, copy_runtime_bytes, push_runtime_bytes};
use crate::error::{RunError, StateSizeError, StepLimitError, TracedRunError};
use crate::program::{Program, RunResult};
use crate::rule::{Action, Rule, RuleAnchor, RulePosition, RuntimeRuleState};
use crate::trace::{TraceEffect, TraceEvent};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct State {
    pub(crate) bytes: Vec<RuntimeByte>,
}

impl State {
    pub(crate) fn parse_input(input: &[u8]) -> Result<Self, RunError> {
        let mut bytes = Vec::new();
        try_reserve_total_exact(&mut bytes, input.len(), AllocationContext::RuntimeInput)?;

        for (zero_based_column, byte) in input.iter().copied().enumerate() {
            bytes.push(RuntimeByte::parse_input(byte, zero_based_column)?);
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
    ) -> Result<(), RunError> {
        self.prepare_replacement_buffer(state_match, rhs, output)?;
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
    ) -> Result<(), RunError> {
        self.prepare_replacement_buffer(state_match, rhs, output)?;
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
    ) -> Result<(), RunError> {
        self.prepare_replacement_buffer(state_match, rhs, output)?;
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
    ) -> Result<(), RunError> {
        let capacity = self.replaced_len(state_match, rhs)?;
        output.clear();
        try_reserve_total_exact(output, capacity, AllocationContext::RuntimeState)?;
        Ok(())
    }

    fn push_prefix(
        &self,
        output: &mut Vec<RuntimeByte>,
        state_match: StateMatch,
    ) -> Result<(), AllocationError> {
        push_runtime_bytes(output, self.bytes[..state_match.position()].iter().copied())
    }

    fn push_suffix(
        &self,
        output: &mut Vec<RuntimeByte>,
        state_match: StateMatch,
    ) -> Result<(), AllocationError> {
        push_runtime_bytes(output, self.bytes[state_match.end()..].iter().copied())
    }

    pub(crate) fn snapshot(&self) -> Result<Vec<u8>, AllocationError> {
        copy_runtime_bytes(&self.bytes, AllocationContext::TraceSnapshot)
    }

    pub(crate) fn into_output(self) -> Result<Vec<u8>, AllocationError> {
        copy_runtime_bytes(&self.bytes, AllocationContext::FinalOutput)
    }

    pub(crate) fn into_step_limit_state(self) -> Result<Vec<u8>, AllocationError> {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RewriteEffect {
    Continue,
    Return(Vec<u8>),
}

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
}

impl<'program> Runtime<'program> {
    pub(crate) fn new(program: &'program Program, input: &[u8]) -> Result<Self, RunError> {
        let state = State::parse_input(input)?;
        let mut once_states = Vec::new();
        try_reserve_total_exact(
            &mut once_states,
            program.rules.len(),
            AllocationContext::RuntimeRuleState,
        )?;

        for _ in &program.rules {
            once_states.push(RuntimeRuleState::Fresh);
        }

        let scratch = Vec::new();

        Ok(Self {
            program,
            state,
            scratch,
            steps: 0,
            once_states,
        })
    }

    pub(crate) fn run(self, max_steps: usize) -> Result<RunResult, RunError> {
        match self.run_impl::<fn(TraceEvent<'program>) -> Result<(), Infallible>, Infallible>(
            max_steps, None,
        ) {
            Ok(result) => Ok(result),
            Err(TracedRunError::Run(error)) => Err(error),
            Err(TracedRunError::Trace(error)) => match error {},
        }
    }

    pub(crate) fn run_with_trace<F, E>(
        self,
        max_steps: usize,
        trace: F,
    ) -> Result<RunResult, TracedRunError<E>>
    where
        F: FnMut(TraceEvent<'program>) -> Result<(), E>,
    {
        self.run_impl(max_steps, Some(trace))
    }

    fn run_impl<F, E>(
        mut self,
        max_steps: usize,
        mut trace: Option<F>,
    ) -> Result<RunResult, TracedRunError<E>>
    where
        F: FnMut(TraceEvent<'program>) -> Result<(), E>,
    {
        self.emit_initial_trace(&mut trace)?;

        loop {
            if self.steps >= max_steps {
                if self.has_match() {
                    let state = self.state.into_step_limit_state().map_err(RunError::from)?;
                    return Err(RunError::StepLimit(StepLimitError::new(max_steps, state)).into());
                }

                return Ok(RunResult::stable(
                    self.state.into_output().map_err(RunError::from)?,
                    self.steps,
                ));
            }

            let Some(matched) = self.find_next_match() else {
                return Ok(RunResult::stable(
                    self.state.into_output().map_err(RunError::from)?,
                    self.steps,
                ));
            };

            if let Some(result) = self.apply_rule(matched, &mut trace)? {
                return Ok(result);
            }
        }
    }

    fn emit_initial_trace<F, E>(&self, trace: &mut Option<F>) -> Result<(), TracedRunError<E>>
    where
        F: FnMut(TraceEvent<'program>) -> Result<(), E>,
    {
        if let Some(trace) = trace.as_mut() {
            let state = self.state.snapshot().map_err(RunError::from)?;
            trace(TraceEvent::Initial { state }).map_err(TracedRunError::Trace)?;
        }

        Ok(())
    }

    fn has_match(&self) -> bool {
        let program = self.program;

        program.rules.iter().enumerate().any(|(index, rule)| {
            let is_available = !rule.repeat.is_once() || !self.once_states[index].is_consumed();
            is_available && find_match(&self.state, rule).is_some()
        })
    }

    fn find_next_match(&mut self) -> Option<MatchedRule<'program>> {
        let program = self.program;

        for (index, rule) in program.rules.iter().enumerate() {
            let is_available = !rule.repeat.is_once() || !self.once_states[index].is_consumed();

            if !is_available {
                continue;
            }

            let Some(state_match) = find_match(&self.state, rule) else {
                continue;
            };

            if rule.repeat.is_once() {
                self.once_states[index] = RuntimeRuleState::Consumed;
            }

            return Some(MatchedRule {
                position: RulePosition::new(index),
                rule,
                state_match,
            });
        }

        None
    }

    fn apply_rule<F, E>(
        &mut self,
        matched: MatchedRule<'program>,
        trace: &mut Option<F>,
    ) -> Result<Option<RunResult>, TracedRunError<E>>
    where
        F: FnMut(TraceEvent<'program>) -> Result<(), E>,
    {
        let effect = self
            .apply_action_to_scratch(matched.state_match, &matched.rule.action)
            .map_err(TracedRunError::Run)?;

        self.steps += 1;

        match effect {
            RewriteEffect::Continue => {
                self.emit_step_trace(
                    trace,
                    self.steps,
                    matched.position,
                    matched.rule,
                    TraceStepPayload::State(&self.scratch),
                )?;
                core::mem::swap(&mut self.state.bytes, &mut self.scratch);
                Ok(None)
            }
            RewriteEffect::Return(output) => {
                self.emit_step_trace(
                    trace,
                    self.steps,
                    matched.position,
                    matched.rule,
                    TraceStepPayload::Return(&output),
                )?;

                Ok(Some(RunResult::from_return(output, self.steps)))
            }
        }
    }

    fn apply_action_to_scratch(
        &mut self,
        state_match: StateMatch,
        action: &Action,
    ) -> Result<RewriteEffect, RunError> {
        match action {
            Action::Replace(rhs) => {
                self.state
                    .replace_at_into(state_match, rhs, &mut self.scratch)?;
                Ok(RewriteEffect::Continue)
            }
            Action::MoveStart(rhs) => {
                self.state
                    .move_start_at_into(state_match, rhs, &mut self.scratch)?;
                Ok(RewriteEffect::Continue)
            }
            Action::MoveEnd(rhs) => {
                self.state
                    .move_end_at_into(state_match, rhs, &mut self.scratch)?;
                Ok(RewriteEffect::Continue)
            }
            Action::Return(output) => Ok(RewriteEffect::Return(output.to_output()?)),
        }
    }

    fn emit_step_trace<F, E>(
        &self,
        trace: &mut Option<F>,
        step: usize,
        position: RulePosition,
        rule: &'program Rule,
        payload: TraceStepPayload<'_>,
    ) -> Result<(), TracedRunError<E>>
    where
        F: FnMut(TraceEvent<'program>) -> Result<(), E>,
    {
        if let Some(trace) = trace.as_mut() {
            let effect = match payload {
                TraceStepPayload::State(state) => TraceEffect::Continue {
                    state: copy_runtime_bytes(state, AllocationContext::TraceSnapshot)
                        .map_err(RunError::from)?,
                },
                TraceStepPayload::Return(output) => TraceEffect::Return {
                    output: copy_bytes(output, AllocationContext::TraceSnapshot)
                        .map_err(RunError::from)?,
                },
            };

            trace(TraceEvent::Step {
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

enum TraceStepPayload<'a> {
    State(&'a [RuntimeByte]),
    Return(&'a [u8]),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes::{CompactByte, Payload};
    use crate::test_support::{
        TestResult, expect_input_error, expect_run_error, expect_step_limit, run_source,
    };
    use crate::{PayloadKind, Program, RunOptions};
    use std::string::String;
    #[test]
    fn normal_replacement_is_ordered_and_leftmost() -> TestResult {
        let source = "aa=x\na=y";
        assert_eq!(run_source(source, "aaaa")?, "xx");
        Ok(())
    }

    #[test]
    fn start_anchor_matches_only_at_start() -> TestResult {
        let source = "(start)a=x";
        assert_eq!(run_source(source, "aba")?, "xba");
        assert_eq!(run_source(source, "ba")?, "ba");
        Ok(())
    }

    #[test]
    fn end_anchor_matches_only_at_end() -> TestResult {
        let source = "(end)a=x";
        assert_eq!(run_source(source, "aba")?, "abx");
        assert_eq!(run_source(source, "ab")?, "ab");
        Ok(())
    }

    #[test]
    fn runtime_continues_after_anchored_replacement() -> TestResult {
        let source = "(start)a=x\na=y";
        assert_eq!(run_source(source, "aba")?, "xby");

        let source = "(end)a=x\na=y";
        assert_eq!(run_source(source, "aba")?, "ybx");
        Ok(())
    }

    #[test]
    fn move_start_works() -> TestResult {
        let source = "a=(start)x";
        assert_eq!(run_source(source, "ba")?, "xb");
        Ok(())
    }

    #[test]
    fn move_end_works() -> TestResult {
        let source = "a=(end)x";
        assert_eq!(run_source(source, "ba")?, "bx");
        Ok(())
    }

    #[test]
    fn empty_lhs_anywhere_matches_at_start() -> TestResult {
        let source = "(once)=x\n(start)x=(return)ok";
        let result = Program::parse(source)?.run(b"ab", RunOptions::new(2))?;

        assert_eq!(result.output(), b"ok");
        assert_eq!(result.steps(), 2);
        assert!(result.returned());
        Ok(())
    }

    #[test]
    fn empty_lhs_start_and_end_anchors_pick_different_edges() -> TestResult {
        let start_result =
            Program::parse("(once)(start)=x\nxab=(return)start")?.run(b"ab", RunOptions::new(2))?;
        let end_result =
            Program::parse("(once)(end)=x\nabx=(return)end")?.run(b"ab", RunOptions::new(2))?;

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
    fn return_discards_runtime_only_bytes_explicitly() -> TestResult {
        let result = Program::parse("a=(return)x")?.run(b"a=()#c", RunOptions::new(1))?;

        assert_eq!(result.output(), b"x");
        assert!(result.returned());
        Ok(())
    }

    #[test]
    fn empty_lhs_inserts_at_start() -> TestResult {
        let source = "aaa=(return)a\n=a";
        assert_eq!(run_source(source, "")?, "a");
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
    fn code_cannot_create_or_match_space_even_when_space_is_written_near_rules() -> TestResult {
        assert_eq!(run_source("a= ", "a ")?, " ");
        assert_eq!(run_source(" a = b ", "a")?, "b");
        Ok(())
    }

    #[test]
    fn reserved_input_bytes_are_preserved_but_not_editable_from_code() -> TestResult {
        assert_eq!(run_source("a=b", "a=()#c")?, "b=()#c");
        assert!(
            Program::parse("a=b")?
                .run("aあ".as_bytes(), RunOptions::default())
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn runtime_input_error_is_structured() -> TestResult {
        let error =
            expect_run_error(Program::parse("a=b")?.run("aあ".as_bytes(), RunOptions::default()))?;
        let error = expect_input_error(error)?;

        assert_eq!(error.column(), 2);
        Ok(())
    }

    #[test]
    fn runtime_state_can_hold_reserved_bytes_that_program_payloads_cannot_construct() -> TestResult
    {
        let program = Program::parse("a=b")?;
        assert!(Program::parse("a=(return)(").is_err());
        assert!(Program::parse("a=b)").is_err());

        let result = program.run(b"a=#()", RunOptions::new(10_000))?;
        assert_eq!(String::from_utf8(result.into_output())?, "b=#()");
        Ok(())
    }

    #[test]
    fn one_step_program_succeeds_at_exact_step_limit() -> TestResult {
        let result = Program::parse("a=b")?.run(b"a", RunOptions::new(1))?;

        assert_eq!(result.output(), b"b");
        assert_eq!(result.steps(), 1);
        assert!(!result.returned());
        Ok(())
    }

    #[test]
    fn return_program_succeeds_at_exact_step_limit() -> TestResult {
        let result = Program::parse("a=(return)b")?.run(b"a", RunOptions::new(1))?;

        assert_eq!(result.output(), b"b");
        assert_eq!(result.steps(), 1);
        assert!(result.returned());
        Ok(())
    }

    #[test]
    fn zero_step_limit_succeeds_when_no_rule_matches() -> TestResult {
        let result = Program::parse("a=b")?.run(b"x", RunOptions::new(0))?;

        assert_eq!(result.output(), b"x");
        assert_eq!(result.steps(), 0);
        assert!(!result.returned());
        Ok(())
    }

    #[test]
    fn zero_step_limit_fails_only_when_a_rule_would_apply() -> TestResult {
        let error = expect_run_error(Program::parse("a=b")?.run(b"a", RunOptions::new(0)))?;
        let error = expect_step_limit(error)?;

        assert_eq!(error.max_steps(), 0);
        assert_eq!(error.state(), b"a");
        Ok(())
    }

    #[test]
    fn zero_step_limit_blocks_return_rule_too() -> TestResult {
        let error = expect_run_error(Program::parse("a=(return)b")?.run(b"a", RunOptions::new(0)))?;
        let error = expect_step_limit(error)?;

        assert_eq!(error.max_steps(), 0);
        assert_eq!(error.state(), b"a");
        Ok(())
    }

    #[test]
    fn step_limit_error_keeps_state_as_bytes() -> TestResult {
        let error = expect_run_error(Program::parse("=a")?.run(b"", RunOptions::new(3)))?;
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
    fn runtime_output_preserves_ascii_control_bytes_from_input() -> TestResult {
        let result = Program::parse("a=b")?.run(b"a\0", RunOptions::new(1))?;
        assert_eq!(result.output(), b"b\0");
        Ok(())
    }

    #[test]
    fn internal_code_and_runtime_bytes_are_distinct_domains() -> TestResult {
        let compact = [CompactByte::new(b'a', 1)];
        let payload = Payload::parse(&compact, 1, PayloadKind::LeftSideData)?;
        let state = State::parse_input(b"a=()# ")?;

        assert_eq!(payload.bytes()[0].as_u8(), b'a');
        assert_eq!(state.bytes[0].as_u8(), b'a');
        assert_eq!(state.bytes[1].as_u8(), b'=');
        assert_eq!(state.bytes[2].as_u8(), b'(');
        assert_eq!(state.bytes[5].as_u8(), b' ');
        Ok(())
    }
}
