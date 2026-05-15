use alloc::vec::Vec;
use core::fmt;

use crate::allocation::{AllocationContext, AllocationError, try_push, try_reserve_total_exact};
use crate::bytes::{Payload, PayloadByteCount};
use crate::source::SourceLineNumber;
use crate::syntax::SyntaxToken;

/// Number of parsed rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuleCount {
    value: usize,
}

impl RuleCount {
    /// Creates a rule count from a primitive count.
    #[must_use]
    pub(crate) const fn new(value: usize) -> Self {
        Self { value }
    }

    /// Parsed-rule count as a primitive value.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }
}

/// One-based rule number for public diagnostics and display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuleNumber {
    one_based: usize,
}

impl RuleNumber {
    fn from_zero_based(zero_based: usize) -> Option<Self> {
        let one_based = zero_based.checked_add(1)?;
        Some(Self { one_based })
    }

    /// One-based rule number as a primitive value.
    #[must_use]
    pub const fn get(self) -> usize {
        self.one_based
    }
}

/// Program-local position of a parsed rule in execution order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RulePosition {
    zero_based: usize,
    number: RuleNumber,
}

impl RulePosition {
    pub(crate) fn from_zero_based(zero_based: usize) -> Option<Self> {
        let number = RuleNumber::from_zero_based(zero_based)?;
        Some(Self { zero_based, number })
    }

    /// One-based rule number for display.
    #[must_use]
    pub const fn number(self) -> RuleNumber {
        self.number
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
    payload: &'program Payload,
}

impl<'program> PayloadView<'program> {
    pub(crate) fn new(payload: &'program Payload) -> Self {
        Self { payload }
    }

    /// Payload length in bytes.
    #[must_use]
    pub fn byte_count(self) -> PayloadByteCount {
        self.payload.byte_count()
    }

    /// Whether the payload is empty.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.byte_count().is_zero()
    }

    /// Payload bytes as a materializing iterator.
    ///
    /// This intentionally does not expose a borrowed `&[u8]`: the parsed payload
    /// is stored as `ProgramByte`, not as untyped bytes. Consumers that need
    /// ownership should call `to_vec` instead of relying on hidden allocation.
    pub fn bytes(self) -> impl Iterator<Item = u8> + 'program {
        self.payload.bytes()
    }

    /// Returns whether this payload has exactly the expected bytes.
    #[must_use]
    pub fn eq_bytes(self, expected: &[u8]) -> bool {
        self.payload.eq_bytes(expected)
    }

    /// Materializes this payload as owned bytes with explicit fallible
    /// allocation.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the output buffer cannot be allocated.
    pub fn to_vec(self) -> Result<Vec<u8>, AllocationError> {
        self.to_vec_with_context(AllocationContext::PayloadView)
    }

    /// Materializes this payload view as owned bytes for the given allocation site.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the output buffer cannot be allocated.
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

/// Read-only structured view of a parsed rule.
///
/// The view borrows the parsed rule itself. Canonical source text is generated
/// from the structured rule when requested; it is not stored as a second source
/// of truth beside the parsed fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleView<'program> {
    rule: &'program Rule,
}

impl<'program> RuleView<'program> {
    pub(crate) fn new(rule: &'program Rule) -> Self {
        Self { rule }
    }

    /// Program-local parsed-rule position.
    #[must_use]
    pub const fn position(self) -> RulePosition {
        self.rule.position()
    }

    /// One-based source line number.
    #[must_use]
    pub fn line_number(self) -> SourceLineNumber {
        self.rule.line_number()
    }

    /// Rule repeat policy.
    #[must_use]
    pub fn repeat(self) -> RuleRepeat {
        self.rule.repeat()
    }

    /// Rule match anchor.
    #[must_use]
    pub fn anchor(self) -> RuleAnchor {
        self.rule.anchor()
    }

    /// Left-side match payload.
    #[must_use]
    pub fn lhs(self) -> PayloadView<'program> {
        PayloadView::new(self.rule.lhs())
    }

    /// Right-side action.
    #[must_use]
    pub fn action(self) -> RuleActionView<'program> {
        self.rule.action().view()
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

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Replace(Payload),
    MoveStart(Payload),
    MoveEnd(Payload),
    Return(Payload),
}

