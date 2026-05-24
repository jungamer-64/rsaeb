use super::budget::RuntimeBudgetState;
use super::rewrite::{PreparedRewrite, RewriteScratch};
use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::bytes::{
    NonEmptyPayloadNeedle, Payload, PayloadByteCount, PayloadNeedle, RuntimeByte,
    RuntimeStateByteCount,
};
use crate::error::{RunError, StateSizeError};
use crate::input::InitialStateBytes;
use crate::program::RuntimeStateSnapshot;
use crate::rule::RewriteAction;
use crate::trace::RuntimeStateView;
use alloc::vec::Vec;

/// Mutable byte state owned by one runtime execution.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct State {
    /// Current runtime-domain bytes.
    bytes: Vec<RuntimeByte>,
}

impl State {
    /// Initializes runtime state with bytes admitted by `RunSeed`.
    pub(crate) fn from_input(input: InitialStateBytes) -> Self {
        Self {
            bytes: input.into_runtime_bytes(),
        }
    }

    /// Returns the typed byte count.
    pub(crate) fn byte_count(&self) -> RuntimeStateByteCount {
        RuntimeStateByteCount::new(self.bytes.len())
    }

    /// Borrows the runtime state as a public byte view.
    pub(crate) fn view(&self) -> RuntimeStateView<'_> {
        RuntimeStateView::new(&self.bytes)
    }

    /// Commits a prepared rewrite to the runtime state.
    pub(crate) fn commit_rewrite(
        &mut self,
        rewrite: PreparedRewrite,
        scratch: &mut RewriteScratch,
    ) {
        let previous_state = core::mem::replace(&mut self.bytes, rewrite.into_runtime_bytes());
        scratch.store_previous_state(previous_state);
    }

    /// Finds a match at the start of the current state.
    pub(crate) fn starts_with_payload(&self, payload: &Payload) -> Option<StateMatch> {
        match payload.needle() {
            PayloadNeedle::Empty(needle) => {
                StateMatch::at_start(needle.byte_count(), self.byte_count())
            }
            PayloadNeedle::NonEmpty(needle) => self.matches_payload_at(StateIndex::start(), needle),
        }
    }

    /// Finds a match at the end of the current state.
    pub(crate) fn ends_with_payload(&self, payload: &Payload) -> Option<StateMatch> {
        match payload.needle() {
            PayloadNeedle::Empty(needle) => {
                StateMatch::at_end(needle.byte_count(), self.byte_count())
            }
            PayloadNeedle::NonEmpty(needle) => {
                let start = StateIndex::ending_match_start(self.byte_count(), needle.byte_count())?;
                self.matches_payload_at(start, needle)
            }
        }
    }

    /// Finds the leftmost match in the current state.
    pub(crate) fn find_payload(&self, payload: &Payload) -> Option<StateMatch> {
        match payload.needle() {
            PayloadNeedle::Empty(needle) => {
                StateMatch::at_start(needle.byte_count(), self.byte_count())
            }
            PayloadNeedle::NonEmpty(needle) => {
                let last_start =
                    StateIndex::ending_match_start(self.byte_count(), needle.byte_count())?;

                for position in StateSearchRange::from_start_to(last_start) {
                    let first_byte_matches = self
                        .bytes
                        .get(position.get())
                        .copied()
                        .and_then(RuntimeByte::program_byte)
                        == Some(needle.first_byte());
                    if !first_byte_matches {
                        continue;
                    }

                    if let Some(state_match) = self.matches_payload_at(position, needle) {
                        return Some(state_match);
                    }
                }
                None
            }
        }
    }

    /// Checks whether a non-empty payload matches at a concrete state index.
    fn matches_payload_at(
        &self,
        position: StateIndex,
        needle: NonEmptyPayloadNeedle<'_>,
    ) -> Option<StateMatch> {
        let state_match =
            StateMatch::at_position(position, needle.byte_count(), self.byte_count())?;
        let matches = state_match
            .matched_bytes(&self.bytes)
            .zip(needle.program_bytes().iter().copied())
            .all(|(actual, expected)| actual.program_byte() == Some(expected));
        matches.then_some(state_match)
    }

    /// Rewrites this state into scratch storage according to `request`.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if replacement size arithmetic overflows, the
    /// rewritten state exceeds limits, or scratch allocation fails.
    pub(crate) fn rewrite_into(
        &self,
        state_match: StateMatch,
        action: &RewriteAction,
        output: &mut RewriteScratch,
        budget: RuntimeBudgetState,
    ) -> Result<PreparedRewrite, RunError> {
        self.prepare_replacement_buffer(state_match, action.payload(), output, budget)?;
        match action {
            RewriteAction::Replace(rhs) => {
                output.push_existing(state_match.prefix_bytes(&self.bytes))?;
                output.push_payload(rhs)?;
                output.push_existing(state_match.suffix_bytes(&self.bytes))?;
            }
            RewriteAction::MoveStart(rhs) => {
                output.push_payload(rhs)?;
                output.push_existing(state_match.prefix_bytes(&self.bytes))?;
                output.push_existing(state_match.suffix_bytes(&self.bytes))?;
            }
            RewriteAction::MoveEnd(rhs) => {
                output.push_existing(state_match.prefix_bytes(&self.bytes))?;
                output.push_existing(state_match.suffix_bytes(&self.bytes))?;
                output.push_payload(rhs)?;
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
        state_match: StateMatch,
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

    /// Clears and reserves scratch storage for one rewrite.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if replacement size arithmetic overflows, the
    /// rewritten state exceeds limits, or scratch allocation fails.
    fn prepare_replacement_buffer(
        &self,
        state_match: StateMatch,
        rhs: &Payload,
        output: &mut RewriteScratch,
        budget: RuntimeBudgetState,
    ) -> Result<(), RunError> {
        let capacity = self.replaced_byte_count(state_match, rhs)?;

        budget.ensure_rewrite_state_len(capacity)?;

        output.clear_and_reserve(capacity)?;
        Ok(())
    }

    /// Materializes runtime state bytes at the requested allocation site.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the output buffer cannot be allocated.
    fn materialize(&self, context: AllocationContext) -> Result<Vec<u8>, AllocationError> {
        let mut output = Vec::new();
        try_reserve_total_exact(
            &mut output,
            RequestedCapacity::from_runtime_state_count(self.byte_count()),
            context,
        )?;
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
        Ok(RuntimeStateSnapshot::from_materialized(bytes))
    }
}

/// Zero-based index into runtime state bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StateIndex {
    /// Zero-based state byte position.
    zero_based: usize,
}

impl StateIndex {
    /// Returns the first runtime-state index.
    const fn start() -> Self {
        Self { zero_based: 0 }
    }

    /// Builds an index from a zero-based offset.
    const fn from_zero_based(zero_based: usize) -> Self {
        Self { zero_based }
    }

    /// Returns the start index for an end-anchored match.
    fn ending_match_start(
        state_len: RuntimeStateByteCount,
        matched_len: PayloadByteCount,
    ) -> Option<Self> {
        let start = state_len.get().checked_sub(matched_len.get())?;
        Some(Self::from_zero_based(start))
    }

    /// Returns the checked add count result.
    fn checked_add_count(self, count: PayloadByteCount) -> Option<Self> {
        let zero_based = self.zero_based.checked_add(count.get())?;
        Some(Self { zero_based })
    }

    /// Returns the checked next result.
    fn checked_next(self) -> Option<Self> {
        let zero_based = self.zero_based.checked_add(1)?;
        Some(Self { zero_based })
    }

    /// Returns the primitive stored value.
    const fn get(self) -> usize {
        self.zero_based
    }
}

/// Iterator over candidate runtime-state match positions.
struct StateSearchRange {
    /// Cursor describing the remaining search range.
    cursor: StateSearchCursor,
}

/// Search cursor state for runtime-state matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StateSearchCursor {
    /// Search still has candidate match positions to inspect.
    Active {
        /// Next candidate start index to inspect.
        next: StateIndex,
        /// Exclusive end index for the search range.
        end: StateIndex,
    },
    /// No candidate positions remain.
    Done,
}

