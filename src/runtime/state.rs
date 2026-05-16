use alloc::vec::Vec;

use super::input::InitialStateBytes;
use super::rewrite::{RewritePlacement, RewriteRequest, RewriteScratch};
use crate::allocation::{AllocationContext, AllocationError, try_push, try_reserve_total_exact};
use crate::bytes::{Payload, PayloadByteCount, RuntimeByte, RuntimeStateByteCount};
use crate::error::{LimitError, RunError, StateLimitContext, StateSizeError};
use crate::program::{RunLimits, RuntimeStateSnapshot};
use crate::trace::RuntimeStateView;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct State {
    bytes: Vec<RuntimeByte>,
}

impl State {
    pub(crate) fn from_input(input: InitialStateBytes) -> Self {
        Self {
            bytes: input.into_runtime_bytes(),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn byte_count(&self) -> RuntimeStateByteCount {
        RuntimeStateByteCount::new(self.bytes.len())
    }

    pub(crate) fn view(&self) -> RuntimeStateView<'_> {
        RuntimeStateView::new(&self.bytes)
    }

    pub(crate) fn swap_with_scratch(&mut self, scratch: &mut RewriteScratch) {
        scratch.swap_with_state_bytes(&mut self.bytes);
    }

    pub(crate) fn starts_with_payload(&self, payload: &Payload) -> Option<MatchedStateSpan> {
        self.matches_payload_at(StateIndex::new(0), payload)
    }

    pub(crate) fn ends_with_payload(&self, payload: &Payload) -> Option<MatchedStateSpan> {
        let start = self.len().checked_sub(payload.len())?;
        self.matches_payload_at(StateIndex::new(start), payload)
    }

    pub(crate) fn find_payload(&self, payload: &Payload) -> Option<MatchedStateSpan> {
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
        let window = self
            .bytes
            .get(state_match.start.get()..state_match.end.get())?;

        window
            .iter()
            .copied()
            .zip(payload.program_bytes().iter().copied())
            .all(|(actual, expected)| actual.program_byte() == Some(expected))
            .then_some(state_match)
    }

    /// Rewrites this state into scratch storage according to `request`.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if replacement size arithmetic overflows, the
    /// rewritten state exceeds limits, or scratch allocation fails.
    pub(crate) fn rewrite_into(
        &self,
        request: RewriteRequest<'_>,
        output: &mut RewriteScratch,
        limits: RunLimits,
    ) -> Result<(), RunError> {
        self.prepare_replacement_buffer(request, output, limits)?;
        match request.placement() {
            RewritePlacement::Replace => {
                self.push_prefix(output, request.state_match())?;
                output.push_payload(request.rhs())?;
                self.push_suffix(output, request.state_match())?;
            }
            RewritePlacement::MoveStart => {
                output.push_payload(request.rhs())?;
                self.push_prefix(output, request.state_match())?;
                self.push_suffix(output, request.state_match())?;
            }
            RewritePlacement::MoveEnd => {
                self.push_prefix(output, request.state_match())?;
                self.push_suffix(output, request.state_match())?;
                output.push_payload(request.rhs())?;
            }
        }
        Ok(())
    }

    /// Computes the rewritten state length for a rewrite request.
    ///
    /// # Errors
    ///
    /// Returns `StateSizeError` if removing the match and adding the payload
    /// cannot be represented as a runtime state byte count.
    fn replaced_byte_count(
        &self,
        request: RewriteRequest<'_>,
    ) -> Result<RuntimeStateByteCount, StateSizeError> {
        let state_len = self.byte_count();
        let lhs_len = request.state_match().matched_len();
        let rhs_len = request.rhs().byte_count();

        state_len
            .get()
            .checked_sub(lhs_len.get())
            .and_then(|base| base.checked_add(rhs_len.get()))
            .map(RuntimeStateByteCount::new)
            .ok_or_else(|| StateSizeError::new(state_len, lhs_len, rhs_len))
    }

    /// Clears and reserves scratch storage for one rewrite.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if replacement size arithmetic overflows, the
    /// rewritten state exceeds limits, or scratch allocation fails.
    fn prepare_replacement_buffer(
        &self,
        request: RewriteRequest<'_>,
        output: &mut RewriteScratch,
        limits: RunLimits,
    ) -> Result<(), RunError> {
        let capacity = self.replaced_byte_count(request)?;

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

    /// Copies bytes before the matched span into scratch storage.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if scratch storage cannot grow.
    fn push_prefix(
        &self,
        output: &mut RewriteScratch,
        state_match: MatchedStateSpan,
    ) -> Result<(), AllocationError> {
        output.push_existing(self.bytes.iter().copied().take(state_match.start.get()))
    }

    /// Copies bytes after the matched span into scratch storage.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if scratch storage cannot grow.
    fn push_suffix(
        &self,
        output: &mut RewriteScratch,
        state_match: MatchedStateSpan,
    ) -> Result<(), AllocationError> {
        output.push_existing(self.bytes.iter().copied().skip(state_match.end.get()))
    }

    /// Materializes runtime state bytes at the requested allocation site.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the output buffer cannot be allocated.
    fn materialize(&self, context: AllocationContext) -> Result<Vec<u8>, AllocationError> {
        let mut output = Vec::new();
        try_reserve_total_exact(&mut output, self.len(), context)?;
        for byte in self.bytes.iter().copied() {
            try_push(&mut output, byte.materialize(), context)?;
        }
        Ok(output)
    }

    /// Materializes this state as a public runtime-state snapshot.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if final output allocation fails.
    pub(crate) fn into_snapshot(self) -> Result<RuntimeStateSnapshot, RunError> {
        let bytes = self
            .materialize(AllocationContext::FinalOutput)
            .map_err(RunError::from)?;
        Ok(RuntimeStateSnapshot::from_vec(bytes))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StateIndex {
    zero_based: usize,
}

impl StateIndex {
    pub(crate) const fn new(zero_based: usize) -> Self {
        Self { zero_based }
    }

    const fn get(self) -> usize {
        self.zero_based
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MatchedStateSpan {
    start: StateIndex,
    end: StateIndex,
    matched_len: PayloadByteCount,
}

impl MatchedStateSpan {
    pub(crate) fn checked(
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

    pub(crate) const fn matched_len(self) -> PayloadByteCount {
        self.matched_len
    }
}
