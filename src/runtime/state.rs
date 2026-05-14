use alloc::vec::Vec;

use super::input::InitialStateBytes;
use super::rewrite::RewriteScratch;
use crate::allocation::{AllocationContext, AllocationError, try_push, try_reserve_total_exact};
use crate::bytes::{Payload, PayloadByteCount, RuntimeByte, RuntimeStateByteCount};
use crate::error::{LimitError, RunError, StateLimitContext, StateSizeError};
use crate::program::{RunLimits, RuntimeStateSnapshot};
use crate::trace::RuntimeStateView;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct State {
    bytes: Vec<RuntimeByte>,
}

impl State {
    pub(super) fn from_input(input: InitialStateBytes) -> Self {
        Self { bytes: input.bytes }
    }

    pub(super) fn len(&self) -> usize {
        self.bytes.len()
    }

    pub(super) fn byte_count(&self) -> RuntimeStateByteCount {
        RuntimeStateByteCount::new(self.bytes.len())
    }

    pub(super) fn view(&self) -> RuntimeStateView<'_> {
        RuntimeStateView::new(&self.bytes)
    }

    pub(super) fn swap_with_scratch(&mut self, scratch: &mut RewriteScratch) {
        core::mem::swap(&mut self.bytes, &mut scratch.bytes);
    }

    #[cfg(test)]
    pub(super) fn materialized_byte_at(&self, index: usize) -> Option<u8> {
        self.bytes.get(index).copied().map(RuntimeByte::materialize)
    }

    pub(super) fn starts_with_payload(&self, payload: &Payload) -> Option<MatchedStateSpan> {
        self.matches_payload_at(StateIndex::new(0), payload)
    }

    pub(super) fn ends_with_payload(&self, payload: &Payload) -> Option<MatchedStateSpan> {
        let start = self.len().checked_sub(payload.len())?;
        self.matches_payload_at(StateIndex::new(start), payload)
    }

    pub(super) fn find_payload(&self, payload: &Payload) -> Option<MatchedStateSpan> {
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
                    .and_then(RuntimeByte::program_byte)
                    == Some(first)
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
            .all(|(actual, expected)| actual.program_byte() == Some(expected))
            .then_some(state_match)
    }

    pub(super) fn replace_at_into(
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

    pub(super) fn move_start_at_into(
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

    pub(super) fn move_end_at_into(
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
    ) -> Result<(), AllocationError> {
        output.push_existing(self.bytes.iter().copied().take(state_match.start()))
    }

    fn push_suffix(
        &self,
        output: &mut RewriteScratch,
        state_match: MatchedStateSpan,
    ) -> Result<(), AllocationError> {
        output.push_existing(self.bytes.iter().copied().skip(state_match.end()))
    }

    fn materialize(&self, context: AllocationContext) -> Result<Vec<u8>, AllocationError> {
        let mut output = Vec::new();
        try_reserve_total_exact(&mut output, self.len(), context)?;
        for byte in self.bytes.iter().copied() {
            try_push(&mut output, byte.materialize(), context)?;
        }
        Ok(output)
    }

    pub(super) fn into_snapshot(self) -> Result<RuntimeStateSnapshot, RunError> {
        let bytes = self
            .materialize(AllocationContext::FinalOutput)
            .map_err(RunError::from)?;
        Ok(RuntimeStateSnapshot::from_vec(bytes))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct StateIndex {
    zero_based: usize,
}

impl StateIndex {
    pub(super) const fn new(zero_based: usize) -> Self {
        Self { zero_based }
    }

    const fn get(self) -> usize {
        self.zero_based
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MatchedStateSpan {
    start: StateIndex,
    end: StateIndex,
    matched_len: PayloadByteCount,
}

impl MatchedStateSpan {
    pub(super) fn checked(
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

    pub(super) const fn start(self) -> usize {
        self.start.get()
    }

    pub(super) const fn matched_len(self) -> PayloadByteCount {
        self.matched_len
    }

    pub(super) const fn end(self) -> usize {
        self.end.get()
    }
}
