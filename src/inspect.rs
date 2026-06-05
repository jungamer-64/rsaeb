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
//! use rsaeb::inspect::{RuleAnchor, RuleView};
//! use rsaeb::policy::DefaultParsePolicy;
//! use rsaeb::program::ExecutableProgram;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let executable = ExecutableProgram::parse_text::<DefaultParsePolicy>(
//!     "(once)(start)a=(return)done",
//! )?;
//! let rule = executable.rules().next().ok_or("missing rule")?;
//!
//! if rule.position().get() != 1 {
//!     return Err("unexpected rule position".into());
//! }
//! if rule.anchor() != RuleAnchor::Start {
//!     return Err("unexpected rule anchor".into());
//! }
//! if rule.lhs().materialize()?.as_slice() != b"a" {
//!     return Err("unexpected left side".into());
//! }
//! match rule {
//!     RuleView::OnceReturn(return_rule) => {
//!         if return_rule.output().materialize()?.as_slice() != b"done" {
//!             return Err("unexpected return output".into());
//!         }
//!     }
//!     RuleView::AlwaysRewrite(_)
//!     | RuleView::OnceRewrite(_)
//!     | RuleView::AlwaysReturn(_) => return Err("expected once return rule".into()),
//! }
//! # Ok(())
//! # }
//! ```

use alloc::vec::Vec;
use core::fmt;
use core::num::NonZeroUsize;

use crate::allocation::{AllocationContext, AllocationError};
use crate::bytes::{Payload, PayloadByteCount};
use crate::limits::SourceByteCount;
use crate::materialized::{CanonicalRuleSourceDomain, MaterializedBytes, PayloadInspectionDomain};
use crate::rule::{ReturnRule, RewriteRule};
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

    /// Returns the checked next rule count.
    pub(crate) fn checked_next(self) -> Option<Self> {
        let value = self.value.checked_add(1)?;
        Some(Self { value })
    }
}

/// Non-zero number of parsed executable rules.
///
/// This count is produced only by [`program::ExecutableProgram`](crate::program::ExecutableProgram).
/// Empty programs do not expose a zero-valued executable count; their empty
/// topology is represented by the program type itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExecutableRuleCount {
    /// Non-zero total executable rule count.
    value: NonZeroUsize,
}

impl ExecutableRuleCount {
    /// ONE boundary value.
    pub(crate) const ONE: Self = Self {
        value: NonZeroUsize::MIN,
    };

    /// Builds a non-zero executable count from a general rule count.
    pub(crate) fn from_rule_count(count: RuleCount) -> Option<Self> {
        Some(Self {
            value: NonZeroUsize::new(count.get())?,
        })
    }

    /// Returns this executable count as a general parsed-rule count.
    pub(crate) const fn as_rule_count(self) -> RuleCount {
        RuleCount {
            value: self.value.get(),
        }
    }

    /// Returns the checked next general parsed-rule count.
    pub(crate) fn checked_next_rule_count(self) -> Option<RuleCount> {
        self.as_rule_count().checked_next()
    }

    /// Non-zero parsed-rule count as a primitive value.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value.get()
    }
}

/// Program-local position of a parsed rule in execution order.
///
/// Rule positions are assigned after parsing removes blank/comment-only lines.
/// Use [`RuleView::line_number`] when diagnostics need the original source
/// line instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RulePosition {
    /// One-based execution-order position.
    one_based: NonZeroUsize,
}

impl RulePosition {
    /// FIRST boundary value.
    pub(crate) const FIRST: Self = Self {
        one_based: NonZeroUsize::MIN,
    };

    /// Builds a position from a non-zero executable-rule count.
    pub(crate) const fn from_executable_count(count: ExecutableRuleCount) -> Self {
        Self {
            one_based: count.value,
        }
    }

    /// One-based rule position as a primitive value.
    #[must_use]
    pub const fn get(self) -> usize {
        self.one_based.get()
    }
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
/// represented by [`RuleView::AlwaysReturn`] and [`RuleView::OnceReturn`]
/// instead of being mixed into rewrite action inspection.
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
/// Each variant carries both repeat behavior and terminal behavior. Callers do
/// not need to match a repeat axis and then re-match an action axis to learn
/// what a rule can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleView<'program> {
    /// Reusable non-terminal rewrite rule.
    AlwaysRewrite(AlwaysRewriteRuleView<'program>),
    /// Once-only non-terminal rewrite rule.
    OnceRewrite(OnceRewriteRuleView<'program>),
    /// Reusable terminal return rule.
    AlwaysReturn(AlwaysReturnRuleView<'program>),
    /// Once-only terminal return rule.
    OnceReturn(OnceReturnRuleView<'program>),
}

/// Read-only structured view of a reusable non-terminal rewrite rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlwaysRewriteRuleView<'program> {
    /// Execution-order position derived from the containing rule topology.
    position: RulePosition,
    /// Parsed rewrite rule borrowed from the program rule table.
    rule: &'program RewriteRule,
}

/// Read-only structured view of a once-only non-terminal rewrite rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnceRewriteRuleView<'program> {
    /// Execution-order position derived from the containing rule topology.
    position: RulePosition,
    /// Parsed rewrite rule borrowed from the program rule table.
    rule: &'program RewriteRule,
}

/// Read-only structured view of a reusable terminal return rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlwaysReturnRuleView<'program> {
    /// Execution-order position derived from the containing rule topology.
    position: RulePosition,
    /// Parsed return rule borrowed from the program rule table.
    rule: &'program ReturnRule,
}

/// Read-only structured view of a once-only terminal return rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnceReturnRuleView<'program> {
    /// Execution-order position derived from the containing rule topology.
    position: RulePosition,
    /// Parsed return rule borrowed from the program rule table.
    rule: &'program ReturnRule,
}

