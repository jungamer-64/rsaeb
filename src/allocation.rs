use alloc::vec::Vec;
use core::error::Error;

use crate::bytes::{PayloadByteCount, RuntimeInputByteCount, RuntimeStateByteCount};
use crate::inspect::RuleCount;

/// Interpreter allocation site reported by [`AllocationError`].
///
/// The value identifies the domain boundary that was allocating, so callers can
/// distinguish parser storage, runtime state growth, final output
/// materialization, and trace snapshot materialization without parsing strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocationContext {
    /// Building a compact code-line byte table.
    ProgramCodeLine,
    /// Storing a parsed program payload.
    ProgramPayload,
    /// Building the parsed program rule table.
    ProgramRuleTable,
    /// Building canonical source bytes from structured rule data.
    CanonicalSource,
    /// Classifying raw runtime input into owned typed input bytes.
    RuntimeInputValidation,
    /// Storing per-run `(once)` slot state.
    OnceRuleState,
    /// Building the next runtime state after a rewrite.
    RuntimeRewriteState,
    /// Materializing a payload view outside parser/runtime execution.
    PayloadView,
    /// Materializing parsed rule payloads for owned execution witnesses.
    OwnedRuleWitness,
    /// Materializing a borrowed runtime-state view outside trace snapshot APIs.
    RuntimeStateView,
    /// Materializing a stable final runtime state as public output bytes.
    FinalOutput,
    /// Materializing `(return)` output bytes.
    ReturnOutput,
    /// Materializing a trace snapshot.
    TraceSnapshot,
}

/// Fallible allocation failure reported instead of silently relying on
/// allocation side effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllocationError {
    /// Allocation boundary that requested memory.
    context: AllocationContext,
    /// Structured reason the allocation boundary failed.
    kind: AllocationErrorKind,
}

/// Reason an allocation boundary failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocationErrorKind {
    /// The required capacity could not be represented as `usize`.
    CapacityOverflow,
    /// Reserving the requested capacity failed.
    ReservationFailed {
        /// Vector capacity requested at the failing site.
        requested_capacity: RequestedCapacity,
    },
}

/// Vector capacity requested at a fallible allocation boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestedCapacity {
    /// Requested vector capacity in elements.
    value: usize,
}

impl RequestedCapacity {
    /// Records a capacity request after the caller has chosen the allocation boundary.
    pub(crate) const fn new(value: usize) -> Self {
        Self { value }
    }

    /// Requests storage for validated runtime-input bytes.
    pub(crate) const fn from_runtime_input_count(count: RuntimeInputByteCount) -> Self {
        Self { value: count.get() }
    }

    /// Requests storage for runtime-state bytes.
    pub(crate) const fn from_runtime_state_count(count: RuntimeStateByteCount) -> Self {
        Self { value: count.get() }
    }

    /// Requests storage for parsed payload bytes.
    pub(crate) const fn from_payload_count(count: PayloadByteCount) -> Self {
        Self { value: count.get() }
    }

    /// Requests storage for parsed rule-table entries.
    pub(crate) const fn from_rule_count(count: RuleCount) -> Self {
        Self { value: count.get() }
    }

    /// Requested capacity as a primitive value.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }
}

impl core::fmt::Display for RequestedCapacity {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.value.fmt(formatter)
    }
}

impl AllocationError {
    /// Builds a capacity-overflow allocation failure.
    pub(crate) const fn capacity_overflow(context: AllocationContext) -> Self {
        Self {
            context,
            kind: AllocationErrorKind::CapacityOverflow,
        }
    }

    /// Builds a reservation-failed allocation failure.
    pub(crate) const fn reservation_failed(
        context: AllocationContext,
        requested_capacity: RequestedCapacity,
    ) -> Self {
        Self {
            context,
            kind: AllocationErrorKind::ReservationFailed { requested_capacity },
        }
    }

    /// Allocation site that failed.
    #[must_use]
    pub const fn context(&self) -> AllocationContext {
        self.context
    }

    /// Structured allocation failure reason.
    #[must_use]
    pub const fn kind(&self) -> AllocationErrorKind {
        self.kind
    }
}

impl Error for AllocationError {}

