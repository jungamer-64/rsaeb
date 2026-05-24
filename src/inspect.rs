//! Borrowed inspection views for parsed rules and payloads.
//!
//! Inspection exposes the parsed program structure without exposing the
//! internal rule table or storing a second copy of source text. Rule and payload
//! views borrow from [`program::Program`](crate::program::Program), so they are cheap to copy and
//! cannot outlive the parsed program they describe.
//!
//! Materializing payload or canonical-source bytes is explicit because it can
//! allocate. Inspection views are the cheap borrowed contract; owned bytes are
//! produced only when the caller asks for them and receives an
//! [`error::AllocationError`](crate::error::AllocationError) if that boundary
//! cannot allocate.
//!
//! ```
//! use rsaeb::limits::DEFAULT_PARSE_LIMITS;
//! use rsaeb::inspect::{RuleActionView, RuleAnchor, RuleRepeat};
//! use rsaeb::program::Program;
//! use rsaeb::source::ProgramSource;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let program = Program::parse(ProgramSource::from_text(
//!     "(once)(start)a=(return)done",
//! ), DEFAULT_PARSE_LIMITS)?;
//! let rule = program.rules().next().ok_or("missing rule")?;
//!
//! if rule.position().number().get() != 1 {
//!     return Err("unexpected rule position".into());
//! }
//! if rule.repeat() != RuleRepeat::Once {
//!     return Err("unexpected rule repeat".into());
//! }
//! if rule.anchor() != RuleAnchor::Start {
//!     return Err("unexpected rule anchor".into());
//! }
//! if rule.lhs().materialize()?.as_slice() != b"a" {
//!     return Err("unexpected left side".into());
//! }
//! match rule.action() {
//!     RuleActionView::Return(output) => {
//!         if output.materialize()?.as_slice() != b"done" {
//!             return Err("unexpected return output".into());
//!         }
//!     }
//!     RuleActionView::Replace(_) | RuleActionView::MoveStart(_) | RuleActionView::MoveEnd(_) => {
//!         return Err("expected return action".into());
//!     }
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
use crate::rule::Rule;
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
/// This count is produced by the parser's once-slot assignment and remains
/// distinct from the total executable rule count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OnceRuleCount {
    /// Parsed rules that require per-run once slots.
    value: usize,
}

impl OnceRuleCount {
    /// Creates a parsed `(once)` rule count from a primitive count.
    #[must_use]
    pub(crate) const fn new(value: usize) -> Self {
        Self { value }
    }

