//! Source-position value types.
//!
//! Program source bytes are parsed through the target program type. Syntax
//! validation belongs to the parser, which reports failures with
//! [`SourceLineNumber`], [`SourceColumn`], and [`SourcePosition`].
//!
//! Source is intentionally separate from [`crate::input::RuntimeInput`].
//! Comments may contain arbitrary bytes, while executable source code is
//! validated by the parser and runtime input is validated by the runtime-input
//! boundary.
//!
//! # Compile-time guards
//!
//! The shape-neutral public `ProgramSource` boundary has been deleted:
//!
//! ```compile_fail
//! use rsaeb::source::ProgramSource;
//!
//! fn main() {}
//! ```
//!
//! Public source-shape marker types have been deleted:
//!
//! ```compile_fail
//! use rsaeb::source::{EmptyProgramSource, ExecutableProgramSource};
//!
//! fn main() {}
//! ```

/// Private raw source carrier used by the parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RawProgramSource<'source> {
    /// Raw source bytes owned by the caller.
    bytes: &'source [u8],
}

impl<'source> RawProgramSource<'source> {
    /// Labels raw bytes as parser input.
    pub(crate) const fn from_bytes(bytes: &'source [u8]) -> Self {
        Self { bytes }
    }

    /// Labels a UTF-8 string as parser input.
    pub(crate) const fn from_text(source: &'source str) -> Self {
        Self {
            bytes: source.as_bytes(),
        }
    }

    /// Borrows the original source bytes.
    pub(crate) const fn as_bytes(self) -> &'source [u8] {
        self.bytes
    }
}

/// One-based source line number in parsed source diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceLineNumber {
    /// One-based source line used in parse diagnostics.
    one_based: usize,
}

impl SourceLineNumber {
    /// ONE boundary value.
    pub(crate) const ONE: Self = Self { one_based: 1 };
    /// MAX boundary value.
    pub(crate) const MAX: Self = Self {
        one_based: usize::MAX,
    };

    /// Builds an index from a zero-based offset.
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

/// One-based source column in parsed source diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceColumn {
    /// One-based source column used in parse diagnostics.
    one_based: usize,
}

impl SourceColumn {
    /// Builds an index from a zero-based offset.
    pub(crate) fn from_zero_based(zero_based: usize) -> Option<Self> {
        let one_based = zero_based.checked_add(1)?;
        Some(Self { one_based })
    }

    /// One-based source column as a primitive value.
    #[must_use]
    pub const fn get(self) -> usize {
        self.one_based
    }
}

/// One-based source position in parsed source diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourcePosition {
    /// One-based source line.
    line: SourceLineNumber,
    /// One-based source column.
    column: SourceColumn,
}

impl SourcePosition {
    /// Combines already validated source coordinates.
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
