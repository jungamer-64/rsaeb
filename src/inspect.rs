//! Borrowed inspection views for parsed rules and payloads.
//!
//! Inspection exposes the parsed program structure without exposing the
//! internal rule table or storing a second copy of source text. Rule and payload
//! views borrow from [`program::ExecutableProgram`](crate::program::ExecutableProgram)
//! or [`program::EmptyProgram`](crate::program::EmptyProgram), so they are cheap
//! to copy and cannot outlive the parsed program they describe.
//!
//! Materializing payload or canonical-source bytes is explicit because it can
//! allocate. Inspection views are the cheap borrowed contract; owned bytes are
//! produced only when the caller asks for them and receives an
//! [`error::AllocationError`](crate::error::AllocationError) if that boundary
//! cannot allocate.
//!
//! ```
//! use rsaeb::inspect::{RepeatRuleView, RuleAnchor, RuleView};
//! use rsaeb::policy::DefaultParsePolicy;
//! use rsaeb::program::ExecutableProgram;
//! use rsaeb::source::ExecutableProgramSource;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let executable = ExecutableProgram::<DefaultParsePolicy>::parse(ExecutableProgramSource::from_text(
//!     "(once)(start)a=(return)done",
//! ))?;
//! let rule = executable.rules().next().ok_or("missing rule")?;
//!
//! if rule.position().number().get() != 1 {
//!     return Err("unexpected rule position".into());
//! }
//! if rule.anchor() != RuleAnchor::Start {
//!     return Err("unexpected rule anchor".into());
//! }
//! if rule.lhs().materialize()?.as_slice() != b"a" {
//!     return Err("unexpected left side".into());
//! }
//! let RuleView::Once(rule) = rule else {
//!     return Err("unexpected rule repeat".into());
//! };
//! match rule {
//!     RepeatRuleView::Return(return_rule) => {
//!         if return_rule.output().materialize()?.as_slice() != b"done" {
//!             return Err("unexpected return output".into());
//!         }
//!     }
//!     RepeatRuleView::Rewrite(_) => return Err("expected return action".into()),
//! }
//! # Ok(())
//! # }
//! ```

use alloc::vec::Vec;
use core::fmt;

use crate::allocation::{AllocationContext, AllocationError};
use crate::bytes::{Payload, PayloadByteCount};
use crate::limits::SourceByteCount;
use crate::materialized::{CanonicalRuleSourceDomain, MaterializedBytes, PayloadInspectionDomain};
use crate::rule::{RepeatRule, ReturnRule, RewriteRule, Rule};
use crate::source::SourceLineNumber;

/// Number of parsed rules.
///
/// This count is produced by a parsed program and keeps rule counts distinct
/// from byte counts and step counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuleCount {
    /// Total executable rules in a parsed program.
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

/// Number of parsed `(once)` rules.
///
/// This count is produced by parsed repeat behavior and remains
/// distinct from the total executable rule count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OnceRuleCount {
    /// Parsed rules that require per-run once availability.
    value: usize,
}

impl OnceRuleCount {
    /// ZERO boundary value.
    pub(crate) const ZERO: Self = Self { value: 0 };

    /// Parsed `(once)` rule count as a primitive value.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
    }

    /// Returns the checked next result.
    pub(crate) fn checked_next(self) -> Option<Self> {
        let value = self.value.checked_add(1)?;
        Some(Self { value })
    }
}

/// One-based rule number for public diagnostics and display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuleNumber {
    /// One-based value exposed to diagnostics and callers.
    one_based: usize,
}

impl RuleNumber {
    /// Builds an index from a zero-based offset.
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
    /// One-based public number for this execution-order position.
    number: RuleNumber,
}

impl RulePosition {
    /// Builds an index from a zero-based offset.
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

/// Compile-time marker for rules that are reusable within one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlwaysRepeat {}

/// Compile-time marker for rules that may commit at most once within one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnceRepeat {}

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
    /// Parsed payload borrowed from the program rule table.
    payload: &'program Payload,
}

/// Materialized parsed payload bytes.
///
/// This value is produced at an explicit inspection boundary. It is distinct
/// unlike runtime input/state bytes because parser payload bytes are executable
/// program data.
#[derive(Debug, PartialEq, Eq)]
pub struct PayloadBytes {
    /// Owned bytes tagged as parsed payload inspection output.
    bytes: MaterializedBytes<PayloadInspectionDomain>,
}

/// Materialized canonical source for one parsed rule.
///
/// The source is generated from typed rule fields and does not preserve
/// whitespace or comments from the original program source.
#[derive(Debug, PartialEq, Eq)]
pub struct CanonicalRuleSource {
    /// Owned bytes generated from structured rule fields.
    bytes: MaterializedBytes<CanonicalRuleSourceDomain>,
}

impl<'program> PayloadView<'program> {
    /// Borrows a parsed payload for public inspection.
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

    /// Returns materialized runtime bytes.
    pub(crate) fn materialized_bytes(self) -> impl Iterator<Item = u8> + 'program {
        self.payload.bytes()
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