impl StateSearchRange {
    /// Starts a search range ending at the supplied index.
    const fn from_start_to(end: StateIndex) -> Self {
        Self {
            cursor: StateSearchCursor::Active {
                next: StateIndex::start(),
                end,
            },
        }
    }
}

impl Iterator for StateSearchRange {
    type Item = StateIndex;

    fn next(&mut self) -> Option<Self::Item> {
        let StateSearchCursor::Active { next, end } = self.cursor else {
            return None;
        };

        let current = next;
        if current == end {
            self.cursor = StateSearchCursor::Done;
        } else if let Some(next) = next.checked_next() {
            self.cursor = StateSearchCursor::Active { next, end };
        } else {
            self.cursor = StateSearchCursor::Done;
        }
        Some(current)
    }
}

/// Half-open runtime-state span for a matched payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StateSpanRange {
    /// Inclusive start index of the span.
    start: StateIndex,
    /// Exclusive end index of the span.
    end: StateIndex,
    /// Length of the payload matched by this span.
    matched_len: PayloadByteCount,
}

impl StateSpanRange {
    /// Builds a match span at a candidate position.
    fn at_position(
        start: StateIndex,
        matched_len: PayloadByteCount,
        state_len: RuntimeStateByteCount,
    ) -> Option<Self> {
        let end = start.checked_add_count(matched_len)?;
        (start.get() <= state_len.get() && end.get() <= state_len.get()).then_some(Self {
            start,
            end,
            matched_len,
        })
    }

