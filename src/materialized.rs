use alloc::vec::Vec;
use core::marker::PhantomData;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RuntimeStateSnapshotDomain {}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReturnOutputDomain {}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PayloadInspectionDomain {}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CanonicalRuleSourceDomain {}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MaterializedBytes<Domain> {
    bytes: Vec<u8>,
    domain: PhantomData<fn() -> Domain>,
}

impl<Domain> MaterializedBytes<Domain> {
    pub(crate) fn from_vec(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            domain: PhantomData,
        }
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn into_raw_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}
