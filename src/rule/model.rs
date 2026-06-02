use crate::bytes::Payload;
use crate::inspect::{PayloadView, RuleActionView, RuleAnchor, RulePosition, RuleRepeat};
use crate::source::SourceLineNumber;

/// Parsed right-side action after syntax has been assigned a domain.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ParsedRuleAction {
    /// Rewrite the runtime state and optionally continue.
    Rewrite(RewriteAction),
    /// Stop execution and materialize the payload as return output.
    Return(Payload),
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
    pub(crate) fn view(&self) -> RuleActionView<'_> {
        match self {
            Self::Replace(payload) => RuleActionView::Replace(PayloadView::new(payload)),
            Self::MoveStart(payload) => RuleActionView::MoveStart(PayloadView::new(payload)),
            Self::MoveEnd(payload) => RuleActionView::MoveEnd(PayloadView::new(payload)),
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
    /// Parsed repeat behavior.
    repeat: RuleRepeatBehavior,
    /// Parsed match anchor modifier.
    anchor: RuleAnchorSyntax,
    /// Left-side executable match payload.
    lhs: Payload,
}

impl RuleHead {
    /// Groups parsed left-side rule fields before program-level rule insertion.
    pub(crate) fn new(repeat: RuleRepeatBehavior, anchor: RuleAnchorSyntax, lhs: Payload) -> Self {
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

    /// Parsed repeat behavior for this rule.
    pub(crate) const fn repeat_behavior(&self) -> RuleRepeatBehavior {
        self.head.repeat
    }
}

/// Repeat behavior parsed for one rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleRepeatBehavior {
    /// Rule has no `(once)` modifier.
    Always,
    /// Rule has a `(once)` modifier and needs run-local availability state.
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

impl RuleRepeatBehavior {
    /// Converts internal repeat state into the public inspection repeat.
    pub(crate) const fn public_repeat(self) -> RuleRepeat {
        match self {
            Self::Always => RuleRepeat::Always,
            Self::Once => RuleRepeat::Once,
        }
    }
}

/// Match fields shared by rewrite and return rules.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RuleMatch {
    /// Execution-order position assigned by the parsed program.
    position: RulePosition,
    /// Original source line for diagnostics and inspection.
    line_number: SourceLineNumber,
    /// Runtime repeat behavior for this rule.
    repeat: RuleRepeatBehavior,
    /// Match anchor used by the runtime matcher.
    anchor: RuleAnchorSyntax,
    /// Left-side executable match payload.
    lhs: Payload,
}

/// Internal parsed rewrite rule.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RewriteRule {
    /// Shared executable match fields.
    matcher: RuleMatch,
    /// Right-side rewrite action applied after a match.
    action: RewriteAction,
}

/// Internal parsed return rule.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ReturnRule {
    /// Shared executable match fields.
    matcher: RuleMatch,
    /// Right-side output returned after a match.
    output: Payload,
}

/// Internal rule split by terminal behavior.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Rule {
    /// Non-terminal rewrite rule.
    Rewrite(RewriteRule),
    /// Terminal return rule.
    Return(ReturnRule),
}

/// Borrowed right-side action for inspection and canonical rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleRightSide<'rule> {
    /// Non-terminal rewrite action.
    Rewrite(&'rule RewriteAction),
    /// Terminal return output.
    Return(&'rule Payload),
}

impl RuleMatch {
    /// Assigns execution position to parsed match fields.
    fn from_head(position: RulePosition, line_number: SourceLineNumber, head: RuleHead) -> Self {
        Self {
            position,
            line_number,
            repeat: head.repeat,
            anchor: head.anchor,
            lhs: head.lhs,
        }
    }
}

impl RewriteRule {
    /// Combines shared match fields with a rewrite action.
    fn from_parts(matcher: RuleMatch, action: RewriteAction) -> Self {
        Self { matcher, action }
    }

    /// Shared executable match fields.
    pub(crate) const fn matcher(&self) -> &RuleMatch {
        &self.matcher
    }

    /// Right-side rewrite action.
    pub(crate) const fn action(&self) -> &RewriteAction {
        &self.action
    }
}

impl ReturnRule {
    /// Combines shared match fields with a return output.
    fn from_parts(matcher: RuleMatch, output: Payload) -> Self {
        Self { matcher, output }
    }

    /// Shared executable match fields.
    pub(crate) const fn matcher(&self) -> &RuleMatch {
        &self.matcher
    }

    /// Right-side return output.
    pub(crate) const fn output(&self) -> &Payload {
        &self.output
    }
}

impl<'rule> RuleRightSide<'rule> {
    /// Borrows the right-side action as a public inspection view.
    pub(crate) fn view(self) -> RuleActionView<'rule> {
        match self {
            Self::Rewrite(action) => action.view(),
            Self::Return(payload) => RuleActionView::Return(PayloadView::new(payload)),
        }
    }

    /// Borrows the right-side shape used for canonical source generation.
    pub(crate) const fn canonical_right_side(self) -> CanonicalRightSide<'rule> {
        match self {
            Self::Rewrite(action) => action.canonical_right_side(),
            Self::Return(payload) => CanonicalRightSide::Return(payload),
        }
    }
}

impl Rule {
    /// Assigns execution position to a parsed rule.
    pub(crate) fn from_parsed(position: RulePosition, parsed: ParsedRule) -> Self {
        let ParsedRule {
            line_number,
            head,
            body,
        } = parsed;
        let matcher = RuleMatch::from_head(position, line_number, head);
        match body.action {
            ParsedRuleAction::Rewrite(action) => {
                Self::Rewrite(RewriteRule::from_parts(matcher, action))
            }
            ParsedRuleAction::Return(output) => {
                Self::Return(ReturnRule::from_parts(matcher, output))
            }
        }
    }

    /// Shared executable match fields.
    const fn matcher(&self) -> &RuleMatch {
        match self {
            Self::Rewrite(rule) => rule.matcher(),
            Self::Return(rule) => rule.matcher(),
        }
    }

    /// Execution-order position assigned by the parsed program.
    pub(crate) const fn position(&self) -> RulePosition {
        self.matcher().position
    }

    /// Source line used for diagnostics and public inspection.
    pub(crate) const fn line_number(&self) -> SourceLineNumber {
        self.matcher().line_number
    }

    /// Public repeat policy for inspection.
    pub(crate) const fn repeat(&self) -> RuleRepeat {
        self.repeat_behavior().public_repeat()
    }

    /// Runtime repeat behavior used by the matcher.
    pub(crate) const fn repeat_behavior(&self) -> RuleRepeatBehavior {
        self.matcher().repeat
    }

    /// Match anchor used by the matcher.
    pub(crate) const fn anchor(&self) -> RuleAnchorSyntax {
        self.matcher().anchor
    }

    /// Left-side executable match payload.
    pub(crate) const fn lhs(&self) -> &Payload {
        &self.matcher().lhs
    }

    /// Right-side action as a borrowed split view.
    pub(crate) const fn right_side(&self) -> RuleRightSide<'_> {
        match self {
            Self::Rewrite(rule) => RuleRightSide::Rewrite(rule.action()),
            Self::Return(rule) => RuleRightSide::Return(rule.output()),
        }
    }

    /// Borrows the right-side action as a public inspection view.
    pub(crate) fn action_view(&self) -> RuleActionView<'_> {
        self.right_side().view()
    }

    /// Borrows the right-side shape used for canonical source generation.
    pub(crate) const fn canonical_right_side(&self) -> CanonicalRightSide<'_> {
        self.right_side().canonical_right_side()
    }
}
