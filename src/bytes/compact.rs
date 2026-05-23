use crate::source::SourceColumn;

/// Internal compact byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CompactByte {
    /// Stored byte.
    byte: u8,
    /// Stored source column.
    source_column: SourceColumn,
}

impl CompactByte {
    /// Constructs the value from validated parts.
    pub(crate) const fn new(byte: u8, source_column: SourceColumn) -> Self {
        Self {
            byte,
            source_column,
        }
    }

    /// Returns the as u8 view.
    pub(crate) const fn as_u8(self) -> u8 {
        self.byte
    }

    /// Runs the source column operation.
    pub(crate) const fn source_column(self) -> SourceColumn {
        self.source_column
    }
}
