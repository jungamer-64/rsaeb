use alloc::vec::Vec;
use core::convert::Infallible;

use crate::allocation::{AllocationContext, try_push, try_reserve_total_exact};
use crate::bytes::{
    Payload, PayloadByteCount, ReturnOutputByteCount, RuntimeByte, RuntimeStateByteCount,
};
use crate::error::{
    InputError, LimitError, RunError, StateLimitContext, StateSizeError, TracedRunError,
};
use crate::program::{
    Program, ReturnOutput, RunLimits, RunResult, RuntimeStateSnapshot, StepCount, StepLimit,
};
use crate::rule::{Action, PayloadView, Rule, RuleAnchor, RuleExecution, RuleView};
use crate::trace::{BorrowedTraceEffect, BorrowedTraceEvent, RuntimeStateView};

type NoTrace<'program> = for<'run> fn(BorrowedTraceEvent<'program, 'run>) -> Result<(), Infallible>;

/// Runtime input after ASCII validation and runtime-byte classification.
#[derive(Debug, PartialEq, Eq)]
pub struct RuntimeInput {
    bytes: Vec<RuntimeByte>,
}

impl RuntimeInput {
    /// Validates raw runtime input bytes.
    ///
    /// # Errors
    ///
    /// Returns `InputError::NonAscii` when `input` contains a non-ASCII byte.
    /// Returns `InputError::Allocation` when storing validated input fails.
    pub fn parse(input: &[u8]) -> Result<Self, InputError> {
        // Validate the whole boundary before allocation so input errors keep
        // precedence over allocation failures.
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

    /// Runtime input length.
    #[must_use]
    pub fn byte_count(&self) -> RuntimeStateByteCount {
        RuntimeStateByteCount::new(self.bytes.len())
    }

    /// Whether this input contains no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Runtime input bytes as a materializing iterator.
    pub fn bytes(&self) -> impl Iterator<Item = u8> + '_ {
        self.bytes.iter().copied().map(RuntimeByte::materialize)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct State {
    bytes: Vec<RuntimeByte>,
}

impl State {
    fn from_input(input: RuntimeInput, limits: RunLimits) -> Result<Self, RunError> {
        let byte_count = input.byte_count();

        if byte_count.get() > limits.state_byte_limit().get() {
            return Err(LimitError::state(
                StateLimitContext::Input,
                limits.state_byte_limit(),
                byte_count,
            )
            .into());
        }

        Ok(Self { bytes: input.bytes })
    }

    fn len(&self) -> usize {
        self.bytes.len()
    }

    fn byte_count(&self) -> RuntimeStateByteCount {
        RuntimeStateByteCount::new(self.bytes.len())
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

    fn starts_with_payload(&self, payload: &Payload) -> Option<MatchedStateSpan> {
        self.matches_payload_at(StateIndex::new(0), payload)
    }

    fn ends_with_payload(&self, payload: &Payload) -> Option<MatchedStateSpan> {
        let start = self.len().checked_sub(payload.len())?;
        self.matches_payload_at(StateIndex::new(start), payload)
    }

    fn find_payload(&self, payload: &Payload) -> Option<MatchedStateSpan> {
        if payload.is_empty() {
            return MatchedStateSpan::checked(
                StateIndex::new(0),
                payload.byte_count(),
                self.byte_count(),
            );
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
            .find_map(|position| self.matches_payload_at(StateIndex::new(position), payload))
    }

    fn matches_payload_at(
        &self,
        position: StateIndex,
        payload: &Payload,
    ) -> Option<MatchedStateSpan> {
        let state_match =
            MatchedStateSpan::checked(position, payload.byte_count(), self.byte_count())?;
        let window = self.bytes.get(state_match.start()..state_match.end())?;

        window
            .iter()
            .copied()
            .zip(payload.program_bytes().iter().copied())
            .all(|(actual, expected)| actual.matches_program_byte(expected))
            .then_some(state_match)
    }

    fn replace_at_into(
        &self,
        state_match: MatchedStateSpan,
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
        state_match: MatchedStateSpan,
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
        state_match: MatchedStateSpan,
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

    fn replaced_byte_count(
        &self,
        state_match: MatchedStateSpan,
        rhs: &Payload,
    ) -> Result<RuntimeStateByteCount, StateSizeError> {
        let state_len = self.byte_count();
        let lhs_len = state_match.matched_len();
        let rhs_len = rhs.byte_count();

        state_len
            .get()
            .checked_sub(lhs_len.get())
            .and_then(|base| base.checked_add(rhs_len.get()))
            .map(RuntimeStateByteCount::new)
            .ok_or_else(|| StateSizeError::new(state_len, lhs_len, rhs_len))
    }

    fn prepare_replacement_buffer(
        &self,
        state_match: MatchedStateSpan,
        rhs: &Payload,
        output: &mut RewriteScratch,
        limits: RunLimits,
    ) -> Result<(), RunError> {
        let capacity = self.replaced_byte_count(state_match, rhs)?;

        if capacity.get() > limits.state_byte_limit().get() {
            return Err(LimitError::state(
                StateLimitContext::Rewrite,
                limits.state_byte_limit(),
                capacity,
            )
            .into());
        }

        output.clear_and_reserve(capacity.get())?;
        Ok(())
    }

    fn push_prefix(
        &self,
        output: &mut RewriteScratch,
        state_match: MatchedStateSpan,
    ) -> Result<(), crate::allocation::AllocationError> {
        output.push_existing(self.bytes.iter().copied().take(state_match.start()))
    }

    fn push_suffix(
        &self,
        output: &mut RewriteScratch,
        state_match: MatchedStateSpan,
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

    fn into_snapshot(self) -> Result<RuntimeStateSnapshot, RunError> {
        let bytes = self
            .materialize(AllocationContext::FinalOutput)
            .map_err(RunError::from)?;
        Ok(RuntimeStateSnapshot::from_vec(bytes))
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

    fn clear_and_reserve(
        &mut self,
        capacity: usize,
    ) -> Result<(), crate::allocation::AllocationError> {
        self.bytes.clear();
        try_reserve_total_exact(
            &mut self.bytes,
            capacity,
            AllocationContext::RuntimeRewriteState,
        )
    }

    fn push_existing(
        &mut self,
        source: impl IntoIterator<Item = RuntimeByte>,
    ) -> Result<(), crate::allocation::AllocationError> {
        for byte in source {
            try_push(
                &mut self.bytes,
                byte,
                AllocationContext::RuntimeRewriteState,
            )?;
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
struct StateIndex {
    zero_based: usize,
}

impl StateIndex {
    const fn new(zero_based: usize) -> Self {
        Self { zero_based }
    }

    const fn get(self) -> usize {
        self.zero_based
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MatchedStateSpan {
    start: StateIndex,
    end: StateIndex,
    matched_len: PayloadByteCount,
}

impl MatchedStateSpan {
    fn checked(
        start: StateIndex,
        matched_len: PayloadByteCount,
        state_len: RuntimeStateByteCount,
    ) -> Option<Self> {
        let end = start.get().checked_add(matched_len.get())?;
        (start.get() <= state_len.get() && end <= state_len.get()).then_some(Self {
            start,
            end: StateIndex::new(end),
            matched_len,
        })
    }

    const fn start(self) -> usize {
        self.start.get()
    }

    const fn matched_len(self) -> PayloadByteCount {
        self.matched_len
    }

    const fn end(self) -> usize {
        self.end.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RewriteEffect<'program> {
    Continue,
    Return(PayloadView<'program>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppliedRuleEffect<'program> {
    Continue,
    Return(PayloadView<'program>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AppliedRule<'program> {
    step: StepCount,
    rule: &'program Rule,
    effect: AppliedRuleEffect<'program>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MatchedRule<'program> {
    rule: &'program Rule,
    execution: RuleExecution,
    state_match: MatchedStateSpan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnceRuleState {
    Fresh,
    Consumed,
}

#[derive(Debug, PartialEq, Eq)]
struct OnceRuleStates {
    states: Vec<OnceRuleState>,
}

impl OnceRuleStates {
    fn new(count: crate::rule::RuleCount) -> Result<Self, crate::allocation::AllocationError> {
        let mut states = Vec::new();
        try_reserve_total_exact(
            &mut states,
            count.get(),
            AllocationContext::RuntimeOnceRuleState,
        )?;

        for _ in 0..count.get() {
            try_push(
                &mut states,
                OnceRuleState::Fresh,
                AllocationContext::RuntimeOnceRuleState,
            )?;
        }

        Ok(Self { states })
    }

    #[expect(
        clippy::expect_used,
        reason = "once rule slots are assigned only by RuleSet and must have matching runtime state"
    )]
    fn is_available(&self, execution: RuleExecution) -> bool {
        match execution {
            RuleExecution::Always => true,
            RuleExecution::Once(slot) => matches!(
                self.states
                    .get(slot.get())
                    .expect("once rule slot must be allocated by RuleSet"),
                OnceRuleState::Fresh
            ),
        }
    }

    #[expect(
        clippy::expect_used,
        reason = "once rule slots are assigned only by RuleSet and must have matching runtime state"
    )]
    fn consume(&mut self, execution: RuleExecution) {
        if let RuleExecution::Once(slot) = execution {
            *self
                .states
                .get_mut(slot.get())
                .expect("once rule slot must be allocated by RuleSet") = OnceRuleState::Consumed;
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

    fn ensure_next_step_allowed(self, state_len: RuntimeStateByteCount) -> Result<(), LimitError> {
        if self.completed_steps.get() >= self.max_steps.get() {
            return Err(LimitError::step(
                self.max_steps,
                self.completed_steps,
                state_len,
            ));
        }

        Ok(())
    }

    fn complete_step(&mut self, state_len: RuntimeStateByteCount) -> Result<StepCount, LimitError> {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutionTerminal<'program> {
    Running,
    Stable,
    Return {
        step: StepCount,
        rule: &'program Rule,
        output: PayloadView<'program>,
    },
}

/// Borrowed effect of one applied execution step.
///
/// A public step only reports continuation effects here. A matching `(return)`
/// rule completes the execution and is reported through [`ExecutionCompletion`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionEffect<'run> {
    state: RuntimeStateView<'run>,
}

impl<'run> ExecutionEffect<'run> {
    const fn continue_with(state: RuntimeStateView<'run>) -> Self {
        Self { state }
    }

    /// Runtime state after the applied rewrite step.
    #[must_use]
    pub const fn state(self) -> RuntimeStateView<'run> {
        self.state
    }

    /// Runtime state length after the applied rewrite step.
    #[must_use]
    pub const fn byte_count(self) -> RuntimeStateByteCount {
        self.state.byte_count()
    }

    /// Whether the runtime state after the step contains no bytes.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.state.is_empty()
    }
}

/// Completed execution state returned by [`Execution::step`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionCompletion<'program, 'run> {
    /// No rule matched the final runtime state.
    Stable {
        /// Number of rewrite steps applied before reaching the stable state.
        steps: StepCount,
        /// Borrowed final runtime state.
        state: RuntimeStateView<'run>,
    },
    /// A matched rule executed `(return)`.
    Return {
        /// One-based applied step count for the return rule.
        step: StepCount,
        /// Structured view of the return rule.
        rule: RuleView<'program>,
        /// Borrowed return payload from the parsed program.
        output: PayloadView<'program>,
    },
}

/// Result of asking an [`Execution`] to advance by one rule application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionStep<'program, 'run> {
    /// One ordinary rewrite rule was applied and execution can be stepped again.
    Applied {
        /// One-based applied step count.
        step: StepCount,
        /// Structured view of the applied rule.
        rule: RuleView<'program>,
        /// Borrowed post-step continuation state.
        effect: ExecutionEffect<'run>,
    },
    /// The execution has completed.
    Complete(ExecutionCompletion<'program, 'run>),
}

/// Stateful execution of one parsed program against one runtime input.
///
/// An execution owns the mutable runtime state, rewrite scratch buffer,
/// completed-step budget, and per-run `(once)` state for one invocation.
#[derive(Debug, PartialEq, Eq)]
pub struct Execution<'program> {
    program: &'program Program,
    state: State,
    scratch: RewriteScratch,
    step_budget: StepBudget,
    once_states: OnceRuleStates,
    limits: RunLimits,
    terminal: ExecutionTerminal<'program>,
}

impl<'program> Execution<'program> {
    pub(crate) fn new(
        program: &'program Program,
        input: RuntimeInput,
        limits: RunLimits,
    ) -> Result<Self, RunError> {
        let state = State::from_input(input, limits)?;
        let once_states = OnceRuleStates::new(program.once_rule_count())?;
        let scratch = RewriteScratch::new();

        Ok(Self {
            program,
            state,
            scratch,
            step_budget: StepBudget::new(limits.step_limit()),
            once_states,
            limits,
            terminal: ExecutionTerminal::Running,
        })
    }

    /// Advances this execution by at most one matching rule.
    ///
    /// Returns [`ExecutionStep::Applied`] after one ordinary rewrite step.
    /// Returns [`ExecutionStep::Complete`] when no rule matches or when the
    /// next matching rule executes `(return)`.
    ///
    /// # Errors
    ///
    /// Returns `RunError` when applying the next matching rule would exceed the
    /// configured limits, allocation fails, or state-size arithmetic overflows.
    pub fn step(&mut self) -> Result<ExecutionStep<'program, '_>, RunError> {
        match self.terminal {
            ExecutionTerminal::Running => {}
            ExecutionTerminal::Stable => {
                return Ok(ExecutionStep::Complete(ExecutionCompletion::Stable {
                    steps: self.step_budget.completed_steps(),
                    state: self.state.view(),
                }));
            }
            ExecutionTerminal::Return { step, rule, output } => {
                return Ok(ExecutionStep::Complete(ExecutionCompletion::Return {
                    step,
                    rule: rule.view(),
                    output,
                }));
            }
        }

        let Some(matched) = self.find_next_match() else {
            self.terminal = ExecutionTerminal::Stable;
            return Ok(ExecutionStep::Complete(ExecutionCompletion::Stable {
                steps: self.step_budget.completed_steps(),
                state: self.state.view(),
            }));
        };

        let applied = self.apply_matched_rule(matched)?;
        match applied.effect {
            AppliedRuleEffect::Continue => Ok(ExecutionStep::Applied {
                step: applied.step,
                rule: applied.rule.view(),
                effect: ExecutionEffect::continue_with(self.state.view()),
            }),
            AppliedRuleEffect::Return(output) => {
                Ok(ExecutionStep::Complete(ExecutionCompletion::Return {
                    step: applied.step,
                    rule: applied.rule.view(),
                    output,
                }))
            }
        }
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
            match self.terminal {
                ExecutionTerminal::Running => {}
                ExecutionTerminal::Stable => {
                    return Ok(RunResult::stable(
                        self.state.into_snapshot()?,
                        self.step_budget.completed_steps(),
                    ));
                }
                ExecutionTerminal::Return { step, output, .. } => {
                    return Ok(RunResult::from_return(
                        self.materialize_return_output(output)
                            .map_err(TracedRunError::Run)?,
                        step,
                    ));
                }
            }

            let Some(matched) = self.find_next_match() else {
                return Ok(RunResult::stable(
                    self.state.into_snapshot()?,
                    self.step_budget.completed_steps(),
                ));
            };

            let applied = self
                .apply_matched_rule(matched)
                .map_err(TracedRunError::Run)?;
            match applied.effect {
                AppliedRuleEffect::Continue => {
                    Self::emit_step_trace(
                        &mut trace,
                        applied.step,
                        applied.rule,
                        BorrowedTraceEffect::Continue {
                            state: self.state.view(),
                        },
                    )?;
                }
                AppliedRuleEffect::Return(output) => {
                    Self::emit_step_trace(
                        &mut trace,
                        applied.step,
                        applied.rule,
                        BorrowedTraceEffect::Return { output },
                    )?;
                    return Ok(RunResult::from_return(
                        self.materialize_return_output(output)
                            .map_err(TracedRunError::Run)?,
                        applied.step,
                    ));
                }
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
        for rule in self.program.rule_slice() {
            let execution = rule.execution();

            if !self.once_states.is_available(execution) {
                continue;
            }

            let Some(state_match) = find_match(&self.state, rule) else {
                continue;
            };

            return Some(MatchedRule {
                rule,
                execution,
                state_match,
            });
        }

        None
    }

    fn apply_matched_rule(
        &mut self,
        matched: MatchedRule<'program>,
    ) -> Result<AppliedRule<'program>, RunError> {
        let effect = self.apply_action_to_scratch(matched.state_match, matched.rule.action())?;

        let step = self
            .step_budget
            .complete_step(self.state.byte_count())
            .map_err(RunError::from)?;

        match effect {
            RewriteEffect::Continue => {
                self.once_states.consume(matched.execution);
                self.state.swap_with_scratch(&mut self.scratch);
                Ok(AppliedRule {
                    step,
                    rule: matched.rule,
                    effect: AppliedRuleEffect::Continue,
                })
            }
            RewriteEffect::Return(output) => {
                self.once_states.consume(matched.execution);
                self.terminal = ExecutionTerminal::Return {
                    step,
                    rule: matched.rule,
                    output,
                };
                Ok(AppliedRule {
                    step,
                    rule: matched.rule,
                    effect: AppliedRuleEffect::Return(output),
                })
            }
        }
    }

    fn materialize_return_output(
        &self,
        output: PayloadView<'program>,
    ) -> Result<ReturnOutput, RunError> {
        Ok(ReturnOutput::from_vec(
            output.to_vec_with_context(AllocationContext::ReturnOutput)?,
        ))
    }

    fn apply_action_to_scratch(
        &mut self,
        state_match: MatchedStateSpan,
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
                let output_len = ReturnOutputByteCount::new(output.len());
                if output_len.get() > self.limits.return_byte_limit().get() {
                    return Err(LimitError::return_output(
                        self.limits.return_byte_limit(),
                        output_len,
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
        rule: &'program Rule,
        effect: BorrowedTraceEffect<'program, '_>,
    ) -> Result<(), TracedRunError<E>>
    where
        F: for<'run> FnMut(BorrowedTraceEvent<'program, 'run>) -> Result<(), E>,
    {
        if let Some(trace) = trace.as_mut() {
            trace(BorrowedTraceEvent::Step {
                step,
                rule: rule.view(),
                effect,
            })
            .map_err(TracedRunError::Trace)?;
        }

        Ok(())
    }
}

fn find_match(state: &State, rule: &Rule) -> Option<MatchedStateSpan> {
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
        TestFailure, TestResult, ensure, ensure_eq, ensure_matches, expect_input_error,
        expect_return_output, expect_run_error, expect_step_limit, into_result_bytes, result_bytes,
        run_program, run_source, runtime_input, source_column, source_line_number,
    };
    use crate::{
        BorrowedTraceEffect, BorrowedTraceEvent, LimitError, PayloadKind, Program, ReturnByteLimit,
        ReturnOutputByteCount, RunLimits, RuntimeStateByteCount, StateByteLimit, StateLimitContext,
    };
    use std::string::String;
    use std::vec::Vec;

    fn expect_runtime_byte(state: &State, index: usize) -> Result<u8, TestFailure> {
        state
            .materialized_byte_at(index)
            .ok_or(TestFailure::message("expected runtime byte"))
    }

    fn expect_payload_byte(payload: &Payload, index: usize) -> Result<u8, TestFailure> {
        payload
            .program_bytes()
            .get(index)
            .copied()
            .map(ProgramByte::get)
            .ok_or(TestFailure::message("expected payload byte"))
    }

    fn runtime_view_bytes(view: RuntimeStateView<'_>) -> Vec<u8> {
        view.bytes().collect()
    }

    fn expect_applied_step(
        step: ExecutionStep<'_, '_>,
        expected_step: usize,
        expected_rule: &[u8],
        expected_state: &[u8],
    ) -> TestResult {
        match step {
            ExecutionStep::Applied { step, rule, effect } => {
                ensure_eq(step.get(), expected_step)?;
                ensure_eq(rule.canonical_source()?.as_slice(), expected_rule)?;
                ensure_eq(
                    runtime_view_bytes(effect.state()).as_slice(),
                    expected_state,
                )?;
                ensure_eq(
                    effect.byte_count(),
                    RuntimeStateByteCount::new(expected_state.len()),
                )?;
                ensure_eq(effect.is_empty(), expected_state.is_empty())?;
                Ok(())
            }
            ExecutionStep::Complete(_) => Err(TestFailure::message("expected applied step")),
        }
    }

    fn expect_stable_completion(
        step: ExecutionStep<'_, '_>,
        expected_steps: usize,
        expected_state: &[u8],
    ) -> TestResult {
        match step {
            ExecutionStep::Complete(ExecutionCompletion::Stable { steps, state }) => {
                ensure_eq(steps.get(), expected_steps)?;
                ensure_eq(runtime_view_bytes(state).as_slice(), expected_state)?;
                Ok(())
            }
            ExecutionStep::Applied { .. } | ExecutionStep::Complete(_) => {
                Err(TestFailure::message("expected stable completion"))
            }
        }
    }

    fn expect_return_completion(
        step: ExecutionStep<'_, '_>,
        expected_step: usize,
        expected_rule: &[u8],
        expected_output: &[u8],
    ) -> TestResult {
        match step {
            ExecutionStep::Complete(ExecutionCompletion::Return { step, rule, output }) => {
                ensure_eq(step.get(), expected_step)?;
                ensure_eq(rule.canonical_source()?.as_slice(), expected_rule)?;
                ensure(
                    output.eq_bytes(expected_output),
                    "expected return completion output",
                )?;
                Ok(())
            }
            ExecutionStep::Applied { .. } | ExecutionStep::Complete(_) => {
                Err(TestFailure::message("expected return completion"))
            }
        }
    }

    #[test]
    fn normal_replacement_is_ordered_and_leftmost() -> TestResult {
        let source = "aa=x\na=y";
        ensure_eq(run_source(source, "aaaa")?, "xx")?;
        Ok(())
    }

    #[test]
    fn execution_step_applies_one_rule_and_waits() -> TestResult {
        let program = Program::parse_str("a=b\nb=c")?;
        let mut execution =
            program.start_execution(runtime_input(b"a")?, RunLimits::new(StepLimit::new(10)))?;

        expect_applied_step(execution.step()?, 1, b"a=b", b"b")?;
        expect_applied_step(execution.step()?, 2, b"b=c", b"c")?;
        expect_stable_completion(execution.step()?, 2, b"c")?;
        expect_stable_completion(execution.step()?, 2, b"c")?;
        Ok(())
    }

    #[test]
    fn execution_step_uses_the_same_once_state_as_full_run() -> TestResult {
        let program = Program::parse_str("(once)a=b\na=c")?;
        let limits = RunLimits::new(StepLimit::new(10));
        let full_run = program.run(runtime_input(b"aa")?, limits)?;
        let mut execution = program.start_execution(runtime_input(b"aa")?, limits)?;

        expect_applied_step(execution.step()?, 1, b"(once)a=b", b"ba")?;
        expect_applied_step(execution.step()?, 2, b"a=c", b"bc")?;
        expect_stable_completion(
            execution.step()?,
            full_run.steps().get(),
            result_bytes(&full_run),
        )?;
        Ok(())
    }

    #[test]
    fn execution_step_return_completes_without_continuation() -> TestResult {
        let program = Program::parse_str("a=(return)ok\na=b")?;
        let mut execution =
            program.start_execution(runtime_input(b"a")?, RunLimits::new(StepLimit::new(10)))?;

        expect_return_completion(execution.step()?, 1, b"a=(return)ok", b"ok")?;
        expect_return_completion(execution.step()?, 1, b"a=(return)ok", b"ok")?;
        Ok(())
    }

    #[test]
    fn execution_step_preserves_step_limit_boundary() -> TestResult {
        let program = Program::parse_str("a=b")?;
        let mut no_match =
            program.start_execution(runtime_input(b"x")?, RunLimits::new(StepLimit::new(0)))?;
        expect_stable_completion(no_match.step()?, 0, b"x")?;

        let mut would_match =
            program.start_execution(runtime_input(b"a")?, RunLimits::new(StepLimit::new(0)))?;
        let error = expect_run_error(would_match.step())?;
        let error = expect_step_limit(error)?;

        ensure_eq(
            error,
            LimitError::Step {
                max_steps: StepLimit::new(0),
                completed_steps: StepCount::ZERO,
                state_len: RuntimeStateByteCount::new(1),
            },
        )?;
        Ok(())
    }

    #[test]
    fn execution_step_preserves_byte_limit_boundaries() -> TestResult {
        let state_limits = RunLimits::bounded(
            StepLimit::new(1),
            StateByteLimit::new(2),
            ReturnByteLimit::new(10),
        );
        let state_program = Program::parse_str("=a")?;
        let mut state_limited =
            state_program.start_execution(runtime_input(b"aa")?, state_limits)?;
        let state_error = expect_run_error(state_limited.step())?;
        ensure_eq(
            state_error,
            RunError::Limit(LimitError::State {
                context: StateLimitContext::Rewrite,
                limit: StateByteLimit::new(2),
                attempted_len: RuntimeStateByteCount::new(3),
            }),
        )?;

        let return_limits = RunLimits::bounded(
            StepLimit::new(1),
            StateByteLimit::new(10),
            ReturnByteLimit::new(1),
        );
        let return_program = Program::parse_str("a=(return)ok")?;
        let mut return_limited =
            return_program.start_execution(runtime_input(b"a")?, return_limits)?;
        let return_error = expect_run_error(return_limited.step())?;
        ensure_eq(
            return_error,
            RunError::Limit(LimitError::Return {
                limit: ReturnByteLimit::new(1),
                attempted_len: ReturnOutputByteCount::new(2),
            }),
        )?;
        Ok(())
    }

    #[test]
    fn anchors_match_only_at_their_edges() -> TestResult {
        ensure_eq(run_source("(start)a=x", "aba")?, "xba")?;
        ensure_eq(run_source("(start)a=x", "ba")?, "ba")?;
        ensure_eq(run_source("(end)a=x", "aba")?, "abx")?;
        ensure_eq(run_source("(end)a=x", "ab")?, "ab")?;
        Ok(())
    }

    #[test]
    fn move_actions_work() -> TestResult {
        ensure_eq(run_source("a=(start)x", "ba")?, "xb")?;
        ensure_eq(run_source("a=(end)x", "ba")?, "bx")?;
        Ok(())
    }

    #[test]
    fn empty_lhs_anywhere_matches_at_start() -> TestResult {
        let source = "(once)=x\n(start)x=(return)ok";
        let result = run_program(
            &Program::parse_str(source)?,
            b"ab",
            RunLimits::new(StepLimit::new(2)),
        )?;

        expect_return_output(&result, b"ok")?;
        ensure_eq(result.steps().get(), 2)?;
        Ok(())
    }

    #[test]
    fn empty_lhs_start_and_end_anchors_pick_different_edges() -> TestResult {
        let limits = RunLimits::new(StepLimit::new(2));
        let start_result = run_program(
            &Program::parse_str("(once)(start)=x\nxab=(return)start")?,
            b"ab",
            limits,
        )?;
        let end_result = run_program(
            &Program::parse_str("(once)(end)=x\nabx=(return)end")?,
            b"ab",
            limits,
        )?;

        ensure_eq(result_bytes(&start_result), b"start".as_slice())?;
        ensure_eq(result_bytes(&end_result), b"end".as_slice())?;
        Ok(())
    }

    #[test]
    fn once_rule_is_used_at_most_once() -> TestResult {
        let source = "(once)a=b\na=c";
        ensure_eq(run_source(source, "aa")?, "bc")?;
        Ok(())
    }

    #[test]
    fn once_rule_lookup_does_not_consume_before_step_commit() -> TestResult {
        let program = Program::parse_str("(once)a=b")?;
        let runtime = Execution::new(
            &program,
            runtime_input(b"a")?,
            RunLimits::new(StepLimit::new(1)),
        )?;

        ensure(
            runtime.find_next_match().is_some(),
            "expected first lookup to find the once rule",
        )?;
        ensure(
            runtime.find_next_match().is_some(),
            "lookup must not consume a once rule before the step commits",
        )?;
        Ok(())
    }

    #[test]
    fn return_discards_current_state() -> TestResult {
        let source = "aa=(return)ok\na=x";
        ensure_eq(run_source(source, "aabb")?, "ok")?;
        Ok(())
    }

    #[test]
    fn runtime_only_bytes_are_preserved_until_return_discards_them() -> TestResult {
        ensure_eq(run_source("a=b", "a=()#c")?, "b=()#c")?;
        let result = run_program(
            &Program::parse_str("a=(return)x")?,
            b"a=()#c",
            RunLimits::new(StepLimit::new(1)),
        )?;
        expect_return_output(&result, b"x")?;
        Ok(())
    }

    #[test]
    fn input_spaces_are_preserved_and_do_not_bridge_matches() -> TestResult {
        ensure_eq(run_source("a= b", "a bc")?, "b bc")?;
        ensure_eq(run_source("a b=bb", "a bc")?, "a bc")?;
        ensure_eq(run_source("ab=bb", "a bc")?, "a bc")?;
        Ok(())
    }

    #[test]
    fn opaque_reserved_input_bytes_do_not_bridge_program_payload_matches() -> TestResult {
        ensure_eq(run_source("ab=x", "a=b")?, "a=b")?;
        ensure_eq(run_source("ab=x", "a#b")?, "a#b")?;
        ensure_eq(run_source("ab=x", "a(b")?, "a(b")?;
        ensure_eq(run_source("ab=x", "a)b")?, "a)b")?;
        Ok(())
    }

    #[test]
    fn runtime_input_error_is_structured() -> TestResult {
        let error = expect_input_error(RuntimeInput::parse("aあ".as_bytes()))?;

        ensure_matches(
            matches!(
                error,
                InputError::NonAscii { column, .. } if column.get() == 2
            ),
            "expected non-ASCII input error at the original column",
        )?;
        Ok(())
    }

    #[test]
    fn runtime_state_can_hold_reserved_bytes_that_program_payloads_cannot_construct() -> TestResult
    {
        let program = Program::parse_str("a=b")?;
        ensure(
            Program::parse_str("a=(return)(").is_err(),
            "expected invalid return payload",
        )?;
        ensure(
            Program::parse_str("a=b)").is_err(),
            "expected invalid payload",
        )?;

        let result = run_program(&program, b"a=#()", RunLimits::new(StepLimit::new(10_000)))?;
        ensure_eq(String::from_utf8(into_result_bytes(result))?, "b=#()")?;
        Ok(())
    }

    #[test]
    fn step_limit_allows_exact_limit_but_blocks_next_match() -> TestResult {
        let exact = run_program(
            &Program::parse_str("a=b")?,
            b"a",
            RunLimits::new(StepLimit::new(1)),
        )?;
        ensure_eq(result_bytes(&exact), b"b".as_slice())?;
        ensure_eq(exact.steps().get(), 1)?;

        let no_match = run_program(
            &Program::parse_str("a=b")?,
            b"x",
            RunLimits::new(StepLimit::new(0)),
        )?;
        ensure_eq(result_bytes(&no_match), b"x".as_slice())?;
        ensure_eq(no_match.steps().get(), 0)?;

        let limits = RunLimits::new(StepLimit::new(0));
        let error = expect_run_error(Program::parse_str("a=b")?.run(runtime_input(b"a")?, limits))?;
        let error = expect_step_limit(error)?;
        ensure_eq(
            error,
            LimitError::Step {
                max_steps: StepLimit::new(0),
                completed_steps: StepCount::ZERO,
                state_len: RuntimeStateByteCount::new(1),
            },
        )?;
        Ok(())
    }

    #[test]
    fn step_limit_error_reports_state_len_without_owning_state_bytes() -> TestResult {
        let limits = RunLimits::new(StepLimit::new(3));
        let error = expect_run_error(Program::parse_str("=a")?.run(runtime_input(b"")?, limits))?;
        let error = expect_step_limit(error)?;

        ensure_eq(
            error,
            LimitError::Step {
                max_steps: StepLimit::new(3),
                completed_steps: StepCount::ZERO
                    .checked_next()
                    .and_then(StepCount::checked_next)
                    .and_then(StepCount::checked_next)
                    .ok_or(TestFailure::message("expected step count"))?,
                state_len: RuntimeStateByteCount::new(3),
            },
        )?;
        Ok(())
    }

    #[test]
    fn borrowed_trace_exposes_last_state_before_step_limit() -> TestResult {
        let program = Program::parse_str("=a")?;
        let mut last_state = Vec::new();
        let limits = RunLimits::new(StepLimit::new(3));

        let error = expect_run_error(program.run_with_borrowed_trace(
            runtime_input(b"")?,
            limits,
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

        ensure_eq(
            error,
            LimitError::Step {
                max_steps: StepLimit::new(3),
                completed_steps: StepCount::ZERO
                    .checked_next()
                    .and_then(StepCount::checked_next)
                    .and_then(StepCount::checked_next)
                    .ok_or(TestFailure::message("expected step count"))?,
                state_len: RuntimeStateByteCount::new(3),
            },
        )?;
        ensure_eq(last_state.as_slice(), b"aaa".as_slice())?;
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

        ensure_eq(run_source(source, "aba")?, "true")?;
        ensure_eq(run_source(source, "ab")?, "false")?;
        Ok(())
    }

    #[test]
    fn runtime_accepts_every_ascii_input_byte() -> TestResult {
        let input: Vec<u8> = (0x00..=0x7f).collect();
        let result = run_program(
            &Program::parse_str("# no executable rules")?,
            &input,
            RunLimits::default(),
        )?;

        ensure_eq(result_bytes(&result), input.as_slice())?;
        ensure_eq(result.steps().get(), 0)?;
        Ok(())
    }

    #[test]
    fn runtime_rejects_every_non_ascii_input_byte() -> TestResult {
        for byte in 0x80..=0xff {
            ensure(
                RuntimeInput::parse(&[byte]).is_err(),
                "byte should be rejected",
            )?;
        }

        Ok(())
    }

    #[test]
    fn internal_code_and_runtime_bytes_are_distinct_domains() -> TestResult {
        let compact = [CompactByte::new(b'a', source_column(1)?)];
        let payload = Payload::parse(&compact, source_line_number(1)?, PayloadKind::LeftSideData)?;
        let state = State::from_input(runtime_input(b"a=()# ")?, RunLimits::default())?;

        ensure_eq(expect_payload_byte(&payload, 0)?, b'a')?;
        ensure_eq(expect_runtime_byte(&state, 0)?, b'a')?;
        ensure_eq(expect_runtime_byte(&state, 1)?, b'=')?;
        ensure_eq(expect_runtime_byte(&state, 2)?, b'(')?;
        ensure_eq(expect_runtime_byte(&state, 5)?, b' ')?;
        ensure_eq(state.byte_at_is_editable(0), Some(true))?;
        ensure_eq(state.byte_at_is_opaque(1), Some(true))?;
        ensure_eq(state.byte_at_is_opaque(2), Some(true))?;
        ensure_eq(state.byte_at_is_opaque(5), Some(true))?;
        Ok(())
    }
}
