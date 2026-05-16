//! Program-source boundary and source-position value types.
//!
//! A [`ProgramSource`] only labels bytes as source input; it does not validate
//! A=B syntax. Validation belongs to [`Program::parse`](crate::Program::parse),
//! which can then report parse failures with [`SourceLineNumber`],
//! [`SourceColumn`], and [`SourcePosition`].
//!
//! Source is intentionally separate from [`RuntimeInput`](crate::RuntimeInput).
//! Comments may contain arbitrary bytes, while executable source code is
//! validated by the parser and runtime input is validated by the runtime-input
//! boundary.
//!
//! ```
//! use rsaeb::limits::DEFAULT_PARSE_LIMITS;
//! use rsaeb::{Program, ProgramSource};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let source = ProgramSource::from_bytes(b"a=b # arbitrary comment bytes: \xff");
//! let program = Program::parse(source, DEFAULT_PARSE_LIMITS)?;
//!
//! assert_eq!(program.rule_count().get(), 1);
//! # Ok(())
//! # }
//! ```

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
    /// Labels raw bytes as parser input.
    ///
    /// This constructor accepts any byte slice. Executable code bytes are
    /// checked later by [`Program::parse`](crate::Program::parse); bytes after a
    /// line-comment marker remain part of the source byte stream but are not
    /// executable code.
    #[must_use]
    pub const fn from_bytes(bytes: &'source [u8]) -> Self {
        Self { bytes }
    }

    /// Labels a UTF-8 string as parser input.
    ///
    /// This is the ergonomic constructor for ordinary source literals. It is
    /// equivalent to [`ProgramSource::from_bytes`] on `source.as_bytes()`.
    #[must_use]
    pub const fn from_text(source: &'source str) -> Self {
        Self {
            bytes: source.as_bytes(),
        }
    }

    /// Borrows the original source bytes.
    #[must_use]
    pub const fn as_bytes(self) -> &'source [u8] {
        self.bytes
    }

    /// Returns whether the source contains no bytes.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.bytes.is_empty()
    }
}

impl<'source> From<&'source [u8]> for ProgramSource<'source> {
    fn from(bytes: &'source [u8]) -> Self {
        Self::from_bytes(bytes)
    }
}

impl<'source> From<&'source str> for ProgramSource<'source> {
    fn from(source: &'source str) -> Self {
        Self::from_text(source)
    }
}

/// One-based source line number in parsed source diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceLineNumber {
    one_based: usize,
}

impl SourceLineNumber {
    pub(crate) const ONE: Self = Self { one_based: 1 };
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

/// One-based source column in parsed source diagnostics.
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

/// One-based source position in parsed source diagnostics.
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
