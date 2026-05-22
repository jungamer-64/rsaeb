use core::fmt;

/// Byte length of executable program payload data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PayloadByteCount {
    value: usize,
}

impl PayloadByteCount {
    /// Creates a payload byte count from a primitive length.
    #[must_use]
    pub(crate) const fn new(value: usize) -> Self {
        Self { value }
    }

    /// Returns this byte count as a primitive length.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }

    /// Returns whether this count is zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.value == 0
    }
}

impl fmt::Display for PayloadByteCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}

/// Byte length of validated runtime input before execution-state materialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuntimeInputByteCount {
    value: usize,
}

impl RuntimeInputByteCount {
    /// Creates a runtime-input byte count from a primitive length.
    #[must_use]
    pub(crate) const fn new(value: usize) -> Self {
        Self { value }
    }

    /// Returns this byte count as a primitive length.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }

    /// Returns whether this count is zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.value == 0
    }
}

impl fmt::Display for RuntimeInputByteCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}

/// Byte length of materialized runtime state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuntimeStateByteCount {
    value: usize,
}

impl RuntimeStateByteCount {
    /// Creates a runtime-state byte count from a primitive length.
    #[must_use]
    pub(crate) const fn new(value: usize) -> Self {
        Self { value }
    }

    /// Converts validated runtime-input length into initial runtime-state length.
    #[must_use]
    pub(crate) const fn from_runtime_input_count(count: RuntimeInputByteCount) -> Self {
        Self { value: count.get() }
    }

    /// Returns this byte count as a primitive length.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }

    /// Returns whether this count is zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.value == 0
    }
}

impl fmt::Display for RuntimeStateByteCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}

/// Byte length of a `(return)` output payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReturnOutputByteCount {
    value: usize,
}

impl ReturnOutputByteCount {
    /// Creates a `(return)` output byte count from a primitive length.
    #[must_use]
    pub(crate) const fn new(value: usize) -> Self {
        Self { value }
    }

    /// Converts a parsed return payload length into return-output length.
    #[must_use]
    pub(crate) const fn from_payload_count(count: PayloadByteCount) -> Self {
        Self { value: count.get() }
    }

    /// Returns this byte count as a primitive length.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }

    /// Returns whether this count is zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.value == 0
    }
}

impl fmt::Display for ReturnOutputByteCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}

/// Byte length budgeted for one trace snapshot event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TraceSnapshotByteCount {
    value: usize,
}

impl TraceSnapshotByteCount {
    /// Converts runtime-state length into a trace snapshot event length.
    #[must_use]
    pub(crate) const fn from_runtime_state_count(count: RuntimeStateByteCount) -> Self {
        Self { value: count.get() }
    }

    /// Converts return-output length into a trace snapshot event length.
    #[must_use]
    pub(crate) const fn from_return_output_count(count: ReturnOutputByteCount) -> Self {
        Self { value: count.get() }
    }

    /// Returns this byte count as a primitive length.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }

    /// Returns whether this count is zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.value == 0
    }
}

impl fmt::Display for TraceSnapshotByteCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}
