use alloc::vec::Vec;

use crate::allocation::{AllocationContext, AllocationError, try_push, try_reserve_total_exact};
use crate::bytes::{Payload, RuntimeByte};

use super::state::MatchedStateSpan;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RewriteRequest<'rule> {
    Replace(RewriteOperands<'rule>),
    MoveStart(RewriteOperands<'rule>),
    MoveEnd(RewriteOperands<'rule>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RewriteOperands<'rule> {
    state_match: MatchedStateSpan,
    rhs: &'rule Payload,
}

impl<'rule> RewriteOperands<'rule> {
    const fn new(state_match: MatchedStateSpan, rhs: &'rule Payload) -> Self {
        Self { state_match, rhs }
    }

    pub(crate) const fn state_match(self) -> MatchedStateSpan {
        self.state_match
    }

    pub(crate) const fn rhs(self) -> &'rule Payload {
        self.rhs
    }
}

impl<'rule> RewriteRequest<'rule> {
    pub(crate) const fn replace(state_match: MatchedStateSpan, rhs: &'rule Payload) -> Self {
        Self::Replace(RewriteOperands::new(state_match, rhs))
    }

    pub(crate) const fn move_start(state_match: MatchedStateSpan, rhs: &'rule Payload) -> Self {
        Self::MoveStart(RewriteOperands::new(state_match, rhs))
    }

    pub(crate) const fn move_end(state_match: MatchedStateSpan, rhs: &'rule Payload) -> Self {
        Self::MoveEnd(RewriteOperands::new(state_match, rhs))
    }

    pub(crate) const fn state_match(self) -> MatchedStateSpan {
        self.operands().state_match()
    }

    pub(crate) const fn rhs(self) -> &'rule Payload {
        self.operands().rhs()
    }

    pub(crate) const fn operands(self) -> RewriteOperands<'rule> {
        match self {
            Self::Replace(operands) | Self::MoveStart(operands) | Self::MoveEnd(operands) => {
                operands
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RewriteScratch {
    bytes: Vec<RuntimeByte>,
}

impl RewriteScratch {
    pub(crate) fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    pub(crate) fn swap_with_state_bytes(&mut self, state_bytes: &mut Vec<RuntimeByte>) {
        core::mem::swap(state_bytes, &mut self.bytes);
    }

    /// Clears scratch storage and reserves the requested rewrite capacity.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the rewrite scratch buffer cannot reserve
    /// the requested capacity.
    pub(crate) fn clear_and_reserve(&mut self, capacity: usize) -> Result<(), AllocationError> {
        self.bytes.clear();
        try_reserve_total_exact(
            &mut self.bytes,
            capacity,
            AllocationContext::RuntimeRewriteState,
        )
    }

    /// Appends existing runtime bytes into scratch storage.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the rewrite scratch buffer cannot grow.
    pub(crate) fn push_existing(
        &mut self,
        source: impl IntoIterator<Item = RuntimeByte>,
    ) -> Result<(), AllocationError> {
        for byte in source {
            try_push(
                &mut self.bytes,
                byte,
                AllocationContext::RuntimeRewriteState,
            )?;
        }

        Ok(())
    }

    /// Appends program payload bytes into scratch storage.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the rewrite scratch buffer cannot grow.
    pub(crate) fn push_payload(&mut self, payload: &Payload) -> Result<(), AllocationError> {
        self.push_existing(payload.runtime_bytes())
    }
}