/// Ensures `vec` can hold exactly `total_capacity` items.
///
/// # Errors
///
/// Returns `AllocationError` if the requested capacity cannot be represented
/// or if the allocator rejects the reservation.
pub(crate) fn try_reserve_total_exact<T>(
    vec: &mut Vec<T>,
    total_capacity: RequestedCapacity,
    context: AllocationContext,
) -> Result<(), AllocationError> {
    if vec.capacity() >= total_capacity.get() {
        return Ok(());
    }

    let additional = total_capacity
        .get()
        .checked_sub(vec.len())
        .ok_or_else(|| AllocationError::capacity_overflow(context))?;

    vec.try_reserve_exact(additional)
        .map_err(|_| AllocationError::reservation_failed(context, total_capacity))
}

/// Pushes one value after reserving through the explicit allocation boundary.
///
/// # Errors
///
/// Returns `AllocationError` if the next capacity cannot be represented or if
/// the allocator rejects the reservation.
pub(crate) fn try_push<T>(
    vec: &mut Vec<T>,
    value: T,
    context: AllocationContext,
) -> Result<(), AllocationError> {
    if vec.len() == vec.capacity() {
        let minimum_capacity = vec
            .len()
            .checked_add(1)
            .ok_or_else(|| AllocationError::capacity_overflow(context))?;
        let doubled_capacity = vec.capacity().checked_mul(2).unwrap_or(minimum_capacity);
        let requested_capacity = RequestedCapacity::new(core::cmp::max(
            minimum_capacity,
            core::cmp::max(4, doubled_capacity),
        ));
        try_reserve_total_exact(vec, requested_capacity, context)?;
    }

    vec.push(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{TestResult, ensure_eq};
    use alloc::string::ToString;

    /// # Errors
    ///
    /// Returns `TestFailure` if an allocation error field does not match the
    /// expected public value.
    #[test]
    fn allocation_contexts_are_publicly_inspectable() -> TestResult {
        let error = AllocationError::reservation_failed(
            AllocationContext::TraceSnapshot,
            RequestedCapacity::new(123),
        );
        ensure_eq!(error.context(), AllocationContext::TraceSnapshot)?;
        ensure_eq!(
            error.kind(),
            AllocationErrorKind::ReservationFailed {
                requested_capacity: RequestedCapacity::new(123),
            },
        )?;

        let error = AllocationError::capacity_overflow(AllocationContext::CanonicalSource);
        ensure_eq!(error.context(), AllocationContext::CanonicalSource)?;
        ensure_eq!(error.kind(), AllocationErrorKind::CapacityOverflow)?;

        let error = AllocationError::reservation_failed(
            AllocationContext::RuntimeInputValidation,
            RequestedCapacity::new(4),
        );
        ensure_eq!(error.context(), AllocationContext::RuntimeInputValidation)?;

        let error = AllocationError::reservation_failed(
            AllocationContext::OwnedRuleWitness,
            RequestedCapacity::new(5),
        );
        ensure_eq!(error.context(), AllocationContext::OwnedRuleWitness)?;

        Ok(())
    }

    /// # Errors
    ///
    /// Returns `TestFailure` if an allocation error display string drifts from
    /// the expected domain wording.
    #[test]
    fn allocation_display_names_the_failed_context_and_capacity() -> TestResult {
        let error = AllocationError::reservation_failed(
            AllocationContext::TraceSnapshot,
            RequestedCapacity::new(123),
        );

        ensure_eq!(
            error.to_string(),
            "allocation reservation failure while building trace snapshot; requested capacity: 123"
        )?;

        let error = AllocationError::reservation_failed(
            AllocationContext::RuntimeStateView,
            RequestedCapacity::new(456),
        );

        ensure_eq!(
            error.to_string(),
            "allocation reservation failure while building runtime state view; requested capacity: 456",
        )?;

        let error = AllocationError::reservation_failed(
            AllocationContext::RuntimeInputValidation,
            RequestedCapacity::new(789),
        );

        ensure_eq!(
            error.to_string(),
            "allocation reservation failure while building runtime input validation; requested capacity: 789",
        )?;

        let error = AllocationError::reservation_failed(
            AllocationContext::OwnedRuleWitness,
            RequestedCapacity::new(5),
        );

        ensure_eq!(
            error.to_string(),
            "allocation reservation failure while building owned execution rule witness; requested capacity: 5",
        )?;

        let error = AllocationError::capacity_overflow(AllocationContext::CanonicalSource);

        ensure_eq!(
            error.to_string(),
            "allocation capacity overflow while building canonical source bytes",
        )
    }
}
