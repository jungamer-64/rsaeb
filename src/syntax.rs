/// Internal syntax token alternatives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyntaxToken {
    /// Once case.
    Once,
    /// Start case.
    Start,
    /// End case.
    End,
    /// Return case.
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
