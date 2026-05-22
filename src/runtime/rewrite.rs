use alloc::vec::Vec;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::bytes::{Payload, RuntimeByte, RuntimeStateByteCount};

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

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PreparedRewrite {
    bytes: Vec<RuntimeByte>,
}

impl PreparedRewrite {
    pub(crate) fn into_runtime_bytes(self) -> Vec<RuntimeByte> {
        self.bytes
    }
}

impl RewriteScratch {
    pub(crate) fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    pub(crate) fn take_prepared(&mut self) -> PreparedRewrite {
        PreparedRewrite {
            bytes: core::mem::take(&mut self.bytes),
        }
    }

    pub(crate) fn store_previous_state(&mut self, bytes: Vec<RuntimeByte>) {
        self.bytes = bytes;
    }

    /// Clears scratch storage and reserves the requested rewrite capacity.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the rewrite scratch buffer cannot reserve
    /// the requested capacity.
    pub(crate) fn clear_and_reserve(
        &mut self,
        capacity: RuntimeStateByteCount,
    ) -> Result<(), AllocationError> {
        self.bytes.clear();
        try_reserve_total_exact(
            &mut self.bytes,
            RequestedCapacity::new(capacity.get()),
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
