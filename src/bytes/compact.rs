use crate::source::SourceColumn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CompactByte {
    byte: u8,
    source_column: SourceColumn,
}

impl CompactByte {
    pub(crate) const fn new(byte: u8, source_column: SourceColumn) -> Self {
        Self {
            byte,
            source_column,
        }
    }

    pub(crate) const fn as_u8(self) -> u8 {
        self.byte
    }

    pub(crate) const fn source_column(self) -> SourceColumn {
        self.source_column
    }
}