impl Action {
    pub(crate) fn view(&self) -> RuleActionView<'_> {
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

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RuleHead {
    repeat: RuleRepeat,
    anchor: RuleAnchor,
    lhs: Payload,
}

impl RuleHead {
    pub(crate) fn new(repeat: RuleRepeat, anchor: RuleAnchor, lhs: Payload) -> Self {
        Self {
            repeat,
            anchor,
            lhs,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RuleBody {
    action: Action,
}

impl RuleBody {
    pub(crate) const fn new(action: Action) -> Self {
        Self { action }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParsedRule {
    line_number: SourceLineNumber,
    head: RuleHead,
    body: RuleBody,
}

impl ParsedRule {
    pub(crate) const fn from_parts(
        line_number: SourceLineNumber,
        head: RuleHead,
        body: RuleBody,
    ) -> Self {
        Self {
            line_number,
            head,
            body,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Rule {
    position: RulePosition,
    line_number: SourceLineNumber,
    repeat: RuleRepeat,
    anchor: RuleAnchor,
    lhs: Payload,
    action: Action,
}

impl Rule {
    pub(crate) fn from_parsed(parsed: ParsedRule, position: RulePosition) -> Self {
        Self {
            position,
            line_number: parsed.line_number,
            repeat: parsed.head.repeat,
            anchor: parsed.head.anchor,
            lhs: parsed.head.lhs,
            action: parsed.body.action,
        }
    }

    pub(crate) const fn position(&self) -> RulePosition {
        self.position
    }

    pub(crate) const fn line_number(&self) -> SourceLineNumber {
        self.line_number
    }

    pub(crate) const fn repeat(&self) -> RuleRepeat {
        self.repeat
    }

    pub(crate) const fn anchor(&self) -> RuleAnchor {
        self.anchor
    }

    pub(crate) const fn lhs(&self) -> &Payload {
        &self.lhs
    }

    pub(crate) const fn action(&self) -> &Action {
        &self.action
    }

    pub(crate) fn view(&self) -> RuleView<'_> {
        RuleView::new(self)
    }

    /// Computes the byte length of this rule's canonical source form.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if canonical source length arithmetic
    /// overflows.
    fn canonical_source_len(&self) -> Result<usize, AllocationError> {
        let (action_token, payload) = self.action.canonical_parts();
        let mut len = self.lhs.len();

        if self.repeat == RuleRepeat::Once {
            len = len.checked_add(SyntaxToken::Once.len()).ok_or_else(|| {
                AllocationError::capacity_overflow(AllocationContext::CanonicalSource)
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
            .ok_or_else(|| {
                AllocationError::capacity_overflow(AllocationContext::CanonicalSource)
            })?;

        Ok(len)
    }

    /// Materializes this rule's canonical source form.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if canonical source length arithmetic
    /// overflows or the output buffer cannot be allocated.
    fn canonical_source(&self) -> Result<Vec<u8>, AllocationError> {
        let mut output = Vec::new();
        try_reserve_total_exact(
            &mut output,
            self.canonical_source_len()?,
            AllocationContext::CanonicalSource,
        )?;

        if self.repeat == RuleRepeat::Once {
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

/// Appends one syntax token to canonical source output.
///
/// # Errors
///
/// Returns `AllocationError` if the output buffer cannot grow.
fn push_token(output: &mut Vec<u8>, token: SyntaxToken) -> Result<(), AllocationError> {
    for byte in token.bytes().iter().copied() {
        try_push(output, byte, AllocationContext::CanonicalSource)?;
    }

    Ok(())
}

/// Appends payload bytes to canonical source output.
///
/// # Errors
///
/// Returns `AllocationError` if the output buffer cannot grow.
fn push_payload(output: &mut Vec<u8>, payload: &Payload) -> Result<(), AllocationError> {
    payload.push_bytes_to(output, AllocationContext::CanonicalSource)
}
