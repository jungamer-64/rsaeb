use alloc::vec::Vec;
use core::marker::PhantomData;

/// Internal runtime state snapshot domain alternatives.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RuntimeStateSnapshotDomain {}

/// Internal return output domain alternatives.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReturnOutputDomain {}

/// Internal payload inspection domain alternatives.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PayloadInspectionDomain {}

/// Internal canonical rule source domain alternatives.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CanonicalRuleSourceDomain {}

/// Internal materialized bytes.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MaterializedBytes<Domain> {
    /// Stored bytes.
    bytes: Vec<u8>,
    /// Stored domain.
    domain: PhantomData<fn() -> Domain>,
}

impl<Domain> MaterializedBytes<Domain> {
    /// Builds the value from vec input.
    pub(crate) fn from_vec(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            domain: PhantomData,
        }
    }

    /// Runs the as slice operation.
    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    /// Runs the into raw bytes operation.
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
