use alloc::vec::Vec;
use core::convert::Infallible;

use crate::allocation::{copy_bytes, try_reserve_total_exact, AllocationContext, AllocationError};
use crate::bytes::{copy_runtime_bytes, push_runtime_bytes, Payload, RuntimeByte};
use crate::error::{RunError, StateSizeError, StepLimitError, TracedRunError};
use crate::program::{Program, RunResult};
use crate::rule::{Action, Anchor, Rule, RulePosition, RuntimeRuleState};
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

    pub(crate) fn matches_payload_at(&self, position: usize, payload: &Payload) -> Option<StateMatch> {
        let state_match = StateMatch::checked(position, payload.len(), self.len())?;
        let window = self.bytes.get(state_match.position()..state_match.end())?;

        window
            .iter()
            .copied()
            .zip(payload.bytes().iter().copied())
            .all(|(state_byte, code_byte)| state_byte.as_u8() == code_byte.as_u8())
            .then_some(state_match)
    }

    pub(crate) fn replace_at(&self, state_match: StateMatch, rhs: &Payload) -> Result<Self, RunError> {
        let mut bytes = self.replacement_buffer(state_match, rhs)?;
        self.push_prefix(&mut bytes, state_match)?;
        push_runtime_bytes(&mut bytes, rhs.runtime_bytes())?;
        self.push_suffix(&mut bytes, state_match)?;
        Ok(Self { bytes })
    }

    pub(crate) fn move_start_at(&self, state_match: StateMatch, rhs: &Payload) -> Result<Self, RunError> {
        let mut bytes = self.replacement_buffer(state_match, rhs)?;
        push_runtime_bytes(&mut bytes, rhs.runtime_bytes())?;
        self.push_prefix(&mut bytes, state_match)?;
        self.push_suffix(&mut bytes, state_match)?;
        Ok(Self { bytes })
    }

    pub(crate) fn move_end_at(&self, state_match: StateMatch, rhs: &Payload) -> Result<Self, RunError> {
        let mut bytes = self.replacement_buffer(state_match, rhs)?;
        self.push_prefix(&mut bytes, state_match)?;
        self.push_suffix(&mut bytes, state_match)?;
        push_runtime_bytes(&mut bytes, rhs.runtime_bytes())?;
        Ok(Self { bytes })
    }

    pub(crate) fn apply_action(
        &self,
        state_match: StateMatch,
        action: &Action,
    ) -> Result<RewriteEffect, RunError> {
        match action {
            Action::Replace(rhs) => Ok(RewriteEffect::Continue(self.replace_at(state_match, rhs)?)),
            Action::MoveStart(rhs) => {
                Ok(RewriteEffect::Continue(self.move_start_at(state_match, rhs)?))
            }
            Action::MoveEnd(rhs) => {
                Ok(RewriteEffect::Continue(self.move_end_at(state_match, rhs)?))
            }
            Action::Return(output) => Ok(RewriteEffect::Return(output.to_output()?)),
        }
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

    fn replacement_buffer(
        &self,
        state_match: StateMatch,
        rhs: &Payload,
    ) -> Result<Vec<RuntimeByte>, RunError> {
        let capacity = self.replaced_len(state_match, rhs)?;
        let mut bytes = Vec::new();
        try_reserve_total_exact(&mut bytes, capacity, AllocationContext::RuntimeState)?;
        Ok(bytes)
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
    Continue(State),
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

        Ok(Self {
            program,
            state,
            steps: 0,
            once_states,
        })
    }

    pub(crate) fn run(self, max_steps: usize) -> Result<RunResult, RunError> {
        match self.run_impl::<fn(TraceEvent<'program>) -> Result<(), Infallible>, Infallible>(
            max_steps,
            None,
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
            .state
            .apply_action(matched.state_match, &matched.rule.action)
            .map_err(TracedRunError::Run)?;

        self.steps += 1;

        match effect {
            RewriteEffect::Continue(next_state) => {
                self.emit_step_trace(
                    trace,
                    self.steps,
                    matched.position,
                    matched.rule,
                    TraceStepPayload::State(&next_state),
                )?;
                self.state = next_state;
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
                    state: state.snapshot().map_err(RunError::from)?,
                },
                TraceStepPayload::Return(output) => TraceEffect::Return {
                    output: copy_bytes(output, AllocationContext::TraceSnapshot)
                        .map_err(RunError::from)?,
                },
            };

            trace(TraceEvent::Step {
                step,
                rule: rule.info(position),
                effect,
            })
            .map_err(TracedRunError::Trace)?;
        }

        Ok(())
    }
}

fn find_match(state: &State, rule: &Rule) -> Option<StateMatch> {
    match rule.anchor {
        Anchor::Anywhere => state.find_payload(&rule.lhs),
        Anchor::Start => state.starts_with_payload(&rule.lhs),
        Anchor::End => state.ends_with_payload(&rule.lhs),
    }
}

enum TraceStepPayload<'a> {
    State(&'a State),
    Return(&'a [u8]),
}
