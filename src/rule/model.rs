use crate::bytes::Payload;
use crate::inspect::{PayloadView, RuleActionView, RuleAnchor, RuleRepeat};
use crate::source::SourceLineNumber;

/// Internal rule action alternatives.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RuleAction {
    /// Rewrite case.
    Rewrite(RewriteAction),
    /// Return case.
    Return(Payload),
}

impl RuleAction {
    /// Borrows the runtime state as a public byte view.
    pub(crate) fn view(&self) -> RuleActionView<'_> {
        match self {
            Self::Rewrite(action) => action.view(),
            Self::Return(payload) => RuleActionView::Return(PayloadView::new(payload)),
        }
    }

    /// Runs the canonical right side operation.
    pub(crate) const fn canonical_right_side(&self) -> CanonicalRightSide<'_> {
        match self {
            Self::Rewrite(action) => action.canonical_right_side(),
            Self::Return(payload) => CanonicalRightSide::Return(payload),
        }
    }
}

/// Internal rewrite action alternatives.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RewriteAction {
    /// Replace case.
    Replace(Payload),
    /// Move start case.
    MoveStart(Payload),
    /// Move end case.
    MoveEnd(Payload),
}

impl RewriteAction {
    /// Borrows the runtime state as a public byte view.
    pub(crate) fn view(&self) -> RuleActionView<'_> {
        match self {
            Self::Replace(payload) => RuleActionView::Replace(PayloadView::new(payload)),
            Self::MoveStart(payload) => RuleActionView::MoveStart(PayloadView::new(payload)),
            Self::MoveEnd(payload) => RuleActionView::MoveEnd(PayloadView::new(payload)),
        }
    }

    /// Runs the canonical right side operation.
    pub(crate) const fn canonical_right_side(&self) -> CanonicalRightSide<'_> {
        match self {
            Self::Replace(payload) => CanonicalRightSide::Replace(payload),
            Self::MoveStart(payload) => CanonicalRightSide::MoveStart(payload),
            Self::MoveEnd(payload) => CanonicalRightSide::MoveEnd(payload),
        }
    }

    /// Runs the payload operation.
    pub(crate) const fn payload(&self) -> &Payload {
        match self {
            Self::Replace(payload) | Self::MoveStart(payload) | Self::MoveEnd(payload) => payload,
        }
    }
}

/// Internal canonical right side alternatives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CanonicalRightSide<'rule> {
    /// Replace case.
    Replace(&'rule Payload),
    /// Move start case.
    MoveStart(&'rule Payload),
    /// Move end case.
    MoveEnd(&'rule Payload),
    /// Return case.
    Return(&'rule Payload),
}

/// Internal rule head.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RuleHead {
    /// Stored repeat.
    repeat: RuleRepeatSyntax,
    /// Stored anchor.
    anchor: RuleAnchorSyntax,
    /// Stored lhs.
    lhs: Payload,
}

impl RuleHead {
    /// Constructs the value from validated parts.
    pub(crate) fn new(repeat: RuleRepeatSyntax, anchor: RuleAnchorSyntax, lhs: Payload) -> Self {
        Self {
            repeat,
            anchor,
            lhs,
        }
    }
}

/// Internal rule body.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RuleBody {
    /// Stored action.
    action: RuleAction,
}

impl RuleBody {
    /// Constructs the value from validated parts.
    pub(crate) const fn new(action: RuleAction) -> Self {
        Self { action }
    }
}

/// Internal parsed rule.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParsedRule {
    /// Stored line number.
    line_number: SourceLineNumber,
    /// Stored head.
    head: RuleHead,
    /// Stored body.
    body: RuleBody,
}

impl ParsedRule {
    /// Builds the value from parts input.
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

    /// Runs the line number operation.
    pub(crate) const fn line_number(&self) -> SourceLineNumber {
        self.line_number
    }

    /// Runs the repeat syntax operation.
    pub(crate) const fn repeat_syntax(&self) -> RuleRepeatSyntax {
        self.head.repeat
    }
}

/// Internal rule repeat syntax alternatives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleRepeatSyntax {
    /// Always case.
    Always,
    /// Once case.
    Once,
}

/// Internal rule anchor syntax alternatives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleAnchorSyntax {
    /// Anywhere case.
    Anywhere,
    /// Start case.
    Start,
    /// End case.
    End,
}

impl RuleAnchorSyntax {
    /// Runs the public anchor operation.
    pub(crate) const fn public_anchor(self) -> RuleAnchor {
        match self {
            Self::Anywhere => RuleAnchor::Anywhere,
            Self::Start => RuleAnchor::Start,
            Self::End => RuleAnchor::End,
        }
    }
}

/// Internal once rule slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OnceRuleSlot {
    /// Stored zero based.
    zero_based: usize,
}

impl OnceRuleSlot {
    /// Runs the zero based operation.
    pub(crate) const fn zero_based(self) -> usize {
        self.zero_based
    }
}

/// Internal once rule count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct OnceRuleCount {
    /// Stored value.
    value: usize,
}

impl OnceRuleCount {
    /// Returns the primitive stored value.
    pub(crate) const fn get(self) -> usize {
        self.value
    }

    /// Runs the reserve next slot operation.
    pub(crate) fn reserve_next_slot(self) -> Option<(OnceRuleSlot, Self)> {
        let next = self.value.checked_add(1)?;
        Some((
            OnceRuleSlot {
                zero_based: self.value,
            },
            Self { value: next },
        ))
    }
}

/// Internal rule repeat state alternatives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleRepeatState {
    /// Always case.
    Always,
    /// Once case.
    Once(OnceRuleSlot),
}

impl RuleRepeatState {
    /// Runs the public repeat operation.
    pub(crate) const fn public_repeat(self) -> RuleRepeat {
        match self {
            Self::Always => RuleRepeat::Always,
            Self::Once(_) => RuleRepeat::Once,
        }
    }
}

/// Internal rule.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Rule {
    /// Stored line number.
    line_number: SourceLineNumber,
    /// Stored repeat.
    repeat: RuleRepeatState,
    /// Stored anchor.
    anchor: RuleAnchorSyntax,
    /// Stored lhs.
    lhs: Payload,
    /// Stored action.
    action: RuleAction,
}

impl Rule {
    /// Builds the value from parsed input.
    pub(crate) fn from_parsed(parsed: ParsedRule, repeat: RuleRepeatState) -> Self {
        Self {
            line_number: parsed.line_number,
            repeat,
            anchor: parsed.head.anchor,
            lhs: parsed.head.lhs,
            action: parsed.body.action,
        }
    }

    /// Runs the line number operation.
    pub(crate) const fn line_number(&self) -> SourceLineNumber {
        self.line_number
    }

    /// Runs the repeat operation.
    pub(crate) const fn repeat(&self) -> RuleRepeat {
        self.repeat.public_repeat()
    }

    /// Runs the repeat state operation.
    pub(crate) const fn repeat_state(&self) -> RuleRepeatState {
        self.repeat
    }

    /// Runs the anchor operation.
    pub(crate) const fn anchor(&self) -> RuleAnchorSyntax {
        self.anchor
    }

    /// Runs the lhs operation.
    pub(crate) const fn lhs(&self) -> &Payload {
        &self.lhs
    }

    /// Runs the action operation.
    pub(crate) const fn action(&self) -> &RuleAction {
        &self.action
    }
}
