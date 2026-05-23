/// Compact source bytes with diagnostic positions.
mod compact;
/// Byte-count domain types.
mod count;
/// Parsed payload storage and matcher views.
mod payload;
/// Executable program byte domain.
mod program;
/// Rejected-byte diagnostic domains.
mod rejection;
/// Runtime input and state byte domains.
mod runtime;

#[cfg(test)]
mod tests;

pub(crate) use compact::CompactByte;
pub use count::{
    PayloadByteCount, ReturnOutputByteCount, RuntimeInputByteCount, RuntimeStateByteCount,
    TraceSnapshotByteCount,
};
pub(crate) use payload::{NonEmptyPayloadNeedle, Payload, PayloadNeedle, PayloadSyntax};
pub use rejection::{
    NonAsciiCodeByte, NonAsciiInputByte, NonPrintableCodeByte, ReservedSyntaxByte,
};
pub(crate) use runtime::{RuntimeByte, RuntimeInputByte};
