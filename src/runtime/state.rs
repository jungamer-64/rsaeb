use super::budget::RuntimeBudgetState;
use super::input::InitialStateBytes;
use super::rewrite::{PreparedRewrite, RewriteRequest, RewriteScratch};
use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::bytes::{
    NonEmptyPayloadNeedle, Payload, PayloadByteCount, PayloadNeedle, RuntimeByte,
    RuntimeStateByteCount,
};
use crate::error::{RunError, StateSizeError};
use crate::program::RuntimeStateSnapshot;
use crate::trace::RuntimeStateView;
use alloc::vec::Vec;

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

    pub(crate) fn commit_rewrite(
        &mut self,
        rewrite: PreparedRewrite,
        scratch: &mut RewriteScratch,
    ) {
        let previous_state = core::mem::replace(&mut self.bytes, rewrite.into_runtime_bytes());
        scratch.store_previous_state(previous_state);
    }

    pub(crate) fn starts_with_payload(&self, payload: &Payload) -> Option<MatchedStateSpan> {
        match payload.needle() {
            PayloadNeedle::Empty(needle) => {
                MatchedStateSpan::at_start(needle.byte_count(), self.byte_count())
            }
            PayloadNeedle::NonEmpty(needle) => self.matches_payload_at(StateIndex::start(), needle),
        }
    }

    pub(crate) fn ends_with_payload(&self, payload: &Payload) -> Option<MatchedStateSpan> {
        match payload.needle() {
            PayloadNeedle::Empty(needle) => {
                MatchedStateSpan::at_end(needle.byte_count(), self.byte_count())
            }
            PayloadNeedle::NonEmpty(needle) => {
                let start = StateIndex::ending_match_start(self.byte_count(), needle.byte_count())?;
                self.matches_payload_at(start, needle)
            }
        }
    }

    pub(crate) fn find_payload(&self, payload: &Payload) -> Option<MatchedStateSpan> {
        match payload.needle() {
            PayloadNeedle::Empty(needle) => {
                MatchedStateSpan::at_start(needle.byte_count(), self.byte_count())
            }
            PayloadNeedle::NonEmpty(needle) => {
                let last_start =
                    StateIndex::ending_match_start(self.byte_count(), needle.byte_count())?;

                StateSearchRange::from_start_to(last_start)
                    .filter(|&position| {
                        self.bytes
                            .get(position.get())
                            .copied()
                            .and_then(RuntimeByte::program_byte)
                            == Some(needle.first_byte())
                    })
                    .find_map(|position| self.matches_payload_at(position, needle))
            }
        }
    }

    fn matches_payload_at(
        &self,
        position: StateIndex,
        needle: NonEmptyPayloadNeedle<'_>,
    ) -> Option<MatchedStateSpan> {
        let state_match =
            MatchedStateSpan::at_position(position, needle.byte_count(), self.byte_count())?;
        state_match
            .matched_bytes(&self.bytes)
            .zip(needle.program_bytes().iter().copied())
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
        budget: RuntimeBudgetState,
    ) -> Result<PreparedRewrite, RunError> {
        self.prepare_replacement_buffer(request, output, budget)?;
        match request {
            RewriteRequest::Replace(operands) => {
                self.push_prefix(output, operands.state_match())?;
                output.push_payload(operands.rhs())?;
                self.push_suffix(output, operands.state_match())?;
            }
            RewriteRequest::MoveStart(operands) => {
                output.push_payload(operands.rhs())?;
                self.push_prefix(output, operands.state_match())?;
                self.push_suffix(output, operands.state_match())?;
            }
            RewriteRequest::MoveEnd(operands) => {
                self.push_prefix(output, operands.state_match())?;
                self.push_suffix(output, operands.state_match())?;
                output.push_payload(operands.rhs())?;
            }
        }
        Ok(output.take_prepared())
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
        budget: RuntimeBudgetState,
    ) -> Result<(), RunError> {
        let capacity = self.replaced_byte_count(request)?;

        budget.ensure_rewrite_state_len(capacity)?;

        output.clear_and_reserve(capacity)?;
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
        output.push_existing(state_match.prefix_bytes(&self.bytes))
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
        output.push_existing(state_match.suffix_bytes(&self.bytes))
    }

    /// Materializes runtime state bytes at the requested allocation site.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the output buffer cannot be allocated.
    fn materialize(&self, context: AllocationContext) -> Result<Vec<u8>, AllocationError> {
        let mut output = Vec::new();
        try_reserve_total_exact(&mut output, RequestedCapacity::new(self.len()), context)?;
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
        Ok(RuntimeStateSnapshot::from_execution_state(bytes))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StateIndex {
    zero_based: usize,
}

impl StateIndex {
    const fn start() -> Self {
        Self { zero_based: 0 }
    }

    const fn from_zero_based(zero_based: usize) -> Self {
        Self { zero_based }
    }

    fn ending_match_start(
        state_len: RuntimeStateByteCount,
        matched_len: PayloadByteCount,
    ) -> Option<Self> {
        let start = state_len.get().checked_sub(matched_len.get())?;
        Some(Self::from_zero_based(start))
    }

    fn checked_add_count(self, count: PayloadByteCount) -> Option<Self> {
        let zero_based = self.zero_based.checked_add(count.get())?;
        Some(Self { zero_based })
    }

    fn checked_next(self) -> Option<Self> {
        let zero_based = self.zero_based.checked_add(1)?;
        Some(Self { zero_based })
    }

    const fn get(self) -> usize {
        self.zero_based
    }
}

struct StateSearchRange {
    next: StateIndex,
    end: StateIndex,
    finished: bool,
}

impl StateSearchRange {
    const fn from_start_to(end: StateIndex) -> Self {
        Self {
            next: StateIndex::start(),
            end,
            finished: false,
        }
    }
}

impl Iterator for StateSearchRange {
    type Item = StateIndex;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        let current = self.next;
        if current == self.end {
            self.finished = true;
        } else if let Some(next) = self.next.checked_next() {
            self.next = next;
        } else {
            self.finished = true;
        }
        Some(current)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StateSpanRange {
    start: StateIndex,
    end: StateIndex,
}

impl StateSpanRange {
    fn at_position(
        start: StateIndex,
        matched_len: PayloadByteCount,
        state_len: RuntimeStateByteCount,
    ) -> Option<Self> {
        let end = start.checked_add_count(matched_len)?;
        (start.get() <= state_len.get() && end.get() <= state_len.get())
            .then_some(Self { start, end })
    }

    const fn prefix_end(self) -> usize {
        self.start.get()
    }

    const fn suffix_start(self) -> usize {
        self.end.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MatchedStateSpan {
    range: StateSpanRange,
    matched_len: PayloadByteCount,
}

impl MatchedStateSpan {
    pub(crate) fn at_start(
        matched_len: PayloadByteCount,
        state_len: RuntimeStateByteCount,
    ) -> Option<Self> {
        Self::at_position(StateIndex::start(), matched_len, state_len)
    }

    pub(crate) fn at_end(
        matched_len: PayloadByteCount,
        state_len: RuntimeStateByteCount,
    ) -> Option<Self> {
        let start = state_len.get().checked_sub(matched_len.get())?;
        Self::at_position(StateIndex::from_zero_based(start), matched_len, state_len)
    }

    fn at_position(
        start: StateIndex,
        matched_len: PayloadByteCount,
        state_len: RuntimeStateByteCount,
    ) -> Option<Self> {
        let range = StateSpanRange::at_position(start, matched_len, state_len)?;
        Some(Self { range, matched_len })
    }

    pub(crate) const fn matched_len(self) -> PayloadByteCount {
        self.matched_len
    }

    fn matched_bytes(self, bytes: &[RuntimeByte]) -> impl Iterator<Item = RuntimeByte> + '_ {
        bytes
            .iter()
            .copied()
            .skip(self.range.prefix_end())
            .take(self.matched_len.get())
    }

    fn prefix_bytes(self, bytes: &[RuntimeByte]) -> impl Iterator<Item = RuntimeByte> + '_ {
        bytes.iter().copied().take(self.range.prefix_end())
    }

    fn suffix_bytes(self, bytes: &[RuntimeByte]) -> impl Iterator<Item = RuntimeByte> + '_ {
        bytes.iter().copied().skip(self.range.suffix_start())
    }
}
