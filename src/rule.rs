use alloc::vec::Vec;
use core::fmt;
use core::iter::{DoubleEndedIterator, ExactSizeIterator};

use crate::allocation::{AllocationContext, AllocationError, try_push, try_reserve_total_exact};
use crate::bytes::{CodeByte, Payload};
use crate::syntax::SyntaxToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RulePosition {
    zero_based: usize,
}

impl RulePosition {
    pub(crate) const fn new(zero_based: usize) -> Self {
        Self { zero_based }
    }

    /// Zero-based rule position in parse order.
    #[must_use]
    pub const fn zero_based(self) -> usize {
        self.zero_based
    }

    /// One-based rule number for display.
    #[must_use]
    pub const fn one_based(self) -> usize {
        self.zero_based + 1
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OnceRulePosition {
    zero_based: usize,
}

impl OnceRulePosition {
    pub(crate) const fn new(zero_based: usize) -> Self {
        Self { zero_based }
    }

    pub(crate) const fn zero_based(self) -> usize {
        self.zero_based
    }
}

/// Rule repeat policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleRepeat {
    /// The rule may apply every time it matches.
    Always,
    /// The rule may apply at most once during one runtime invocation.
    Once,
}

impl RuleRepeat {
    /// Whether this repeat policy is `(once)`.
    #[must_use]
    pub const fn is_once(self) -> bool {
        matches!(self, Self::Once)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleMode {
    Always,
    Once(OnceRulePosition),
}

impl RuleMode {
    pub(crate) const fn repeat(self) -> RuleRepeat {
        match self {
            Self::Always => RuleRepeat::Always,
            Self::Once(_) => RuleRepeat::Once,
        }
    }

    pub(crate) const fn once_position(self) -> Option<OnceRulePosition> {
        match self {
            Self::Always => None,
            Self::Once(position) => Some(position),
        }
    }
}

/// Rule match anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleAnchor {
    /// Search for the left-side payload anywhere in the runtime state.
    Anywhere,
    /// Match only at the start of the runtime state.
    Start,
    /// Match only at the end of the runtime state.
    End,
}

/// Read-only view of a program payload.
///
/// Program payload bytes are compact executable-code bytes. Whitespace,
/// comments, reserved syntax, non-ASCII bytes, and control bytes cannot exist
/// inside this view because payload construction is owned by the parser.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PayloadView<'program> {
    pub(crate) payload: &'program Payload,
}

impl<'program> PayloadView<'program> {
    pub(crate) fn new(payload: &'program Payload) -> Self {
        Self { payload }
    }

    /// Payload length in bytes.
    #[must_use]
    pub fn len(self) -> usize {
        self.payload.len()
    }

    /// Whether the payload is empty.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.payload.is_empty()
    }

    /// Iterates over payload bytes.
    #[must_use]
    pub fn bytes(self) -> impl DoubleEndedIterator<Item = u8> + ExactSizeIterator + 'program {
        self.payload.bytes().iter().copied().map(CodeByte::as_u8)
    }

    /// Returns whether this payload has exactly the expected bytes.
    #[must_use]
    pub fn eq_bytes(self, expected: &[u8]) -> bool {
        self.bytes().eq(expected.iter().copied())
    }

    pub(crate) fn to_vec_with_context(
        self,
        context: AllocationContext,
    ) -> Result<Vec<u8>, AllocationError> {
        self.payload.to_vec_with_context(context)
    }
}

impl fmt::Debug for PayloadView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries((*self).bytes()).finish()
    }
}

