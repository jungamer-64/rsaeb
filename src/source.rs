/// Borrowed A=B program source at the parser boundary.
///
/// Program source remains a byte format because comments may contain arbitrary
/// non-UTF-8 bytes. Constructing this value labels a byte slice as source
/// input; syntax validation still happens in [`Program::parse`](crate::Program::parse).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgramSource<'source> {
    bytes: &'source [u8],
}

impl<'source> ProgramSource<'source> {
    /// Labels raw source bytes as parser input.
    #[must_use]
    pub const fn from_bytes(bytes: &'source [u8]) -> Self {
        Self { bytes }
    }

    /// Labels a UTF-8 source string as parser input.
    #[must_use]
    pub const fn from_str(source: &'source str) -> Self {
        Self {
            bytes: source.as_bytes(),
        }
    }

    /// Borrow the source bytes.
    #[must_use]
    pub const fn as_bytes(self) -> &'source [u8] {
        self.bytes
    }

    /// Whether the source contains no bytes.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.bytes.is_empty()
    }
}

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
    pub(crate) fn from_one_based(one_based: usize) -> Option<Self> {
        (one_based != 0).then_some(Self { one_based })
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
    pub(crate) fn from_one_based(one_based: usize) -> Option<Self> {
        (one_based != 0).then_some(Self { one_based })
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
