use super::budget::{RuntimeBudgetState, StepReservation};
use super::rewrite::{PreparedRewrite, RewriteScratch};
use crate::allocation::AllocationError;
use crate::bytes::{
    EmptyPayloadNeedle, NonEmptyPayloadNeedle, Payload, PayloadByteCount, PayloadNeedle,
    RuntimeByte, RuntimeStateByteCount,
};
use crate::error::{RewriteSizeError, RunStepError};
use crate::input::InitialStateBytes;
use crate::policy::ExecutionPolicy;
use crate::program::RuntimeStateSnapshot;
use crate::rule::{RewriteAction, RuleAnchorSyntax};
use crate::trace::RuntimeStateView;
use alloc::vec::Vec;

/// Mutable byte state owned by one runtime execution.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct State {
    /// Current runtime-domain bytes.
    bytes: Vec<RuntimeByte>,
}

/// Result of comparing a parsed payload shape with runtime state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatePayloadMatch<'state> {
    /// The payload matched and carries the matched state span.
    Matched(StateMatch<'state>),
    /// The payload did not match the runtime state.
    Mismatched,
}

impl State {
    /// Initializes runtime state with bytes admitted by `AdmittedRun`.
    pub(crate) fn from_input(input: InitialStateBytes) -> Self {
        let (bytes, _permit) = input.into_runtime_bytes();
        Self { bytes }
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

    /// Compares a parsed payload with this runtime state under one rule anchor.
    pub(crate) fn match_payload(
        &self,
        anchor: RuleAnchorSyntax,
        payload: &Payload,
    ) -> StatePayloadMatch<'_> {
        match payload.needle() {
            PayloadNeedle::Empty(needle) => self.match_empty_payload(anchor, needle),
            PayloadNeedle::NonEmpty(needle) => self.match_non_empty_payload(anchor, needle),
        }
    }

    /// Matches an empty payload with anchor-specific zero-length placement.
    fn match_empty_payload(
        &self,
        anchor: RuleAnchorSyntax,
        needle: EmptyPayloadNeedle<'_>,
    ) -> StatePayloadMatch<'_> {
        let range = match anchor {
            RuleAnchorSyntax::Anywhere | RuleAnchorSyntax::Start => {
                StateSpanRange::empty_at_start(needle.byte_count())
            }
            RuleAnchorSyntax::End => {
                StateSpanRange::empty_at_end(needle.byte_count(), self.byte_count())
            }
        };
        StatePayloadMatch::Matched(StateMatch::from_range(range, &self.bytes))
    }

    /// Matches a non-empty payload through checked candidate spans.
    fn match_non_empty_payload(
        &self,
        anchor: RuleAnchorSyntax,
        needle: NonEmptyPayloadNeedle<'_>,
    ) -> StatePayloadMatch<'_> {
        match anchor {
            RuleAnchorSyntax::Anywhere => self.find_non_empty_payload(needle),
            RuleAnchorSyntax::Start => self.match_non_empty_at(StateIndex::start(), needle),
            RuleAnchorSyntax::End => {
                match StateIndex::ending_match_start(self.byte_count(), needle.byte_count()) {
                    CandidateStart::InBounds(start) => self.match_non_empty_at(start, needle),
                    CandidateStart::OutOfBounds => StatePayloadMatch::Mismatched,
                }
            }
        }
    }

    /// Finds the leftmost match for a non-empty payload.
    fn find_non_empty_payload(&self, needle: NonEmptyPayloadNeedle<'_>) -> StatePayloadMatch<'_> {
        let last_start =
            match StateIndex::ending_match_start(self.byte_count(), needle.byte_count()) {
                CandidateStart::InBounds(start) => start,
                CandidateStart::OutOfBounds => return StatePayloadMatch::Mismatched,
            };

        for position in StateSearchRange::from_start_to(last_start) {
            if let StatePayloadMatch::Matched(state_match) =
                self.match_non_empty_at(position, needle)
            {
                return StatePayloadMatch::Matched(state_match);
            }
        }

        StatePayloadMatch::Mismatched
    }

    /// Checks whether a non-empty payload matches at a candidate state index.
    fn match_non_empty_at(
        &self,
        position: StateIndex,
        needle: NonEmptyPayloadNeedle<'_>,
    ) -> StatePayloadMatch<'_> {
        match StateSpanRange::candidate_at(position, needle.byte_count(), self.byte_count()) {
            StateSpanCandidate::InBounds(range) => self.match_candidate_range(range, needle),
            StateSpanCandidate::OutOfBounds => StatePayloadMatch::Mismatched,
        }
    }

    /// Compares bytes within a candidate span that is already proven in-bounds.
    fn match_candidate_range(
        &self,
        range: StateSpanRange,
        needle: NonEmptyPayloadNeedle<'_>,
    ) -> StatePayloadMatch<'_> {
        let state_match = StateMatch::from_range(range, &self.bytes);
        let first_byte_matches = state_match.matched_bytes().next().is_some_and(|actual| {
            actual
                .projection()
                .matches_program_byte(needle.first_byte())
        });
        if !first_byte_matches {
            return StatePayloadMatch::Mismatched;
        }

        let matches = state_match
            .matched_bytes()
            .zip(needle.program_bytes().iter().copied())
            .all(|(actual, expected)| actual.projection().matches_program_byte(expected));
        if matches {
            StatePayloadMatch::Matched(state_match)
        } else {
            StatePayloadMatch::Mismatched
        }
    }

    /// Materializes this state as a public runtime-state snapshot.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if final output allocation fails.
    pub(crate) fn into_snapshot(self) -> Result<RuntimeStateSnapshot, AllocationError> {
        RuntimeStateSnapshot::from_final_state_view(self.view())
    }
}

