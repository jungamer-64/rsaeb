//! Borrowed inspection views for parsed rules and payloads.
//!
//! Inspection exposes the parsed program structure without exposing the
//! internal rule table or storing a second copy of source text. Rule and payload
//! views borrow from [`Program`](crate::Program), so they are cheap to copy and
//! cannot outlive the parsed program they describe.
//!
//! ```
//! use rsaeb::limits::DEFAULT_PARSE_LIMITS;
//! use rsaeb::inspect::{RuleActionView, RuleAnchor, RuleRepeat};
//! use rsaeb::{Program, ProgramSource};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::parse(ProgramSource::from_text(
//!     "(once)(start)a=(return)done",
//! ), DEFAULT_PARSE_LIMITS)?;
//! let rule = program.rules().next().ok_or("missing rule")?;
//!
//! assert_eq!(rule.position().number().get(), 1);
//! assert_eq!(rule.repeat(), RuleRepeat::Once);
//! assert_eq!(rule.anchor(), RuleAnchor::Start);
//! assert!(rule.lhs().eq_bytes(b"a"));
//! assert!(matches!(
//!     rule.action(),
//!     RuleActionView::Return(output) if output.eq_bytes(b"done")
//! ));
//! # Ok(())
//! # }
//! ```

use alloc::vec::Vec;
use core::fmt;

use crate::allocation::{AllocationContext, AllocationError};
use crate::bytes::{Payload, PayloadByteCount};
use crate::rule::Rule;
use crate::source::SourceLineNumber;

/// Number of parsed rules.
///
/// This count is produced by a parsed program and keeps rule counts distinct
/// from byte counts and step counts.
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
///
/// Rule positions are assigned after parsing removes blank/comment-only lines.
/// Use [`RuleView::line_number`] when diagnostics need the original source
/// line instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RulePosition {
    number: RuleNumber,
}

impl RulePosition {
    pub(crate) fn from_zero_based(zero_based: usize) -> Option<Self> {
        let number = RuleNumber::from_zero_based(zero_based)?;
        Some(Self { number })
    }

    /// One-based rule number for display.
    #[must_use]
    pub const fn number(self) -> RuleNumber {
        self.number
    }
}

/// Rule repeat policy.
///
/// Repeat policy is per runtime invocation. A `(once)` rule can be used again
/// by a later call to [`Program::run`](crate::Program::run) or
/// [`Program::start_execution`](crate::Program::start_execution).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleRepeat {
    /// The rule may apply every time it matches.
    Always,
    /// The rule may apply at most once during one runtime invocation.
    Once,
}

/// Rule match anchor.
///
/// Anchors constrain where the left-side payload may match in the current
/// runtime state.
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
///
/// Each variant carries the right-side payload in the domain implied by the
/// parsed action token. There is no boolean flag that can confuse ordinary
/// replacement, movement, and return behavior.
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
#[derive(Clone, Copy)]
pub struct RuleView<'program> {
    rule: &'program Rule,
}

impl core::fmt::Debug for RuleView<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RuleView")
            .field("position", &self.position())
            .field("line_number", &self.line_number())
            .field("repeat", &self.repeat())
            .field("anchor", &self.anchor())
            .field("lhs", &self.lhs())
            .field("action", &self.action())
            .finish()
    }
}

impl PartialEq for RuleView<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.position() == other.position()
            && self.line_number() == other.line_number()
            && self.repeat() == other.repeat()
            && self.anchor() == other.anchor()
            && self.lhs() == other.lhs()
            && self.action() == other.action()
    }
}

impl Eq for RuleView<'_> {}

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
        crate::rule::canonical_source(self.rule)
    }
}