    /// Materializes this payload view into typed owned payload bytes.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the output buffer cannot be allocated.
    pub fn materialize(self) -> Result<PayloadBytes, AllocationError> {
        Ok(PayloadBytes {
            bytes: MaterializedBytes::from_payload_view(self)?,
        })
    }
}

impl PayloadBytes {
    /// Borrow the materialized payload bytes.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    /// Consumes this value and returns the materialized host bytes.
    #[must_use]
    pub fn into_raw_bytes(self) -> Vec<u8> {
        self.bytes.into_raw_bytes()
    }

    /// Materialized payload length in bytes.
    #[must_use]
    pub fn byte_count(&self) -> PayloadByteCount {
        PayloadByteCount::new(self.bytes.len())
    }

    /// Returns whether this materialized payload contains no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl CanonicalRuleSource {
    /// Borrow the generated canonical source bytes.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    /// Consumes this value and returns the generated source bytes.
    #[must_use]
    pub fn into_raw_bytes(self) -> Vec<u8> {
        self.bytes.into_raw_bytes()
    }

    /// Generated canonical source length in bytes.
    #[must_use]
    pub fn byte_count(&self) -> SourceByteCount {
        SourceByteCount::new(self.bytes.len())
    }

    /// Returns whether the generated canonical source contains no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl fmt::Debug for PayloadView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries((*self).materialized_bytes())
            .finish()
    }
}

/// Structured borrowed rewrite action.
///
/// Return output is intentionally absent from this enum. Terminal behavior is
/// represented by [`ReturnRuleView`] instead of being mixed into rewrite action
/// inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewriteActionView<'program> {
    /// Replace the matched bytes with the payload.
    Replace(PayloadView<'program>),
    /// Remove the matched bytes and insert the payload at the start.
    MoveStart(PayloadView<'program>),
    /// Remove the matched bytes and append the payload at the end.
    MoveEnd(PayloadView<'program>),
}

impl<'program> RewriteActionView<'program> {
    /// Borrow the payload carried by this rewrite action.
    #[must_use]
    pub const fn payload(self) -> PayloadView<'program> {
        match self {
            Self::Replace(payload) | Self::MoveStart(payload) | Self::MoveEnd(payload) => payload,
        }
    }
}

/// Read-only structured view of a parsed rule.
///
/// The outer variant carries the repeat axis, while the inner
/// [`RepeatRuleView`] carries the action axis. Callers cannot observe return
/// output through a rewrite action view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleView<'program> {
    /// Reusable parsed rule.
    Always(RepeatRuleView<'program, AlwaysRepeat>),
    /// Once-only parsed rule.
    Once(RepeatRuleView<'program, OnceRepeat>),
}

/// Read-only structured view of one repeat-axis rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatRuleView<'program, R> {
    /// Non-terminal rewrite rule.
    Rewrite(RewriteRuleView<'program, R>),
    /// Terminal return rule.
    Return(ReturnRuleView<'program, R>),
}

/// Read-only structured view of a non-terminal rewrite rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RewriteRuleView<'program, R> {
    /// Repeat-axis rule that owns this rewrite rule.
    repeat_rule: &'program RepeatRule<R>,
    /// Parsed rewrite rule borrowed from the program rule table.
    rule: &'program RewriteRule<R>,
}

/// Read-only structured view of a terminal return rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReturnRuleView<'program, R> {
    /// Repeat-axis rule that owns this return rule.
    repeat_rule: &'program RepeatRule<R>,
    /// Parsed return rule borrowed from the program rule table.
    rule: &'program ReturnRule<R>,
}

impl<'program> RuleView<'program> {
    /// Borrows a parsed rule with its stored execution-order position.
    pub(crate) fn new(rule: &'program Rule) -> Self {
        match rule {
            Rule::Always(rule) => Self::Always(RepeatRuleView::new(rule)),
            Rule::Once(rule) => Self::Once(RepeatRuleView::new(rule)),
        }
    }

    /// Borrows a typed always-repeat rule.
    pub(crate) fn from_always(rule: &'program RepeatRule<AlwaysRepeat>) -> Self {
        Self::Always(RepeatRuleView::new(rule))
    }

    /// Borrows a typed once-repeat rule.
    pub(crate) fn from_once(rule: &'program RepeatRule<OnceRepeat>) -> Self {
        Self::Once(RepeatRuleView::new(rule))
    }

    /// Program-local parsed-rule position.
    #[must_use]
    pub fn position(self) -> RulePosition {
        match self {
            Self::Always(rule) => rule.position(),
            Self::Once(rule) => rule.position(),
        }
    }

    /// One-based source line number.
    #[must_use]
    pub fn line_number(self) -> SourceLineNumber {
        match self {
            Self::Always(rule) => rule.line_number(),
            Self::Once(rule) => rule.line_number(),
        }
    }

    /// Rule match anchor.
    #[must_use]
    pub fn anchor(self) -> RuleAnchor {
        match self {
            Self::Always(rule) => rule.anchor(),
            Self::Once(rule) => rule.anchor(),
        }
    }

    /// Left-side match payload.
    #[must_use]
    pub fn lhs(self) -> PayloadView<'program> {
        match self {
            Self::Always(rule) => rule.lhs(),
            Self::Once(rule) => rule.lhs(),
        }
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
    /// allocated or if its computed length overflows.
    pub fn canonical_source(self) -> Result<CanonicalRuleSource, AllocationError> {
        Ok(CanonicalRuleSource {
            bytes: match self {
                Self::Always(rule) => MaterializedBytes::from_canonical_source(
                    crate::rule::canonical_always_source(rule.into_rule())?,
                ),
                Self::Once(rule) => MaterializedBytes::from_canonical_source(
                    crate::rule::canonical_once_source(rule.into_rule())?,
                ),
            },
        })
    }
}

impl<'program, R> RepeatRuleView<'program, R> {
    /// Borrows a parsed repeat-axis rule with its action shape preserved.
    pub(crate) fn new(rule: &'program RepeatRule<R>) -> Self {
        match rule {
            RepeatRule::Rewrite(rewrite) => Self::Rewrite(RewriteRuleView::new(rule, rewrite)),
            RepeatRule::Return(return_rule) => Self::Return(ReturnRuleView::new(rule, return_rule)),
        }
    }

    /// Returns the borrowed internal rule.
    pub(crate) const fn into_rule(self) -> &'program RepeatRule<R> {
        match self {
            Self::Rewrite(rule) => rule.into_repeat_rule(),
            Self::Return(rule) => rule.into_repeat_rule(),
        }
    }

    /// Program-local parsed-rule position.
    #[must_use]
    pub fn position(self) -> RulePosition {
        match self {
            Self::Rewrite(rule) => rule.position(),
            Self::Return(rule) => rule.position(),
        }
    }

    /// One-based source line number.
    #[must_use]
    pub fn line_number(self) -> SourceLineNumber {
        match self {
            Self::Rewrite(rule) => rule.line_number(),
            Self::Return(rule) => rule.line_number(),
        }
    }

    /// Rule match anchor.
    #[must_use]
    pub fn anchor(self) -> RuleAnchor {
        match self {
            Self::Rewrite(rule) => rule.anchor(),
            Self::Return(rule) => rule.anchor(),
        }
    }

    /// Left-side match payload.
    #[must_use]
    pub fn lhs(self) -> PayloadView<'program> {
        match self {
            Self::Rewrite(rule) => rule.lhs(),
            Self::Return(rule) => rule.lhs(),
        }
    }
}