/// Zero-based index into runtime state bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StateIndex {
    /// Zero-based state byte position.
    zero_based: usize,
}

/// Result of deriving a state index for a candidate match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CandidateStart {
    /// The candidate start is in the runtime state's addressable range.
    InBounds(StateIndex),
    /// The payload cannot fit at the requested anchor.
    OutOfBounds,
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
    ) -> CandidateStart {
        match state_len.get().checked_sub(matched_len.get()) {
            Some(start) => CandidateStart::InBounds(Self::from_zero_based(start)),
            None => CandidateStart::OutOfBounds,
        }
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

/// Candidate runtime span derived before byte comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StateSpanCandidate {
    /// Candidate span is fully inside the runtime state.
    InBounds(StateSpanRange),
    /// Candidate span would point outside the runtime state.
    OutOfBounds,
}

impl StateSpanRange {
    /// Builds a zero-length start-anchored match span.
    fn empty_at_start(matched_len: PayloadByteCount) -> Self {
        debug_assert!(matched_len.is_zero());
        Self {
            start: StateIndex::start(),
            end: StateIndex::start(),
            matched_len,
        }
    }

    /// Builds a zero-length end-anchored match span.
    fn empty_at_end(matched_len: PayloadByteCount, state_len: RuntimeStateByteCount) -> Self {
        debug_assert!(matched_len.is_zero());
        let end = StateIndex::from_zero_based(state_len.get());
        Self {
            start: end,
            end,
            matched_len,
        }
    }

    /// Classifies a candidate match span at a concrete position.
    fn candidate_at(
        start: StateIndex,
        matched_len: PayloadByteCount,
        state_len: RuntimeStateByteCount,
    ) -> StateSpanCandidate {
        let Some(end) = start.checked_add_count(matched_len) else {
            return StateSpanCandidate::OutOfBounds;
        };
        if start.get() <= state_len.get() && end.get() <= state_len.get() {
            StateSpanCandidate::InBounds(Self {
                start,
                end,
                matched_len,
            })
        } else {
            StateSpanCandidate::OutOfBounds
        }
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
pub(crate) struct StateMatch<'state> {
    /// Runtime-state range covered by the match.
    range: StateSpanRange,
    /// Runtime-state bytes proven to own this match range.
    bytes: &'state [RuntimeByte],
}

impl<'state> StateMatch<'state> {
    /// Builds a matched-state witness from a validated runtime span.
    fn from_range(range: StateSpanRange, bytes: &'state [RuntimeByte]) -> Self {
        Self { range, bytes }
    }

