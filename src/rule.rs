use alloc::vec::Vec;
use core::fmt;
use core::iter::{DoubleEndedIterator, ExactSizeIterator};

use crate::bytes::Payload;

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
    pub(crate) const fn new(payload: &'program Payload) -> Self {
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
    pub fn bytes(self) -> impl DoubleEndedIterator<Item = u8> + ExactSizeIterator + 'program {
        self.payload
            .bytes()
            .iter()
            .copied()
            .map(|byte| byte.as_u8())
    }

    /// Returns whether this payload has exactly the expected bytes.
    #[must_use]
    pub fn eq_bytes(self, expected: &[u8]) -> bool {
        self.bytes().eq(expected.iter().copied())
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
/// This exposes the parsed rule shape directly. Callers do not need to parse
/// `compact_source()` again to discover the repeat policy, anchor, left-side
/// payload, or right-side action. A text blob is metadata, not the source of
/// truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleView<'program> {
    position: RulePosition,
    line_number: usize,
    repeat: RuleRepeat,
    anchor: RuleAnchor,
    lhs: PayloadView<'program>,
    action: RuleActionView<'program>,
    compact_source: &'program [u8],
}

impl<'program> RuleView<'program> {
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
    pub const fn line_number(self) -> usize {
        self.line_number
    }

    /// Rule repeat policy.
    #[must_use]
    pub const fn repeat(self) -> RuleRepeat {
        self.repeat
    }

    /// Rule match anchor.
    #[must_use]
    pub const fn anchor(self) -> RuleAnchor {
        self.anchor
    }

    /// Left-side match payload.
    #[must_use]
    pub const fn lhs(self) -> PayloadView<'program> {
        self.lhs
    }

    /// Right-side action.
    #[must_use]
    pub const fn action(self) -> RuleActionView<'program> {
        self.action
    }

    /// Whitespace-stripped executable code for this rule.
    ///
    /// This is useful for diagnostics and display. It is deliberately not the
    /// only way to inspect a parsed rule.
    #[must_use]
    pub const fn compact_source(self) -> &'program [u8] {
        self.compact_source
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
    const fn view(&self) -> RuleActionView<'_> {
        match self {
            Self::Replace(payload) => RuleActionView::Replace(PayloadView::new(payload)),
            Self::MoveStart(payload) => RuleActionView::MoveStart(PayloadView::new(payload)),
            Self::MoveEnd(payload) => RuleActionView::MoveEnd(PayloadView::new(payload)),
            Self::Return(payload) => RuleActionView::Return(PayloadView::new(payload)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rule {
    pub(crate) line_number: usize,
    pub(crate) compact_source: Vec<u8>,
    pub(crate) repeat: RuleRepeat,
    pub(crate) anchor: RuleAnchor,
    pub(crate) lhs: Payload,
    pub(crate) action: Action,
}

impl Rule {
    pub(crate) fn view<'program>(&'program self, position: RulePosition) -> RuleView<'program> {
        RuleView {
            position,
            line_number: self.line_number,
            repeat: self.repeat,
            anchor: self.anchor,
            lhs: PayloadView::new(&self.lhs),
            action: self.action.view(),
            compact_source: &self.compact_source,
        }
    }
}
