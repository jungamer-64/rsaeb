#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyntaxToken {
    Once,
    Start,
    End,
    Return,
}

impl SyntaxToken {
    pub(crate) const fn bytes(self) -> &'static [u8] {
        match self {
            Self::Once => b"(once)",
            Self::Start => b"(start)",
            Self::End => b"(end)",
            Self::Return => b"(return)",
        }
    }

    pub(crate) const fn len(self) -> usize {
        self.bytes().len()
    }
}