    /// Iterates over bytes covered by this freshly built match witness.
    fn matched_bytes(self) -> impl Iterator<Item = RuntimeByte> + 'state {
        self.bytes
            .iter()
            .copied()
            .skip(self.range.start())
            .take(self.range.byte_count().get())
    }
}

impl<'state> StateMatch<'state> {
    /// Rewrites this matched state into scratch storage according to `request`.
    ///
    /// # Errors
    ///
    /// Returns `RunStepError` if replacement size arithmetic overflows, the
    /// rewritten state exceeds limits, or scratch allocation fails.
    pub(crate) fn rewrite_into<E: ExecutionPolicy>(
        self,
        action: &RewriteAction,
        output: &mut RewriteScratch,
        _step: &StepReservation<'_, E>,
    ) -> Result<PreparedRewrite, RunStepError> {
        self.prepare_replacement_buffer::<E>(action.payload(), output)?;
        match action {
            RewriteAction::Replace(rhs) => {
                output.push_existing(self.prefix_bytes())?;
                output.push_payload(rhs)?;
                output.push_existing(self.suffix_bytes())?;
            }
            RewriteAction::MoveStart(rhs) => {
                output.push_payload(rhs)?;
                output.push_existing(self.prefix_bytes())?;
                output.push_existing(self.suffix_bytes())?;
            }
            RewriteAction::MoveEnd(rhs) => {
                output.push_existing(self.prefix_bytes())?;
                output.push_existing(self.suffix_bytes())?;
                output.push_payload(rhs)?;
            }
        }
        Ok(output.take_prepared())
    }

    /// Computes the rewritten state length for a rewrite request.
    ///
    /// # Errors
    ///
    /// Returns `RewriteSizeError` if removing the match and adding the payload
    /// cannot be represented as a runtime state byte count.
    fn replaced_byte_count(self, rhs: &Payload) -> Result<RuntimeStateByteCount, RewriteSizeError> {
        let state_len = RuntimeStateByteCount::new(self.bytes.len());
        let lhs_len = self.matched_len();
        let rhs_len = rhs.byte_count();

        state_len
            .get()
            .checked_sub(lhs_len.get())
            .and_then(|base| base.checked_add(rhs_len.get()))
            .map(RuntimeStateByteCount::new)
            .ok_or_else(|| RewriteSizeError::new(state_len, lhs_len, rhs_len))
    }

    /// Clears and reserves scratch storage for one rewrite.
    ///
    /// # Errors
    ///
    /// Returns `RunStepError` if replacement size arithmetic overflows, the
    /// rewritten state exceeds limits, or scratch allocation fails.
    fn prepare_replacement_buffer<E: ExecutionPolicy>(
        self,
        rhs: &Payload,
        output: &mut RewriteScratch,
    ) -> Result<(), RunStepError> {
        let capacity = self.replaced_byte_count(rhs)?;

        let capacity_permit = RuntimeBudgetState::<E>::ensure_rewrite_state_len(capacity)?;

        output.clear_and_reserve(capacity_permit)?;
        Ok(())
    }

    /// Returns the matched payload length.
    fn matched_len(self) -> PayloadByteCount {
        self.range.byte_count()
    }

    /// Iterates over bytes before this match witness.
    fn prefix_bytes(self) -> impl Iterator<Item = RuntimeByte> + 'state {
        self.bytes.iter().copied().take(self.range.start())
    }

    /// Iterates over bytes after this match witness.
    fn suffix_bytes(self) -> impl Iterator<Item = RuntimeByte> + 'state {
        self.bytes.iter().copied().skip(self.range.end())
    }
}
