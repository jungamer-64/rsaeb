/// Reserved multi-byte syntax tokens recognized by parser phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyntaxToken {
    /// `(once)` left-side repeat modifier.
    Once,
    /// `(start)` anchor or right-side action token.
    Start,
    /// `(end)` anchor or right-side action token.
    End,
    /// `(return)` right-side action token.
    Return,
}

impl SyntaxToken {
    /// Returns the stored bytes.
    pub(crate) const fn bytes(self) -> &'static [u8] {
        match self {
            Self::Once => b"(once)",
            Self::Start => b"(start)",
            Self::End => b"(end)",
            Self::Return => b"(return)",
        }
    }

    /// Returns the runtime state length in bytes.
    pub(crate) const fn len(self) -> usize {
        self.bytes().len()
    }
}