    /// Parsed `(once)` rule count as a primitive value.
    #[must_use]
    pub const fn get(self) -> usize {
        self.value
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

/// Rule repeat policy.
///
/// Repeat policy is per runtime invocation. A `(once)` rule can be used again
/// by a later call to [`program::Program::run`](crate::program::Program::run) or
/// [`program::Program::start_run`](crate::program::Program::start_run).
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
            bytes: MaterializedBytes::from_vec(
                self.to_vec_with_context(AllocationContext::PayloadView)?,
            ),
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

/// Owned snapshot of a parsed rule action.
///
/// This is used by owned execution transitions, where the parsed program moves
/// with the transition and public rule metadata cannot borrow from a separate
/// external [`Program`](crate::program::Program).
#[derive(Debug, PartialEq, Eq)]
pub enum RuleActionSnapshot {
    /// Replace the matched bytes with the payload.
    Replace(PayloadBytes),
    /// Remove the matched bytes and insert the payload at the start.
    MoveStart(PayloadBytes),
    /// Remove the matched bytes and append the payload at the end.
    MoveEnd(PayloadBytes),
    /// Stop execution and return the payload as output.
    Return(PayloadBytes),
}

/// Read-only structured view of a parsed rule.
///
/// The view borrows the parsed rule and carries the rule's execution position.
/// Canonical source text is generated from the structured rule when requested;
/// it is not stored as a second source of truth beside the parsed fields.
#[derive(Clone, Copy)]
pub struct RuleView<'program> {
    /// Parsed rule borrowed from the program rule table.
    rule: &'program Rule,
}

/// Owned parsed-rule metadata retained by owned execution transitions.
///
/// The byte payloads are materialized explicitly at the owned execution
/// boundary. Borrowed execution transitions use [`RuleView`] instead.
#[derive(Debug, PartialEq, Eq)]
pub struct RuleSnapshot {
    /// Program-local parsed-rule position.
    position: RulePosition,
    /// One-based source line number.
    line_number: SourceLineNumber,
    /// Rule repeat policy.
    repeat: RuleRepeat,
    /// Rule match anchor.
    anchor: RuleAnchor,
    /// Materialized left-side match payload.
    lhs: PayloadBytes,
    /// Materialized right-side action payload.
    action: RuleActionSnapshot,
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
    /// Borrows a parsed rule with its stored execution-order position.
    pub(crate) const fn new(rule: &'program Rule) -> Self {
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
        self.rule.anchor().public_anchor()
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

    /// Materializes this borrowed rule into an owned snapshot.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if either rule payload cannot be materialized
    /// at the owned rule-snapshot boundary.
    pub fn to_snapshot(self) -> Result<RuleSnapshot, AllocationError> {
        let lhs = snapshot_payload(self.lhs())?;
        let action = match self.action() {
            RuleActionView::Replace(payload) => {
                RuleActionSnapshot::Replace(snapshot_payload(payload)?)
            }
            RuleActionView::MoveStart(payload) => {
                RuleActionSnapshot::MoveStart(snapshot_payload(payload)?)
            }
            RuleActionView::MoveEnd(payload) => {
                RuleActionSnapshot::MoveEnd(snapshot_payload(payload)?)
            }
            RuleActionView::Return(payload) => {
                RuleActionSnapshot::Return(snapshot_payload(payload)?)
            }
        };

        Ok(RuleSnapshot {
            position: self.position(),
            line_number: self.line_number(),
            repeat: self.repeat(),
            anchor: self.anchor(),
            lhs,
            action,
        })
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
            bytes: MaterializedBytes::from_vec(crate::rule::canonical_source(self.rule)?),
        })
    }
}

impl RuleActionSnapshot {
    /// Borrow the materialized action payload.
    #[must_use]
    pub const fn payload(&self) -> &PayloadBytes {
        match self {
            Self::Replace(payload)
            | Self::MoveStart(payload)
            | Self::MoveEnd(payload)
            | Self::Return(payload) => payload,
        }
    }
}

impl RuleSnapshot {
    /// Program-local parsed-rule position.
    #[must_use]
    pub const fn position(&self) -> RulePosition {
        self.position
    }

    /// One-based source line number.
    #[must_use]
    pub const fn line_number(&self) -> SourceLineNumber {
        self.line_number
    }

    /// Rule repeat policy.
    #[must_use]
    pub const fn repeat(&self) -> RuleRepeat {
        self.repeat
    }

    /// Rule match anchor.
    #[must_use]
    pub const fn anchor(&self) -> RuleAnchor {
        self.anchor
    }

    /// Materialized left-side match payload.
    #[must_use]
    pub const fn lhs(&self) -> &PayloadBytes {
        &self.lhs
    }

    /// Materialized right-side action payload.
    #[must_use]
    pub const fn action(&self) -> &RuleActionSnapshot {
        &self.action
    }
}

/// Materializes a payload at the owned rule-snapshot boundary.
///
/// # Errors
///
/// Returns `AllocationError` if the payload bytes cannot be materialized.
fn snapshot_payload(payload: PayloadView<'_>) -> Result<PayloadBytes, AllocationError> {
    Ok(PayloadBytes {
        bytes: MaterializedBytes::from_vec(
            payload.to_vec_with_context(AllocationContext::RuleSnapshot)?,
        ),
    })
}
