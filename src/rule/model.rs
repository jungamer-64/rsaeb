use crate::bytes::Payload;
use crate::inspect::{
    OnceRuleCount, PayloadView, RuleAction, RuleAnchor, RulePosition, RuleRepeat,
};
use crate::source::SourceLineNumber;

/// Parsed right-side action after syntax has been assigned a domain.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ParsedRuleAction {
    /// Rewrite the runtime state and optionally continue.
    Rewrite(RewriteAction),
    /// Stop execution and materialize the payload as return output.
    Return(Payload),
}

impl ParsedRuleAction {
    /// Borrows the runtime state as a public byte view.
    pub(crate) fn view(&self) -> RuleAction<PayloadView<'_>> {
        match self {
            Self::Rewrite(action) => action.view(),
            Self::Return(payload) => RuleAction::Return(PayloadView::new(payload)),
        }
    }

    /// Borrows the right-side shape used for canonical source generation.
    pub(crate) const fn canonical_right_side(&self) -> CanonicalRightSide<'_> {
        match self {
            Self::Rewrite(action) => action.canonical_right_side(),
            Self::Return(payload) => CanonicalRightSide::Return(payload),
        }
    }
}

/// Parsed non-return rewrite action.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RewriteAction {
    /// Replace the matched payload at the match position.
    Replace(Payload),
    /// Move replacement payload to the start of runtime state.
    MoveStart(Payload),
    /// Move replacement payload to the end of runtime state.
    MoveEnd(Payload),
}

impl RewriteAction {
    /// Borrows the runtime state as a public byte view.
    pub(crate) fn view(&self) -> RuleAction<PayloadView<'_>> {
        match self {
            Self::Replace(payload) => RuleAction::Replace(PayloadView::new(payload)),
            Self::MoveStart(payload) => RuleAction::MoveStart(PayloadView::new(payload)),
            Self::MoveEnd(payload) => RuleAction::MoveEnd(PayloadView::new(payload)),
        }
    }

    /// Borrows the rewrite shape used for canonical source generation.
    pub(crate) const fn canonical_right_side(&self) -> CanonicalRightSide<'_> {
        match self {
            Self::Replace(payload) => CanonicalRightSide::Replace(payload),
            Self::MoveStart(payload) => CanonicalRightSide::MoveStart(payload),
            Self::MoveEnd(payload) => CanonicalRightSide::MoveEnd(payload),
        }
    }

    /// Payload emitted by this rewrite action.
    pub(crate) const fn payload(&self) -> &Payload {
        match self {
            Self::Replace(payload) | Self::MoveStart(payload) | Self::MoveEnd(payload) => payload,
        }
    }
}

/// Right-side syntax shape used by canonical source generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CanonicalRightSide<'rule> {
    /// Plain replacement payload.
    Replace(&'rule Payload),
    /// `(start)` action payload.
    MoveStart(&'rule Payload),
    /// `(end)` action payload.
    MoveEnd(&'rule Payload),
    /// `(return)` action payload.
    Return(&'rule Payload),
}

/// Internal rule head.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RuleHead {
    /// Parsed repeat modifier.
    repeat: RuleRepeatSyntax,
    /// Parsed match anchor modifier.
    anchor: RuleAnchorSyntax,
    /// Left-side executable match payload.
    lhs: Payload,
}

impl RuleHead {
    /// Groups parsed left-side rule fields before once-slot assignment.
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
    /// Parsed right-side action.
    action: ParsedRuleAction,
}

impl RuleBody {
    /// Wraps the parsed right-side action.
    pub(crate) const fn new(action: ParsedRuleAction) -> Self {
        Self { action }
    }
}

/// Internal parsed rule.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParsedRule {
    /// Original source line containing this rule.
    line_number: SourceLineNumber,
    /// Parsed left-side rule fields.
    head: RuleHead,
    /// Parsed right-side rule action.
    body: RuleBody,
}

impl ParsedRule {
    /// Combines parsed rule parts before program-level repeat-state assignment.
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

    /// Source line used for diagnostics and public inspection.
    pub(crate) const fn line_number(&self) -> SourceLineNumber {
        self.line_number
    }

    /// Repeat syntax before `(once)` slot assignment.
    pub(crate) const fn repeat_syntax(&self) -> RuleRepeatSyntax {
        self.head.repeat
    }
}

/// Repeat modifier as it appears in parsed syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleRepeatSyntax {
    /// Rule has no `(once)` modifier.
    Always,
    /// Rule has a `(once)` modifier and needs a run-local slot.
    Once,
}

