/// One-based source line number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceLineNumber {
    one_based: usize,
}

impl SourceLineNumber {
    pub(crate) const MAX: Self = Self {
        one_based: usize::MAX,
    };

    #[cfg(test)]
    pub(crate) const fn from_one_based_unchecked(one_based: usize) -> Self {
        Self { one_based }
    }

    pub(crate) fn from_zero_based(zero_based: usize) -> Option<Self> {
        let one_based = zero_based.checked_add(1)?;
        Some(Self { one_based })
    }

    /// One-based source line number as a primitive value.
    #[must_use]
    pub const fn get(self) -> usize {
        self.one_based
    }
}

/// One-based source column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceColumn {
    one_based: usize,
}

impl SourceColumn {
    pub(crate) fn from_zero_based(zero_based: usize) -> Option<Self> {
        let one_based = zero_based.checked_add(1)?;
        Some(Self { one_based })
    }

    #[cfg(test)]
    pub(crate) const fn from_one_based_unchecked(one_based: usize) -> Self {
        Self { one_based }
    }

    /// One-based source column as a primitive value.
    #[must_use]
    pub const fn get(self) -> usize {
        self.one_based
    }
}

/// One-based source position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourcePosition {
    line: SourceLineNumber,
    column: SourceColumn,
}

impl SourcePosition {
    pub(crate) const fn new(line: SourceLineNumber, column: SourceColumn) -> Self {
        Self { line, column }
    }

    /// One-based source line.
    #[must_use]
    pub const fn line(self) -> SourceLineNumber {
        self.line
    }

    /// One-based source column.
    #[must_use]
    pub const fn column(self) -> SourceColumn {
        self.column
    }
}
