use alloc::vec::Vec;
use core::error::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocationContext {
    /// Building the parsed program rule table.
    ProgramRules,
    /// Building a compact code-line byte table.
    CompactCodeLine,
    /// Building canonical source bytes from structured rule data.
    CanonicalSource,
    /// Storing a parsed program payload.
    Payload,
    /// Storing validated runtime input.
    RuntimeInput,
    /// Storing per-run `(once)` rule state.
    RuntimeRuleState,
    /// Building the next runtime state after a rewrite.
    RuntimeState,
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
    context: AllocationContext,
    kind: AllocationErrorKind,
}

/// Reason an allocation boundary failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocationErrorKind {
    /// The required capacity could not be represented as `usize`.
    CapacityOverflow,
    /// The allocator rejected the requested capacity.
    ReserveFailed {
        /// Vector capacity requested at the failing site.
        requested_capacity: usize,
    },
}

impl AllocationError {
    pub(crate) const fn capacity_overflow(context: AllocationContext) -> Self {
        Self {
            context,
            kind: AllocationErrorKind::CapacityOverflow,
        }
    }

    pub(crate) const fn reserve_failed(
        context: AllocationContext,
        requested_capacity: usize,
    ) -> Self {
        Self {
            context,
            kind: AllocationErrorKind::ReserveFailed { requested_capacity },
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

    /// Requested vector capacity, when allocation reached the allocator.
    #[must_use]
    pub const fn requested_capacity(&self) -> Option<usize> {
        match self.kind {
            AllocationErrorKind::CapacityOverflow => None,
            AllocationErrorKind::ReserveFailed { requested_capacity } => Some(requested_capacity),
        }
    }
}

impl Error for AllocationError {}

pub(crate) fn try_reserve_total_exact<T>(
    vec: &mut Vec<T>,
    total_capacity: usize,
    context: AllocationContext,
) -> Result<(), AllocationError> {
    if vec.capacity() >= total_capacity {
        return Ok(());
    }

    let additional = total_capacity
        .checked_sub(vec.len())
        .ok_or_else(|| AllocationError::capacity_overflow(context))?;

    vec.try_reserve_exact(additional)
        .map_err(|_| AllocationError::reserve_failed(context, total_capacity))
}

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
        let requested_capacity =
            core::cmp::max(minimum_capacity, core::cmp::max(4, doubled_capacity));
        try_reserve_total_exact(vec, requested_capacity, context)?;
    }

    vec.push(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocation_contexts_are_publicly_inspectable() {
        let error = AllocationError::reserve_failed(AllocationContext::TraceSnapshot, 123);
        assert_eq!(error.context(), AllocationContext::TraceSnapshot);
        assert_eq!(
            error.kind(),
            AllocationErrorKind::ReserveFailed {
                requested_capacity: 123,
            },
        );
        assert_eq!(error.requested_capacity(), Some(123));

        let error = AllocationError::capacity_overflow(AllocationContext::CanonicalSource);
        assert_eq!(error.context(), AllocationContext::CanonicalSource);
        assert_eq!(error.kind(), AllocationErrorKind::CapacityOverflow);
        assert_eq!(error.requested_capacity(), None);
    }
}