impl<'program> RuleView<'program> {
    /// Borrows a reusable rewrite rule.
    pub(crate) const fn from_always_rewrite(
        position: RulePosition,
        rule: &'program RewriteRule,
    ) -> Self {
        Self::AlwaysRewrite(AlwaysRewriteRuleView { position, rule })
    }

    /// Borrows a once-only rewrite rule.
    pub(crate) const fn from_once_rewrite(
        position: RulePosition,
        rule: &'program RewriteRule,
    ) -> Self {
        Self::OnceRewrite(OnceRewriteRuleView { position, rule })
    }

    /// Borrows a reusable return rule.
    pub(crate) const fn from_always_return(
        position: RulePosition,
        rule: &'program ReturnRule,
    ) -> Self {
        Self::AlwaysReturn(AlwaysReturnRuleView { position, rule })
    }

    /// Borrows a once-only return rule.
    pub(crate) const fn from_once_return(
        position: RulePosition,
        rule: &'program ReturnRule,
    ) -> Self {
        Self::OnceReturn(OnceReturnRuleView { position, rule })
    }

    /// Program-local parsed-rule position.
    #[must_use]
    pub fn position(self) -> RulePosition {
        match self {
            Self::AlwaysRewrite(rule) => rule.position(),
            Self::OnceRewrite(rule) => rule.position(),
            Self::AlwaysReturn(rule) => rule.position(),
            Self::OnceReturn(rule) => rule.position(),
        }
    }

    /// One-based source line number.
    #[must_use]
    pub fn line_number(self) -> SourceLineNumber {
        match self {
            Self::AlwaysRewrite(rule) => rule.line_number(),
            Self::OnceRewrite(rule) => rule.line_number(),
            Self::AlwaysReturn(rule) => rule.line_number(),
            Self::OnceReturn(rule) => rule.line_number(),
        }
    }

    /// Rule match anchor.
    #[must_use]
    pub fn anchor(self) -> RuleAnchor {
        match self {
            Self::AlwaysRewrite(rule) => rule.anchor(),
            Self::OnceRewrite(rule) => rule.anchor(),
            Self::AlwaysReturn(rule) => rule.anchor(),
            Self::OnceReturn(rule) => rule.anchor(),
        }
    }

    /// Left-side match payload.
    #[must_use]
    pub fn lhs(self) -> PayloadView<'program> {
        match self {
            Self::AlwaysRewrite(rule) => rule.lhs(),
            Self::OnceRewrite(rule) => rule.lhs(),
            Self::AlwaysReturn(rule) => rule.lhs(),
            Self::OnceReturn(rule) => rule.lhs(),
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
                Self::AlwaysRewrite(rule) => MaterializedBytes::from_canonical_source(
                    crate::rule::canonical_always_rewrite_source(rule.into_rule())?,
                ),
                Self::OnceRewrite(rule) => MaterializedBytes::from_canonical_source(
                    crate::rule::canonical_once_rewrite_source(rule.into_rule())?,
                ),
                Self::AlwaysReturn(rule) => MaterializedBytes::from_canonical_source(
                    crate::rule::canonical_always_return_source(rule.into_rule())?,
                ),
                Self::OnceReturn(rule) => MaterializedBytes::from_canonical_source(
                    crate::rule::canonical_once_return_source(rule.into_rule())?,
                ),
            },
        })
    }
}

/// Implements the shared read-only methods for concrete rewrite rule views.
macro_rules! impl_rewrite_rule_view {
    ($view:ident) => {
        impl<'program> $view<'program> {
            /// Borrows a stored rewrite rule with its topology-derived position.
            pub(crate) const fn new(position: RulePosition, rule: &'program RewriteRule) -> Self {
                Self { position, rule }
            }

            /// Rebuilds the borrowed internal rule for private rendering.
            pub(crate) const fn into_rule(self) -> &'program RewriteRule {
                self.rule
            }

            /// Program-local parsed-rule position.
            #[must_use]
            pub fn position(self) -> RulePosition {
                self.position
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
    };
}

/// Implements the shared read-only methods for concrete return rule views.
macro_rules! impl_return_rule_view {
    ($view:ident) => {
        impl<'program> $view<'program> {
            /// Borrows a stored return rule with its topology-derived position.
            pub(crate) const fn new(position: RulePosition, rule: &'program ReturnRule) -> Self {
                Self { position, rule }
            }

            /// Rebuilds the borrowed internal rule for private rendering.
            pub(crate) const fn into_rule(self) -> &'program ReturnRule {
                self.rule
            }

            /// Program-local parsed-rule position.
            #[must_use]
            pub fn position(self) -> RulePosition {
                self.position
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
    };
}

impl_rewrite_rule_view!(AlwaysRewriteRuleView);
impl_rewrite_rule_view!(OnceRewriteRuleView);
impl_return_rule_view!(AlwaysReturnRuleView);
impl_return_rule_view!(OnceReturnRuleView);
