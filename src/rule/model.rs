use crate::bytes::Payload;
use crate::inspect::{PayloadView, RuleActionView, RuleAnchor, RuleRepeat};
use crate::source::SourceLineNumber;

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

    pub(crate) const fn canonical_right_side(&self) -> CanonicalRightSide<'_> {
        match self {
            Self::Replace(payload) => CanonicalRightSide::Replace(payload),
            Self::MoveStart(payload) => CanonicalRightSide::MoveStart(payload),
            Self::MoveEnd(payload) => CanonicalRightSide::MoveEnd(payload),
            Self::Return(payload) => CanonicalRightSide::Return(payload),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CanonicalRightSide<'rule> {
    Replace(&'rule Payload),
    MoveStart(&'rule Payload),
    MoveEnd(&'rule Payload),
    Return(&'rule Payload),
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

    pub(crate) const fn line_number(&self) -> SourceLineNumber {
        self.line_number
    }

    pub(crate) const fn repeat(&self) -> RuleRepeat {
        self.head.repeat
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OnceRuleSlot {
    zero_based: usize,
}

impl OnceRuleSlot {
    pub(crate) const fn zero_based(self) -> usize {
        self.zero_based
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct OnceRuleCount {
    value: usize,
}

impl OnceRuleCount {
    pub(crate) const fn get(self) -> usize {
        self.value
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleRepeatState {
    Always,
    Once(OnceRuleSlot),
}

impl RuleRepeatState {
    pub(crate) const fn public_repeat(self) -> RuleRepeat {
        match self {
            Self::Always => RuleRepeat::Always,
            Self::Once(_) => RuleRepeat::Once,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Rule {
    line_number: SourceLineNumber,
    repeat: RuleRepeatState,
    anchor: RuleAnchor,
    lhs: Payload,
    action: Action,
}

impl Rule {
    pub(crate) fn from_parsed(parsed: ParsedRule, repeat: RuleRepeatState) -> Self {
        Self {
            line_number: parsed.line_number,
            repeat,
            anchor: parsed.head.anchor,
            lhs: parsed.head.lhs,
            action: parsed.body.action,
        }
    }

    pub(crate) const fn line_number(&self) -> SourceLineNumber {
        self.line_number
    }

    pub(crate) const fn repeat(&self) -> RuleRepeat {
        self.repeat.public_repeat()
    }

    pub(crate) const fn repeat_state(&self) -> RuleRepeatState {
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
}