/// Match anchor as it appears in parsed syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleAnchorSyntax {
    /// No anchor modifier; search every possible match position.
    Anywhere,
    /// `(start)` modifier; match only at the beginning.
    Start,
    /// `(end)` modifier; match only at the end.
    End,
}

impl RuleAnchorSyntax {
    /// Converts parser syntax into the public inspection anchor.
    pub(crate) const fn public_anchor(self) -> RuleAnchor {
        match self {
            Self::Anywhere => RuleAnchor::Anywhere,
            Self::Start => RuleAnchor::Start,
            Self::End => RuleAnchor::End,
        }
    }
}

/// Parser-assigned `(once)` slot used by one runtime invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct OnceRuleSlot {
    /// Zero-based slot in the per-run `(once)` state table.
    zero_based: usize,
}

impl OnceRuleSlot {
    /// Assigns the next slot from the current parsed `(once)` count.
    pub(crate) const fn from_count(count: OnceRuleCount) -> Self {
        Self {
            zero_based: count.get(),
        }
    }

    /// Zero-based slot index.
    pub(crate) const fn index(self) -> usize {
        self.zero_based
    }
}

/// Runtime availability assigned after program-level rule construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleAvailability {
    /// Rule can apply on every match.
    Always,
    /// Rule can apply once per run and owns this parser-assigned runtime slot.
    Once(OnceRuleSlot),
}

impl RuleAvailability {
    /// Converts internal repeat state into the public inspection repeat.
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
    /// Execution-order position assigned by the parsed program.
    position: RulePosition,
    /// Original source line for diagnostics and inspection.
    line_number: SourceLineNumber,
    /// Runtime availability for this rule.
    availability: RuleAvailability,
    /// Match anchor used by the runtime matcher.
    anchor: RuleAnchorSyntax,
    /// Left-side executable match payload.
    lhs: Payload,
    /// Right-side action applied after a match.
    action: ParsedRuleAction,
}

impl Rule {
    /// Assigns execution position and runtime repeat state to a parsed rule.
    pub(crate) fn from_parsed(
        position: RulePosition,
        parsed: ParsedRule,
        availability: RuleAvailability,
    ) -> Self {
        Self {
            position,
            line_number: parsed.line_number,
            availability,
            anchor: parsed.head.anchor,
            lhs: parsed.head.lhs,
            action: parsed.body.action,
        }
    }

    /// Execution-order position assigned by the parsed program.
    pub(crate) const fn position(&self) -> RulePosition {
        self.position
    }

    /// Source line used for diagnostics and public inspection.
    pub(crate) const fn line_number(&self) -> SourceLineNumber {
        self.line_number
    }

    /// Public repeat policy for inspection.
    pub(crate) const fn repeat(&self) -> RuleRepeat {
        self.availability.public_repeat()
    }

    /// Runtime availability used by the matcher.
    pub(crate) const fn availability(&self) -> RuleAvailability {
        self.availability
    }

    /// Match anchor used by the matcher.
    pub(crate) const fn anchor(&self) -> RuleAnchorSyntax {
        self.anchor
    }

    /// Left-side executable match payload.
    pub(crate) const fn lhs(&self) -> &Payload {
        &self.lhs
    }

    /// Right-side action applied after a match.
    pub(crate) const fn action(&self) -> &ParsedRuleAction {
        &self.action
    }
}
