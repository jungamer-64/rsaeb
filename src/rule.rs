use alloc::vec::Vec;

use crate::bytes::Payload;

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

/// Read-only metadata for a parsed rule.
///
/// This type intentionally contains all public rule metadata directly. There is
/// no public API that accepts a rule index as authority, because a numeric index
/// cannot prove which program produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleInfo<'program> {
    position: RulePosition,
    pub(crate) line_number: usize,
    compact_source: &'program [u8],
}

impl<'program> RuleInfo<'program> {
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

    /// Whitespace-stripped executable code for this rule.
    #[must_use]
    pub const fn compact_source(self) -> &'program [u8] {
        self.compact_source
    }
}

/// Allocation site reported by fallible parser/runtime operations.
pub(crate) enum Anchor {
    Anywhere,
    Start,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleRepeat {
    Always,
    Once,
}

impl RuleRepeat {
    pub(crate) const fn is_once(self) -> bool {
        matches!(self, Self::Once)
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


pub(crate) enum Action {
    Replace(Payload),
    MoveStart(Payload),
    MoveEnd(Payload),
    Return(Payload),
}

pub(crate) struct Rule {
    pub(crate) line_number: usize,
    pub(crate) compact_source: Vec<u8>,
    pub(crate) repeat: RuleRepeat,
    pub(crate) anchor: Anchor,
    pub(crate) lhs: Payload,
    pub(crate) action: Action,
}

impl Rule {
    pub(crate) fn info<'program>(&'program self, position: RulePosition) -> RuleInfo<'program> {
        RuleInfo {
            position,
            line_number: self.line_number,
            compact_source: &self.compact_source,
        }
    }
}

/// Parsed A=B rewrite program.
///
/// A parsed program is immutable and reusable. Per-run `(once)` state lives in
/// the runtime invocation, not in this value.