/// Read-only view of a parsed rule action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleActionView<'program> {
    /// Replace the matched bytes with the payload.
    Replace(PayloadView<'program>),
    /// Remove the matched bytes and insert the payload at the start.
    MoveStart(PayloadView<'program>),
    /// Remove the matched bytes and append the payload at the end.
    MoveEnd(PayloadView<'program>),
    /// Stop execution and return the payload as output.
    Return(PayloadView<'program>),
}

impl<'program> RuleActionView<'program> {
    /// Action payload bytes.
    #[must_use]
    pub const fn payload(self) -> PayloadView<'program> {
        match self {
            Self::Replace(payload)
            | Self::MoveStart(payload)
            | Self::MoveEnd(payload)
            | Self::Return(payload) => payload,
        }
    }

    /// Whether this action is `(return)`.
    #[must_use]
    pub const fn is_return(self) -> bool {
        matches!(self, Self::Return(_))
    }
}

/// Read-only structured view of a parsed rule.
///
/// The view borrows the parsed rule itself. Canonical source text is generated
/// from the structured rule when requested; it is not stored as a second source
/// of truth beside the parsed fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleView<'program> {
    position: RulePosition,
    rule: &'program Rule,
}

impl<'program> RuleView<'program> {
    pub(crate) fn new(position: RulePosition, rule: &'program Rule) -> Self {
        Self { position, rule }
    }

    /// Program-local parsed-rule position.
    #[must_use]
    pub const fn position(self) -> RulePosition {
        self.position
    }

    /// Zero-based parsed-rule position.
    #[must_use]
    pub const fn zero_based_position(self) -> usize {
        self.position.zero_based()
    }

    /// One-based source line number.
    #[must_use]
    pub fn line_number(self) -> usize {
        self.rule.line_number
    }

    /// Rule repeat policy.
    #[must_use]
    pub fn repeat(self) -> RuleRepeat {
        self.rule.mode.repeat()
    }

    /// Rule match anchor.
    #[must_use]
    pub fn anchor(self) -> RuleAnchor {
        self.rule.anchor
    }

    /// Left-side match payload.
    #[must_use]
    pub fn lhs(self) -> PayloadView<'program> {
        PayloadView::new(&self.rule.lhs)
    }

    /// Right-side action.
    #[must_use]
    pub fn action(self) -> RuleActionView<'program> {
        self.rule.action.view()
    }

    /// Generates canonical executable source for diagnostics/display.
    ///
    /// Whitespace and comments are not preserved by design. The canonical text
    /// is derived from the typed rule fields every time, so there is no stored
    /// textual metadata that can drift from the executable rule.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the canonical byte buffer cannot be
    /// allocated or if its computed length overflows `usize`.
    pub fn canonical_source(self) -> Result<Vec<u8>, AllocationError> {
        self.rule.canonical_source()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeRuleState {
    Fresh,
    Consumed,
}

impl RuntimeRuleState {
    pub(crate) const fn is_consumed(self) -> bool {
        matches!(self, Self::Consumed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Action {
    Replace(Payload),
    MoveStart(Payload),
    MoveEnd(Payload),
    Return(Payload),
}

impl Action {
    fn view(&self) -> RuleActionView<'_> {
        match self {
            Self::Replace(payload) => RuleActionView::Replace(PayloadView::new(payload)),
            Self::MoveStart(payload) => RuleActionView::MoveStart(PayloadView::new(payload)),
            Self::MoveEnd(payload) => RuleActionView::MoveEnd(PayloadView::new(payload)),
            Self::Return(payload) => RuleActionView::Return(PayloadView::new(payload)),
        }
    }

    fn canonical_parts(&self) -> (Option<SyntaxToken>, &Payload) {
        match self {
            Self::Replace(payload) => (None, payload),
            Self::MoveStart(payload) => (Some(SyntaxToken::Start), payload),
            Self::MoveEnd(payload) => (Some(SyntaxToken::End), payload),
            Self::Return(payload) => (Some(SyntaxToken::Return), payload),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rule {
    pub(crate) line_number: usize,
    pub(crate) mode: RuleMode,
    pub(crate) anchor: RuleAnchor,
    pub(crate) lhs: Payload,
    pub(crate) action: Action,
}

impl Rule {
    pub(crate) const fn once_position(&self) -> Option<OnceRulePosition> {
        self.mode.once_position()
    }

    pub(crate) fn view(&self, position: RulePosition) -> RuleView<'_> {
        RuleView::new(position, self)
    }

    fn canonical_source_len(&self) -> Result<usize, AllocationError> {
        let (action_token, payload) = self.action.canonical_parts();
        let mut len = self.lhs.len();

        if self.mode.repeat().is_once() {
            len = len.checked_add(SyntaxToken::Once.len()).ok_or_else(|| {
                AllocationError::new(AllocationContext::CanonicalSource, usize::MAX)
            })?;
        }

        let anchor_len = match self.anchor {
            RuleAnchor::Anywhere => 0,
            RuleAnchor::Start => SyntaxToken::Start.len(),
            RuleAnchor::End => SyntaxToken::End.len(),
        };

        len = len
            .checked_add(anchor_len)
            .and_then(|len| len.checked_add(1))
            .and_then(|len| len.checked_add(action_token.map_or(0, SyntaxToken::len)))
            .and_then(|len| len.checked_add(payload.len()))
            .ok_or_else(|| AllocationError::new(AllocationContext::CanonicalSource, usize::MAX))?;

        Ok(len)
    }

    fn canonical_source(&self) -> Result<Vec<u8>, AllocationError> {
        let mut output = Vec::new();
        try_reserve_total_exact(
            &mut output,
            self.canonical_source_len()?,
            AllocationContext::CanonicalSource,
        )?;

        if self.mode.repeat().is_once() {
            push_token(&mut output, SyntaxToken::Once)?;
        }

        match self.anchor {
            RuleAnchor::Anywhere => {}
            RuleAnchor::Start => push_token(&mut output, SyntaxToken::Start)?,
            RuleAnchor::End => push_token(&mut output, SyntaxToken::End)?,
        }

        push_payload(&mut output, &self.lhs)?;
        try_push(&mut output, b'=', AllocationContext::CanonicalSource)?;

        let (action_token, payload) = self.action.canonical_parts();
        if let Some(token) = action_token {
            push_token(&mut output, token)?;
        }
        push_payload(&mut output, payload)?;

        Ok(output)
    }
}

fn push_token(output: &mut Vec<u8>, token: SyntaxToken) -> Result<(), AllocationError> {
    for byte in token.bytes().iter().copied() {
        try_push(output, byte, AllocationContext::CanonicalSource)?;
    }

    Ok(())
}

fn push_payload(output: &mut Vec<u8>, payload: &Payload) -> Result<(), AllocationError> {
    for byte in payload.bytes().iter().copied() {
        try_push(output, byte.as_u8(), AllocationContext::CanonicalSource)?;
    }

    Ok(())
}
