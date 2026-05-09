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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocationError {
    context: AllocationContext,
    requested_capacity: usize,
}

impl AllocationError {
    pub(crate) const fn new(context: AllocationContext, requested_capacity: usize) -> Self {
        Self {
            context,
            requested_capacity,
        }
    }

    /// Allocation site that failed.
    #[must_use]
    pub const fn context(&self) -> AllocationContext {
        self.context
    }

    /// Requested vector capacity at the failing site.
    #[must_use]
    pub const fn requested_capacity(&self) -> usize {
        self.requested_capacity
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

    let additional = total_capacity.saturating_sub(vec.len());
    vec.try_reserve_exact(additional)
        .map_err(|_| AllocationError::new(context, total_capacity))
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
            .ok_or_else(|| AllocationError::new(context, usize::MAX))?;
        let doubled_capacity = vec.capacity().saturating_mul(2);
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
        let error = AllocationError::new(AllocationContext::TraceSnapshot, 123);
        assert_eq!(error.context(), AllocationContext::TraceSnapshot);
        assert_eq!(error.requested_capacity(), 123);
    }
}
