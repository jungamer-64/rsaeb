/// Non-ASCII byte rejected from executable program code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NonAsciiCodeByte {
    /// Stored byte.
    byte: u8,
}

impl NonAsciiCodeByte {
    /// Parses the value at this boundary.
    pub(crate) const fn parse(byte: u8) -> Option<Self> {
        if byte.is_ascii() {
            None
        } else {
            Some(Self { byte })
        }
    }

    /// Returns the rejected raw byte.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.byte
    }
}

/// Non-printable ASCII byte rejected from executable program code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NonPrintableCodeByte {
    /// Stored byte.
    byte: u8,
}

impl NonPrintableCodeByte {
    /// Parses the value at this boundary.
    pub(crate) const fn parse(byte: u8) -> Option<Self> {
        if byte.is_ascii() && !byte.is_ascii_graphic() {
            Some(Self { byte })
        } else {
            None
        }
    }

    /// Returns the rejected raw byte.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.byte
    }
}

/// Non-ASCII byte rejected from runtime input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NonAsciiInputByte {
    /// Stored byte.
    byte: u8,
}

impl NonAsciiInputByte {
    /// Parses the value at this boundary.
    pub(crate) const fn parse(byte: u8) -> Option<Self> {
        if byte.is_ascii() {
            None
        } else {
            Some(Self { byte })
        }
    }

    /// Returns the rejected raw byte.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.byte
    }
}

/// Reserved executable syntax byte rejected from program payload data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReservedSyntaxByte {
    /// The `=` rule separator byte.
    Equals,
    /// The `#` line-comment byte.
    Comment,
    /// The `(` modifier/action opening byte.
    OpenParen,
    /// The `)` modifier/action closing byte.
    CloseParen,
}

impl ReservedSyntaxByte {
    /// Parses the value at this boundary.
    pub(crate) const fn parse(byte: u8) -> Option<Self> {
        match byte {
            b'=' => Some(Self::Equals),
            b'#' => Some(Self::Comment),
            b'(' => Some(Self::OpenParen),
            b')' => Some(Self::CloseParen),
            _ => None,
        }
    }

    /// Returns the reserved raw syntax byte.
    #[must_use]
    pub const fn get(self) -> u8 {
        match self {
            Self::Equals => b'=',
            Self::Comment => b'#',
            Self::OpenParen => b'(',
            Self::CloseParen => b')',
        }
    }
}
