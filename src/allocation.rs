use alloc::vec::Vec;
use core::error::Error;
use core::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocationContext {
    /// Building the parsed program rule table.
    ProgramRules,
    /// Building a compact code-line byte table.
    CompactCodeLine,
    /// Storing compact source bytes for rule metadata.
    CompactSource,
    /// Storing a parsed program payload.
    Payload,
    /// Storing validated runtime input.
    RuntimeInput,
    /// Storing per-run `(once)` rule state.
    RuntimeRuleState,
    /// Building the next runtime state after a rewrite.
    RuntimeState,
    /// Materializing `(return)` output bytes.
    ReturnOutput,
    /// Materializing the final stable output bytes.
    FinalOutput,
    /// Materializing the state stored in a step-limit error.
    StepLimitState,
    /// Materializing a trace snapshot.
    TraceSnapshot,
}

impl fmt::Display for AllocationContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProgramRules => write!(f, "program rule table"),
            Self::CompactCodeLine => write!(f, "compact code line"),
            Self::CompactSource => write!(f, "compact source metadata"),
            Self::Payload => write!(f, "program payload"),
            Self::RuntimeInput => write!(f, "runtime input state"),
            Self::RuntimeRuleState => write!(f, "runtime rule state"),
            Self::RuntimeState => write!(f, "runtime rewrite state"),
            Self::ReturnOutput => write!(f, "return output"),
            Self::FinalOutput => write!(f, "final stable output"),
            Self::StepLimitState => write!(f, "step-limit state"),
            Self::TraceSnapshot => write!(f, "trace snapshot"),
        }
    }
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

impl fmt::Display for AllocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "allocation failure while building {}; requested capacity: {}",
            self.context, self.requested_capacity,
        )
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
        let requested_capacity = vec
            .len()
            .checked_add(1)
            .ok_or_else(|| AllocationError::new(context, usize::MAX))?;
        try_reserve_total_exact(vec, requested_capacity, context)?;
    }

    vec.push(value);
    Ok(())
}

pub(crate) fn copy_bytes(
    source: &[u8],
    context: AllocationContext,
) -> Result<Vec<u8>, AllocationError> {
    let mut output = Vec::new();
    try_reserve_total_exact(&mut output, source.len(), context)?;
    output.extend_from_slice(source);
    Ok(output)
}
