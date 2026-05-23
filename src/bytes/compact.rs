use crate::source::SourceColumn;

/// Executable source byte after whitespace removal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CompactByte {
    /// Compact executable byte.
    byte: u8,
    /// Original source column retained for diagnostics.
    source_column: SourceColumn,
}

impl CompactByte {
    /// Attaches the original source column to a compact executable byte.
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

    /// Original source column for parse errors involving this byte.
    pub(crate) const fn source_column(self) -> SourceColumn {
        self.source_column
    }
}
