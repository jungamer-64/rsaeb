use alloc::vec::Vec;

use crate::allocation::{AllocationContext, AllocationError, try_push, try_reserve_total_exact};
use crate::bytes::{Payload, RuntimeByte};

use super::state::MatchedStateSpan;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RewritePlacement {
    Replace,
    MoveStart,
    MoveEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RewriteRequest<'rule> {
    state_match: MatchedStateSpan,
    rhs: &'rule Payload,
    placement: RewritePlacement,
}

impl<'rule> RewriteRequest<'rule> {
    pub(super) const fn new(
        state_match: MatchedStateSpan,
        rhs: &'rule Payload,
        placement: RewritePlacement,
    ) -> Self {
        Self {
            state_match,
            rhs,
            placement,
        }
    }

    pub(super) const fn state_match(self) -> MatchedStateSpan {
        self.state_match
    }

    pub(super) const fn rhs(self) -> &'rule Payload {
        self.rhs
    }

    pub(super) const fn placement(self) -> RewritePlacement {
        self.placement
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct RewriteScratch {
    pub(super) bytes: Vec<RuntimeByte>,
}

impl RewriteScratch {
    pub(super) fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    pub(super) fn clear_and_reserve(&mut self, capacity: usize) -> Result<(), AllocationError> {
        self.bytes.clear();
        try_reserve_total_exact(
            &mut self.bytes,
            capacity,
            AllocationContext::RuntimeRewriteState,
        )
    }

    pub(super) fn push_existing(
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

    pub(super) fn push_payload(&mut self, payload: &Payload) -> Result<(), AllocationError> {
        self.push_existing(payload.runtime_bytes())
    }
}
