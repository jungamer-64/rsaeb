use alloc::vec::Vec;
use core::marker::PhantomData;

/// Marker for bytes materialized from runtime state.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RuntimeStateSnapshotDomain {}

/// Marker for bytes materialized from `(return)` output.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReturnOutputDomain {}

/// Marker for bytes materialized from parsed payload inspection.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PayloadInspectionDomain {}

/// Marker for bytes materialized as canonical rule source.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CanonicalRuleSourceDomain {}

/// Owned bytes tagged with the domain that produced them.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MaterializedBytes<Domain> {
    /// Materialized byte payload.
    bytes: Vec<u8>,
    /// Compile-time tag preventing byte-domain mixups.
    domain: PhantomData<fn() -> Domain>,
}

impl<Domain> MaterializedBytes<Domain> {
    /// Tags already materialized bytes with their producing domain.
    pub(crate) fn from_vec(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            domain: PhantomData,
        }
    }

    /// Borrows the materialized bytes without erasing the domain tag.
    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    /// Releases the byte payload at the public boundary.
    pub(crate) fn into_raw_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Returns the runtime state length in bytes.
    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Returns whether the byte sequence is empty.
    pub(crate) const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}