    /// Returns the inclusive start offset.
    const fn start(self) -> usize {
        self.start.get()
    }

    /// Returns the exclusive end offset.
    const fn end(self) -> usize {
        self.end.get()
    }

    /// Returns the typed byte count.
    fn byte_count(self) -> PayloadByteCount {
        self.matched_len
    }
}

/// Typed runtime-state match span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StateMatch {
    /// Runtime-state range covered by the match.
    range: StateSpanRange,
}

impl StateMatch {
    /// Builds a start-anchored match span.
    fn at_start(matched_len: PayloadByteCount, state_len: RuntimeStateByteCount) -> Option<Self> {
        Self::at_position(StateIndex::start(), matched_len, state_len)
    }

    /// Builds an end-anchored match span.
    fn at_end(matched_len: PayloadByteCount, state_len: RuntimeStateByteCount) -> Option<Self> {
        let start = state_len.get().checked_sub(matched_len.get())?;
        Self::at_position(StateIndex::from_zero_based(start), matched_len, state_len)
    }

    /// Builds a match span at a candidate position.
    fn at_position(
        start: StateIndex,
        matched_len: PayloadByteCount,
        state_len: RuntimeStateByteCount,
    ) -> Option<Self> {
        let range = StateSpanRange::at_position(start, matched_len, state_len)?;
        Some(Self { range })
    }

    /// Returns the matched payload length.
    pub(crate) fn matched_len(self) -> PayloadByteCount {
        self.range.byte_count()
    }

    /// Iterates over bytes before this match witness.
    fn prefix_bytes(self, bytes: &[RuntimeByte]) -> impl Iterator<Item = RuntimeByte> + '_ {
        bytes.iter().copied().take(self.range.start())
    }

    /// Iterates over the bytes covered by this match witness.
    fn matched_bytes(self, bytes: &[RuntimeByte]) -> impl Iterator<Item = RuntimeByte> + '_ {
        bytes
            .iter()
            .copied()
            .skip(self.range.start())
            .take(self.range.byte_count().get())
    }

    /// Iterates over bytes after this match witness.
    fn suffix_bytes(self, bytes: &[RuntimeByte]) -> impl Iterator<Item = RuntimeByte> + '_ {
        bytes.iter().copied().skip(self.range.end())
    }
}
