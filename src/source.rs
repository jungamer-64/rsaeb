//! Program-source boundary and source-position value types.
//!
//! Source values label host bytes by the program shape the caller expects to
//! parse. [`ExecutableProgramSource`] can only enter
//! [`program::ExecutableProgram::parse`](crate::program::ExecutableProgram::parse),
//! while [`EmptyProgramSource`] can only enter
//! [`program::EmptyProgram::parse`](crate::program::EmptyProgram::parse).
//! Syntax validation still belongs to the parser, which reports failures with
//! [`SourceLineNumber`], [`SourceColumn`], and [`SourcePosition`].
//!
//! Source is intentionally separate from [`crate::input::RuntimeInput`].
//! Comments may contain arbitrary bytes, while executable source code is
//! validated by the parser and runtime input is validated by the runtime-input
//! boundary.
//!
//! ```
//! use rsaeb::policy::DefaultParsePolicy;
//! use rsaeb::program::ExecutableProgram;
//! use rsaeb::source::ExecutableProgramSource;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let source = ExecutableProgramSource::from_bytes(b"a=b # arbitrary comment bytes: \xff");
//! let executable = ExecutableProgram::<DefaultParsePolicy>::parse(source)?;
//!
//! if executable.rule_count().get() != 1 {
//!     return Err("unexpected rule count".into());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ```
//! use rsaeb::policy::DefaultParsePolicy;
//! use rsaeb::program::EmptyProgram;
//! use rsaeb::source::EmptyProgramSource;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let source = EmptyProgramSource::from_text("# no executable rules");
//! let empty = EmptyProgram::<DefaultParsePolicy>::parse(source)?;
//!
//! if empty.rule_count().get() != 0 {
//!     return Err("unexpected rule count".into());
//! }
//! # Ok(())
//! # }
//! ```

/// Borrowed source expected to parse into an executable program.
///
/// Constructing this value labels host bytes for the executable-program parser.
/// It does not validate syntax or prove that the bytes contain executable
/// rules; that content check remains part of
/// [`program::ExecutableProgram::parse`](crate::program::ExecutableProgram::parse).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutableProgramSource<'source> {
    /// Shape-specific wrapper around raw source bytes.
    raw: RawProgramSource<'source>,
}

/// Borrowed source expected to parse into an empty program.
///
/// Constructing this value labels host bytes for the empty-program parser. It
/// does not validate syntax or prove that the bytes contain no executable
/// rules; that content check remains part of
/// [`program::EmptyProgram::parse`](crate::program::EmptyProgram::parse).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptyProgramSource<'source> {
    /// Shape-specific wrapper around raw source bytes.
    raw: RawProgramSource<'source>,
}

/// Private raw source carrier shared by both public source shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RawProgramSource<'source> {
    /// Raw source bytes owned by the caller.
    bytes: &'source [u8],
}

impl<'source> ExecutableProgramSource<'source> {
    /// Labels raw bytes as executable-program parser input.
    ///
    /// This constructor accepts any byte slice. Executable code bytes are
    /// checked later by
    /// [`program::ExecutableProgram::parse`](crate::program::ExecutableProgram::parse);
    /// bytes after a line-comment marker remain part of the source byte stream
    /// but are not executable code.
    #[must_use]
    pub const fn from_bytes(bytes: &'source [u8]) -> Self {
        Self {
            raw: RawProgramSource::from_bytes(bytes),
        }
    }

    /// Labels a UTF-8 string as executable-program parser input.
    #[must_use]
    pub const fn from_text(source: &'source str) -> Self {
        Self {
            raw: RawProgramSource::from_text(source),
        }
    }

    /// Borrows the original source bytes.
    #[must_use]
    pub const fn as_bytes(self) -> &'source [u8] {
        self.raw.as_bytes()
    }

    /// Returns whether the source contains no bytes.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.raw.is_empty()
    }

    /// Moves this typed public source into the parser's raw carrier.
    pub(crate) const fn into_raw(self) -> RawProgramSource<'source> {
        self.raw
    }
}

impl<'source> EmptyProgramSource<'source> {
    /// Labels raw bytes as empty-program parser input.
    ///
    /// This constructor accepts any byte slice. Executable code bytes are
    /// checked later by
    /// [`program::EmptyProgram::parse`](crate::program::EmptyProgram::parse);
    /// bytes after a line-comment marker remain part of the source byte stream
    /// but are not executable code.
    #[must_use]
    pub const fn from_bytes(bytes: &'source [u8]) -> Self {
        Self {
            raw: RawProgramSource::from_bytes(bytes),
        }
    }

    /// Labels a UTF-8 string as empty-program parser input.
    #[must_use]
    pub const fn from_text(source: &'source str) -> Self {
        Self {
            raw: RawProgramSource::from_text(source),
        }
    }

    /// Borrows the original source bytes.
    #[must_use]
    pub const fn as_bytes(self) -> &'source [u8] {
        self.raw.as_bytes()
    }

    /// Returns whether the source contains no bytes.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.raw.is_empty()
    }

    /// Moves this typed public source into the parser's raw carrier.
    pub(crate) const fn into_raw(self) -> RawProgramSource<'source> {
        self.raw
    }
}

impl<'source> RawProgramSource<'source> {
    /// Labels raw bytes as parser input.
    const fn from_bytes(bytes: &'source [u8]) -> Self {
        Self { bytes }
    }

    /// Labels a UTF-8 string as parser input.
    const fn from_text(source: &'source str) -> Self {
        Self {
            bytes: source.as_bytes(),
        }
    }

    /// Borrows the original source bytes.
    pub(crate) const fn as_bytes(self) -> &'source [u8] {
        self.bytes
    }

    /// Returns whether the source contains no bytes.
    const fn is_empty(self) -> bool {
        self.bytes.is_empty()
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
