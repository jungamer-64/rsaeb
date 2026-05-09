use alloc::vec::Vec;
use core::convert::Infallible;

use crate::allocation::{AllocationContext, try_push, try_reserve_total_exact};
use crate::bytes::{Payload, RuntimeByte};
use crate::error::{LimitError, RunError, StateLimitContext, StateSizeError, TracedRunError};
use crate::program::{Program, RunLimits, RunResult, StepCount, StepLimit};
use crate::rule::{Action, OnceRuleSlot, PayloadView, Rule, RuleAnchor, RulePosition};
use crate::trace::{BorrowedTraceEffect, BorrowedTraceEvent, RuntimeStateView};

type NoTrace<'program> = for<'run> fn(BorrowedTraceEvent<'program, 'run>) -> Result<(), Infallible>;

#[derive(Debug, PartialEq, Eq)]
struct State {
    bytes: Vec<RuntimeByte>,
}

impl State {
    fn parse_input(input: &[u8], limits: RunLimits) -> Result<Self, RunError> {
        if input.len() > limits.state_byte_limit().get() {
            return Err(LimitError::state(
                StateLimitContext::Input,
                limits.state_byte_limit(),
                input.len(),
            )
            .into());
        }

        for (zero_based_column, byte) in input.iter().copied().enumerate() {
            RuntimeByte::parse_input(byte, zero_based_column)?;
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

    fn len(&self) -> usize {
        self.bytes.len()
    }

    fn view(&self) -> RuntimeStateView<'_> {
        RuntimeStateView::new(&self.bytes)
    }

    fn swap_with_scratch(&mut self, scratch: &mut RewriteScratch) {
        core::mem::swap(&mut self.bytes, &mut scratch.bytes);
    }

    #[cfg(test)]
    fn materialized_byte_at(&self, index: usize) -> Option<u8> {
        self.bytes.get(index).copied().map(RuntimeByte::materialize)
    }

    #[cfg(test)]
    fn byte_at_is_editable(&self, index: usize) -> Option<bool> {
        self.bytes.get(index).copied().map(RuntimeByte::is_editable)
    }

    #[cfg(test)]
    fn byte_at_is_opaque(&self, index: usize) -> Option<bool> {
        self.bytes.get(index).copied().map(RuntimeByte::is_opaque)
    }

    fn starts_with_payload(&self, payload: &Payload) -> Option<StateMatch> {
        self.matches_payload_at(0, payload)
    }

    fn ends_with_payload(&self, payload: &Payload) -> Option<StateMatch> {
        let start = self.len().checked_sub(payload.len())?;
        self.matches_payload_at(start, payload)
    }

    fn find_payload(&self, payload: &Payload) -> Option<StateMatch> {
        if payload.is_empty() {
            return StateMatch::checked(0, 0, self.len());
        }

        let first = payload.first_byte()?;
        let last_start = self.len().checked_sub(payload.len())?;

        (0..=last_start)
            .filter(|&position| {
                self.bytes
                    .get(position)
                    .copied()
                    .is_some_and(|byte| byte.matches_program_byte(first))
            })
            .find_map(|position| self.matches_payload_at(position, payload))
    }

    fn matches_payload_at(&self, position: usize, payload: &Payload) -> Option<StateMatch> {
        let state_match = StateMatch::checked(position, payload.len(), self.len())?;
        let window = self.bytes.get(state_match.position()..state_match.end())?;

        window
            .iter()
            .copied()
            .zip(payload.program_bytes().iter().copied())
            .all(|(actual, expected)| actual.matches_program_byte(expected))
            .then_some(state_match)
    }

    fn replace_at_into(
        &self,
        state_match: StateMatch,
        rhs: &Payload,
        output: &mut RewriteScratch,
        limits: RunLimits,
    ) -> Result<(), RunError> {
        self.prepare_replacement_buffer(state_match, rhs, output, limits)?;
        self.push_prefix(output, state_match)?;
        output.push_payload(rhs)?;
        self.push_suffix(output, state_match)?;
        Ok(())
    }

    fn move_start_at_into(
        &self,
        state_match: StateMatch,
        rhs: &Payload,
        output: &mut RewriteScratch,
        limits: RunLimits,
    ) -> Result<(), RunError> {
        self.prepare_replacement_buffer(state_match, rhs, output, limits)?;
        output.push_payload(rhs)?;
        self.push_prefix(output, state_match)?;
        self.push_suffix(output, state_match)?;
        Ok(())
    }

    fn move_end_at_into(
        &self,
        state_match: StateMatch,
        rhs: &Payload,
        output: &mut RewriteScratch,
        limits: RunLimits,
    ) -> Result<(), RunError> {
        self.prepare_replacement_buffer(state_match, rhs, output, limits)?;
        self.push_prefix(output, state_match)?;
        self.push_suffix(output, state_match)?;
        output.push_payload(rhs)?;
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
        output: &mut RewriteScratch,
        limits: RunLimits,
    ) -> Result<(), RunError> {
        let capacity = self.replaced_len(state_match, rhs)?;

        if capacity > limits.state_byte_limit().get() {
            return Err(LimitError::state(
                StateLimitContext::Rewrite,
                limits.state_byte_limit(),
                capacity,
            )
            .into());
        }

        output.clear_and_reserve(capacity)?;
        Ok(())
    }

    fn push_prefix(
        &self,
        output: &mut RewriteScratch,
        state_match: StateMatch,
    ) -> Result<(), crate::allocation::AllocationError> {
        output.push_existing(self.bytes.iter().copied().take(state_match.position()))
    }

    fn push_suffix(
        &self,
        output: &mut RewriteScratch,
        state_match: StateMatch,
    ) -> Result<(), crate::allocation::AllocationError> {
        output.push_existing(self.bytes.iter().copied().skip(state_match.end()))
    }

    fn materialize(
        &self,
        context: AllocationContext,
    ) -> Result<Vec<u8>, crate::allocation::AllocationError> {
        let mut output = Vec::new();
        try_reserve_total_exact(&mut output, self.len(), context)?;
        for byte in self.bytes.iter().copied() {
            try_push(&mut output, byte.materialize(), context)?;
        }
        Ok(output)
    }

    fn into_output(self) -> Result<Vec<u8>, RunError> {
        self.materialize(AllocationContext::FinalOutput)
            .map_err(RunError::from)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct RewriteScratch {
    bytes: Vec<RuntimeByte>,
}

impl RewriteScratch {
    fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    fn view(&self) -> RuntimeStateView<'_> {
        RuntimeStateView::new(&self.bytes)
    }

    fn clear_and_reserve(
        &mut self,
        capacity: usize,
    ) -> Result<(), crate::allocation::AllocationError> {
        self.bytes.clear();
        try_reserve_total_exact(&mut self.bytes, capacity, AllocationContext::RuntimeState)
    }

    fn push_existing(
        &mut self,
        source: impl IntoIterator<Item = RuntimeByte>,
    ) -> Result<(), crate::allocation::AllocationError> {
        for byte in source {
            try_push(&mut self.bytes, byte, AllocationContext::RuntimeState)?;
        }

        Ok(())
    }

    fn push_payload(
        &mut self,
        payload: &Payload,
    ) -> Result<(), crate::allocation::AllocationError> {
        self.push_existing(payload.runtime_bytes())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StateMatch {
    start: usize,
    end: usize,
}

impl StateMatch {
    fn checked(position: usize, lhs_len: usize, state_len: usize) -> Option<Self> {
        let end = position.checked_add(lhs_len)?;
        (position <= state_len && end <= state_len).then_some(Self {
            start: position,
            end,
        })
    }

    const fn position(self) -> usize {
        self.start
    }

    const fn lhs_len(self) -> usize {
        self.end - self.start
    }

    const fn end(self) -> usize {
        self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RewriteEffect<'program> {
    Continue,
    Return(PayloadView<'program>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MatchedRule<'program> {
    position: RulePosition,
    rule: &'program Rule,
    state_match: StateMatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeRuleState {
    Fresh,
    Consumed,
}

impl RuntimeRuleState {
    const fn is_consumed(self) -> bool {
        matches!(self, Self::Consumed)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct OnceRuleStates {
    states: Vec<RuntimeRuleState>,
}

impl OnceRuleStates {
    fn new(count: usize) -> Result<Self, crate::allocation::AllocationError> {
        let mut states = Vec::new();
        try_reserve_total_exact(&mut states, count, AllocationContext::RuntimeRuleState)?;

        for _ in 0..count {
            try_push(
                &mut states,
                RuntimeRuleState::Fresh,
                AllocationContext::RuntimeRuleState,
            )?;
        }

        Ok(Self { states })
    }

    fn is_available(&self, slot: Option<OnceRuleSlot>) -> bool {
        let Some(slot) = slot else {
            return true;
        };

        debug_assert!(
            slot.zero_based() < self.states.len(),
            "once rule slot must be allocated by RuleSet"
        );

        self.states
            .get(slot.zero_based())
            .copied()
            .is_some_and(|state| !state.is_consumed())
    }

    fn consume(&mut self, slot: Option<OnceRuleSlot>) {
        let Some(slot) = slot else {
            return;
        };

        debug_assert!(
            slot.zero_based() < self.states.len(),
            "once rule slot must be allocated by RuleSet"
        );

        if let Some(state) = self.states.get_mut(slot.zero_based()) {
            *state = RuntimeRuleState::Consumed;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StepBudget {
    max_steps: StepLimit,
    completed_steps: StepCount,
}

impl StepBudget {
    const fn new(max_steps: StepLimit) -> Self {
        Self {
            max_steps,
            completed_steps: StepCount::ZERO,
        }
    }

    const fn completed_steps(self) -> StepCount {
        self.completed_steps
    }

    fn ensure_next_step_allowed(self, state_len: usize) -> Result<(), LimitError> {
        if self.completed_steps.get() >= self.max_steps.get() {
            return Err(LimitError::step(
                self.max_steps,
                self.completed_steps,
                state_len,
            ));
        }

        Ok(())
    }

    fn complete_step(&mut self, state_len: usize) -> Result<StepCount, LimitError> {
        self.ensure_next_step_allowed(state_len)?;

        let Some(next_steps) = self.completed_steps.checked_next() else {
            return Err(LimitError::step(
                self.max_steps,
                self.completed_steps,
                state_len,
            ));
        };

        self.completed_steps = next_steps;
        Ok(next_steps)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Runtime<'program> {
    program: &'program Program,
    state: State,
    scratch: RewriteScratch,
    step_budget: StepBudget,
    once_states: OnceRuleStates,
    limits: RunLimits,
}

impl<'program> Runtime<'program> {
    pub(crate) fn new(
        program: &'program Program,
        input: &[u8],
        limits: RunLimits,
    ) -> Result<Self, RunError> {
        let state = State::parse_input(input, limits)?;
        let once_states = OnceRuleStates::new(program.runtime_once_rule_count())?;
        let scratch = RewriteScratch::new();

        Ok(Self {
            program,
            state,
            scratch,
            step_budget: StepBudget::new(limits.step_limit()),
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
                    self.state.into_output()?,
                    self.step_budget.completed_steps(),
                ));
            };

            self.step_budget
                .ensure_next_step_allowed(self.state.len())
                .map_err(RunError::from)?;

            self.once_states.consume(matched.rule.once_slot());

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
                state: self.state.view(),
            })
            .map_err(TracedRunError::Trace)?;
        }

        Ok(())
    }

    fn find_next_match(&self) -> Option<MatchedRule<'program>> {
        for (index, rule) in self.program.rule_slice().iter().enumerate() {
            if !self.once_states.is_available(rule.once_slot()) {
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

    fn apply_rule<F, E>(
        &mut self,
        matched: MatchedRule<'program>,
        trace: &mut Option<F>,
    ) -> Result<Option<RunResult>, TracedRunError<E>>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), E>,
    {
        let effect = self
            .apply_action_to_scratch(matched.state_match, matched.rule.action())
            .map_err(TracedRunError::Run)?;

        let step = self
            .step_budget
            .complete_step(self.state.len())
            .map_err(RunError::from)
            .map_err(TracedRunError::Run)?;

        match effect {
            RewriteEffect::Continue => {
                Self::emit_step_trace(
                    trace,
                    step,
                    matched.position,
                    matched.rule,
                    BorrowedTraceEffect::Continue {
                        state: self.scratch.view(),
                    },
                )?;
                self.state.swap_with_scratch(&mut self.scratch);
                Ok(None)
            }
            RewriteEffect::Return(output) => {
                Self::emit_step_trace(
                    trace,
                    step,
                    matched.position,
                    matched.rule,
                    BorrowedTraceEffect::Return { output },
                )?;

                Ok(Some(RunResult::from_return(
                    output
                        .to_vec_with_context(AllocationContext::ReturnOutput)
                        .map_err(RunError::from)?,
                    step,
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
                if output.len() > self.limits.return_byte_limit().get() {
                    return Err(LimitError::return_output(
                        self.limits.return_byte_limit(),
                        output.len(),
                    )
                    .into());
                }

                Ok(RewriteEffect::Return(PayloadView::new(output)))
            }
        }
    }

    fn emit_step_trace<F, E>(
        trace: &mut Option<F>,
        step: StepCount,
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
    match rule.anchor() {
        RuleAnchor::Anywhere => state.find_payload(rule.lhs()),
        RuleAnchor::Start => state.starts_with_payload(rule.lhs()),
        RuleAnchor::End => state.ends_with_payload(rule.lhs()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes::{CompactByte, Payload, ProgramByte};
    use crate::test_support::{
        TestFailure, TestResult, expect_input_error, expect_run_error, expect_step_limit,
        run_source,
    };
    use crate::{
        BorrowedTraceEffect, BorrowedTraceEvent, LimitError, PayloadKind, Program, RunLimits,
        RunTermination, SourceColumn, SourceLineNumber,
    };
    use std::string::String;
    use std::vec::Vec;

    fn expect_runtime_byte(state: &State, index: usize) -> Result<u8, TestFailure> {
        state
            .materialized_byte_at(index)
            .ok_or(TestFailure::Message("expected runtime byte"))
    }

    fn expect_payload_byte(payload: &Payload, index: usize) -> Result<u8, TestFailure> {
        payload
            .program_bytes()
            .get(index)
            .copied()
            .map(ProgramByte::get)
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
        let result = Program::parse_str(source)?.run(b"ab", RunLimits::new(StepLimit::new(2)))?;

        assert_eq!(result.output(), b"ok");
        assert_eq!(result.steps().get(), 2);
        assert_eq!(result.termination(), RunTermination::Return);
        Ok(())
    }

    #[test]
    fn empty_lhs_start_and_end_anchors_pick_different_edges() -> TestResult {
        let start_result = Program::parse_str("(once)(start)=x\nxab=(return)start")?
            .run(b"ab", RunLimits::new(StepLimit::new(2)))?;
        let end_result = Program::parse_str("(once)(end)=x\nabx=(return)end")?
            .run(b"ab", RunLimits::new(StepLimit::new(2)))?;

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
        let result =
            Program::parse_str("a=(return)x")?.run(b"a=()#c", RunLimits::new(StepLimit::new(1)))?;
        assert_eq!(result.output(), b"x");
        assert_eq!(result.termination(), RunTermination::Return);
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
    fn opaque_reserved_input_bytes_do_not_bridge_program_payload_matches() -> TestResult {
        assert_eq!(run_source("ab=x", "a=b")?, "a=b");
        assert_eq!(run_source("ab=x", "a#b")?, "a#b");
        assert_eq!(run_source("ab=x", "a(b")?, "a(b");
        assert_eq!(run_source("ab=x", "a)b")?, "a)b");
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

        let result = program.run(b"a=#()", RunLimits::new(StepLimit::new(10_000)))?;
        assert_eq!(String::from_utf8(result.into_output())?, "b=#()");
        Ok(())
    }

    #[test]
    fn step_limit_allows_exact_limit_but_blocks_next_match() -> TestResult {
        let exact = Program::parse_str("a=b")?.run(b"a", RunLimits::new(StepLimit::new(1)))?;
        assert_eq!(exact.output(), b"b");
        assert_eq!(exact.steps().get(), 1);

        let no_match = Program::parse_str("a=b")?.run(b"x", RunLimits::new(StepLimit::new(0)))?;
        assert_eq!(no_match.output(), b"x");
        assert_eq!(no_match.steps().get(), 0);

        let error = expect_run_error(
            Program::parse_str("a=b")?.run(b"a", RunLimits::new(StepLimit::new(0))),
        )?;
        let error = expect_step_limit(error)?;
        assert_eq!(
            error,
            LimitError::Step {
                max_steps: StepLimit::new(0),
                completed_steps: StepCount::ZERO,
                state_len: 1,
            },
        );
        Ok(())
    }

    #[test]
    fn step_limit_error_reports_state_len_without_owning_state_bytes() -> TestResult {
        let error = expect_run_error(
            Program::parse_str("=a")?.run(b"", RunLimits::new(StepLimit::new(3))),
        )?;
        let error = expect_step_limit(error)?;

        assert_eq!(
            error,
            LimitError::Step {
                max_steps: StepLimit::new(3),
                completed_steps: StepCount::ZERO
                    .checked_next()
                    .and_then(StepCount::checked_next)
                    .and_then(StepCount::checked_next)
                    .ok_or(TestFailure::Message("expected step count"))?,
                state_len: 3,
            },
        );
        Ok(())
    }

    #[test]
    fn borrowed_trace_exposes_last_state_before_step_limit() -> TestResult {
        let program = Program::parse_str("=a")?;
        let mut last_state = Vec::new();

        let error = expect_run_error(program.run_with_borrowed_trace(
            b"",
            RunLimits::new(StepLimit::new(3)),
            |event| {
                last_state.clear();
                match event {
                    BorrowedTraceEvent::Initial { state }
                    | BorrowedTraceEvent::Step {
                        effect: BorrowedTraceEffect::Continue { state },
                        ..
                    } => last_state.extend(state.bytes()),
                    BorrowedTraceEvent::Step {
                        effect: BorrowedTraceEffect::Return { output },
                        ..
                    } => last_state.extend(output.bytes()),
                }
            },
        ))?;
        let error = expect_step_limit(error)?;

        assert_eq!(
            error,
            LimitError::Step {
                max_steps: StepLimit::new(3),
                completed_steps: StepCount::ZERO
                    .checked_next()
                    .and_then(StepCount::checked_next)
                    .and_then(StepCount::checked_next)
                    .ok_or(TestFailure::Message("expected step count"))?,
                state_len: 3,
            },
        );
        assert_eq!(last_state, b"aaa");
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
        assert_eq!(result.steps().get(), 0);
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
        let compact = [CompactByte::new(
            b'a',
            SourceColumn::from_one_based_unchecked(1),
        )];
        let payload = Payload::parse(
            &compact,
            SourceLineNumber::from_one_based_unchecked(1),
            PayloadKind::LeftSideData,
        )?;
        let state = State::parse_input(b"a=()# ", RunLimits::default())?;

        assert_eq!(expect_payload_byte(&payload, 0)?, b'a');
        assert_eq!(expect_runtime_byte(&state, 0)?, b'a');
        assert_eq!(expect_runtime_byte(&state, 1)?, b'=');
        assert_eq!(expect_runtime_byte(&state, 2)?, b'(');
        assert_eq!(expect_runtime_byte(&state, 5)?, b' ');
        assert_eq!(state.byte_at_is_editable(0), Some(true));
        assert_eq!(state.byte_at_is_opaque(1), Some(true));
        assert_eq!(state.byte_at_is_opaque(2), Some(true));
        assert_eq!(state.byte_at_is_opaque(5), Some(true));
        Ok(())
    }
}