impl<'program, R> RewriteRuleView<'program, R> {
    /// Borrows a parsed rewrite rule.
    pub(crate) const fn new(
        repeat_rule: &'program RepeatRule<R>,
        rule: &'program RewriteRule<R>,
    ) -> Self {
        Self { repeat_rule, rule }
    }

    /// Rebuilds the repeat-axis view for this rewrite rule.
    const fn into_repeat_rule(self) -> &'program RepeatRule<R> {
        self.repeat_rule
    }

    /// Program-local parsed-rule position.
    #[must_use]
    pub fn position(self) -> RulePosition {
        self.rule.pattern().position()
    }

    /// One-based source line number.
    #[must_use]
    pub fn line_number(self) -> SourceLineNumber {
        self.rule.pattern().line_number()
    }

    /// Rule match anchor.
    #[must_use]
    pub fn anchor(self) -> RuleAnchor {
        self.rule.pattern().anchor().public_anchor()
    }

    /// Left-side match payload.
    #[must_use]
    pub fn lhs(self) -> PayloadView<'program> {
        PayloadView::new(self.rule.pattern().lhs())
    }

    /// Right-side rewrite action.
    #[must_use]
    pub fn rewrite_action(self) -> RewriteActionView<'program> {
        self.rule.rewrite_action().view()
    }
}

impl<'program, R> ReturnRuleView<'program, R> {
    /// Borrows a parsed return rule.
    pub(crate) const fn new(
        repeat_rule: &'program RepeatRule<R>,
        rule: &'program ReturnRule<R>,
    ) -> Self {
        Self { repeat_rule, rule }
    }

    /// Rebuilds the repeat-axis view for this return rule.
    const fn into_repeat_rule(self) -> &'program RepeatRule<R> {
        self.repeat_rule
    }

    /// Program-local parsed-rule position.
    #[must_use]
    pub fn position(self) -> RulePosition {
        self.rule.pattern().position()
    }

    /// One-based source line number.
    #[must_use]
    pub fn line_number(self) -> SourceLineNumber {
        self.rule.pattern().line_number()
    }

    /// Rule match anchor.
    #[must_use]
    pub fn anchor(self) -> RuleAnchor {
        self.rule.pattern().anchor().public_anchor()
    }

    /// Left-side match payload.
    #[must_use]
    pub fn lhs(self) -> PayloadView<'program> {
        PayloadView::new(self.rule.pattern().lhs())
    }

    /// Return output payload.
    #[must_use]
    pub fn output(self) -> PayloadView<'program> {
        PayloadView::new(self.rule.output())
    }
}
