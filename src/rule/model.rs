use crate::bytes::Payload;
use crate::inspect::{PayloadView, RuleActionView, RuleAnchor, RulePosition, RuleRepeat};
use crate::source::SourceLineNumber;

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

/// Parser-built match pattern before program-local position assignment.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParsedRulePattern {
    /// Original source line containing this rule.
    line_number: SourceLineNumber,
    /// Parsed match anchor modifier.
    anchor: RuleAnchorSyntax,
    /// Left-side executable match payload.
    lhs: Payload,
}

/// Parsed rewrite rule before program-local position assignment.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParsedRewriteRule {
    /// Positionless parsed match pattern.
    pattern: ParsedRulePattern,
    /// Right-side rewrite action.
    action: RewriteAction,
}

/// Parsed return rule before program-local position assignment.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParsedReturnRule {
    /// Positionless parsed match pattern.
    pattern: ParsedRulePattern,
    /// Right-side return output.
    output: Payload,
}

/// Internal parsed rule with repeat and terminal behavior preserved in the variant.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ParsedRule {
    /// Reusable non-terminal rewrite rule.
    AlwaysRewrite(ParsedRewriteRule),
    /// Once-only non-terminal rewrite rule.
    OnceRewrite(ParsedRewriteRule),
    /// Reusable terminal return rule.
    AlwaysReturn(ParsedReturnRule),
    /// Once-only terminal return rule.
    OnceReturn(ParsedReturnRule),
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

/// Stored match fields shared by all executable rule variants.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RulePattern {
    /// Execution-order position assigned by the parsed program.
    position: RulePosition,
    /// Original source line for diagnostics and inspection.
    line_number: SourceLineNumber,
    /// Match anchor used by the runtime matcher.
    anchor: RuleAnchorSyntax,
    /// Left-side executable match payload.
    lhs: Payload,
}

/// Stored always-available rewrite rule.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AlwaysRewriteRule {
    /// Shared executable match fields.
    pattern: RulePattern,
    /// Right-side rewrite action applied after a match.
    action: RewriteAction,
}

/// Stored once-only rewrite rule.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct OnceRewriteRule {
    /// Shared executable match fields.
    pattern: RulePattern,
    /// Right-side rewrite action applied after a match.
    action: RewriteAction,
}

/// Stored always-available return rule.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AlwaysReturnRule {
    /// Shared executable match fields.
    pattern: RulePattern,
    /// Right-side output returned after a match.
    output: Payload,
}

/// Stored once-only return rule.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct OnceReturnRule {
    /// Shared executable match fields.
    pattern: RulePattern,
    /// Right-side output returned after a match.
    output: Payload,
}

/// Internal rule split by repeat and terminal behavior.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Rule {
    /// Reusable non-terminal rewrite rule.
    AlwaysRewrite(AlwaysRewriteRule),
    /// Once-only non-terminal rewrite rule.
    OnceRewrite(OnceRewriteRule),
    /// Reusable terminal return rule.
    AlwaysReturn(AlwaysReturnRule),
    /// Once-only terminal return rule.
    OnceReturn(OnceReturnRule),
}

/// Borrowed right-side action for inspection and canonical rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleRightSide<'rule> {
    /// Non-terminal rewrite action.
    Rewrite(&'rule RewriteAction),
    /// Terminal return output.
    Return(&'rule Payload),
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

impl ParsedRulePattern {
    /// Groups parsed left-side rule fields before program-level position assignment.
    pub(crate) const fn new(
        line_number: SourceLineNumber,
        anchor: RuleAnchorSyntax,
        lhs: Payload,
    ) -> Self {
        Self {
            line_number,
            anchor,
            lhs,
        }
    }
}

impl ParsedRule {
    /// Builds a reusable rewrite rule.
    pub(crate) const fn always_rewrite(pattern: ParsedRulePattern, action: RewriteAction) -> Self {
        Self::AlwaysRewrite(ParsedRewriteRule { pattern, action })
    }

    /// Builds a once-only rewrite rule.
    pub(crate) const fn once_rewrite(pattern: ParsedRulePattern, action: RewriteAction) -> Self {
        Self::OnceRewrite(ParsedRewriteRule { pattern, action })
    }

    /// Builds a reusable return rule.
    pub(crate) const fn always_return(pattern: ParsedRulePattern, output: Payload) -> Self {
        Self::AlwaysReturn(ParsedReturnRule { pattern, output })
    }

    /// Builds a once-only return rule.
    pub(crate) const fn once_return(pattern: ParsedRulePattern, output: Payload) -> Self {
        Self::OnceReturn(ParsedReturnRule { pattern, output })
    }

    /// Source line used for diagnostics and public inspection.
    pub(crate) const fn line_number(&self) -> SourceLineNumber {
        self.pattern().line_number
    }

    /// Borrows the positionless match pattern.
    const fn pattern(&self) -> &ParsedRulePattern {
        match self {
            Self::AlwaysRewrite(rule) | Self::OnceRewrite(rule) => &rule.pattern,
            Self::AlwaysReturn(rule) | Self::OnceReturn(rule) => &rule.pattern,
        }
    }
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

impl RulePattern {
    /// Assigns program-local position to parsed match fields.
    fn from_parsed(position: RulePosition, pattern: ParsedRulePattern) -> Self {
        Self {
            position,
            line_number: pattern.line_number,
            anchor: pattern.anchor,
            lhs: pattern.lhs,
        }
    }
}

impl AlwaysRewriteRule {
    /// Combines shared match fields with a rewrite action.
    const fn from_parts(pattern: RulePattern, action: RewriteAction) -> Self {
        Self { pattern, action }
    }

    /// Shared executable match fields.
    pub(crate) const fn pattern(&self) -> &RulePattern {
        &self.pattern
    }

    /// Right-side rewrite action.
    pub(crate) const fn action(&self) -> &RewriteAction {
        &self.action
    }
}

impl OnceRewriteRule {
    /// Combines shared match fields with a rewrite action.
    const fn from_parts(pattern: RulePattern, action: RewriteAction) -> Self {
        Self { pattern, action }
    }

    /// Shared executable match fields.
    pub(crate) const fn pattern(&self) -> &RulePattern {
        &self.pattern
    }

    /// Right-side rewrite action.
    pub(crate) const fn action(&self) -> &RewriteAction {
        &self.action
    }
}

impl AlwaysReturnRule {
    /// Combines shared match fields with a return output.
    const fn from_parts(pattern: RulePattern, output: Payload) -> Self {
        Self { pattern, output }
    }

    /// Shared executable match fields.
    pub(crate) const fn pattern(&self) -> &RulePattern {
        &self.pattern
    }

    /// Right-side return output.
    pub(crate) const fn output(&self) -> &Payload {
        &self.output
    }
}

impl OnceReturnRule {
    /// Combines shared match fields with a return output.
    const fn from_parts(pattern: RulePattern, output: Payload) -> Self {
        Self { pattern, output }
    }

    /// Shared executable match fields.
    pub(crate) const fn pattern(&self) -> &RulePattern {
        &self.pattern
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
        match parsed {
            ParsedRule::AlwaysRewrite(rule) => {
                let pattern = RulePattern::from_parsed(position, rule.pattern);
                Self::AlwaysRewrite(AlwaysRewriteRule::from_parts(pattern, rule.action))
            }
            ParsedRule::OnceRewrite(rule) => {
                let pattern = RulePattern::from_parsed(position, rule.pattern);
                Self::OnceRewrite(OnceRewriteRule::from_parts(pattern, rule.action))
            }
            ParsedRule::AlwaysReturn(rule) => {
                let pattern = RulePattern::from_parsed(position, rule.pattern);
                Self::AlwaysReturn(AlwaysReturnRule::from_parts(pattern, rule.output))
            }
            ParsedRule::OnceReturn(rule) => {
                let pattern = RulePattern::from_parsed(position, rule.pattern);
                Self::OnceReturn(OnceReturnRule::from_parts(pattern, rule.output))
            }
        }
    }

    /// Shared executable match fields.
    pub(crate) const fn pattern(&self) -> &RulePattern {
        match self {
            Self::AlwaysRewrite(rule) => rule.pattern(),
            Self::OnceRewrite(rule) => rule.pattern(),
            Self::AlwaysReturn(rule) => rule.pattern(),
            Self::OnceReturn(rule) => rule.pattern(),
        }
    }

    /// Execution-order position assigned by the parsed program.
    pub(crate) const fn position(&self) -> RulePosition {
        self.pattern().position
    }

    /// Source line used for diagnostics and public inspection.
    pub(crate) const fn line_number(&self) -> SourceLineNumber {
        self.pattern().line_number
    }

    /// Public repeat policy for inspection.
    pub(crate) const fn repeat(&self) -> RuleRepeat {
        match self {
            Self::AlwaysRewrite(_) | Self::AlwaysReturn(_) => RuleRepeat::Always,
            Self::OnceRewrite(_) | Self::OnceReturn(_) => RuleRepeat::Once,
        }
    }

    /// Match anchor used by the matcher.
    pub(crate) const fn anchor(&self) -> RuleAnchorSyntax {
        self.pattern().anchor
    }

    /// Left-side executable match payload.
    pub(crate) const fn lhs(&self) -> &Payload {
        &self.pattern().lhs
    }

    /// Right-side action as a borrowed split view.
    pub(crate) const fn right_side(&self) -> RuleRightSide<'_> {
        match self {
            Self::AlwaysRewrite(rule) => RuleRightSide::Rewrite(rule.action()),
            Self::OnceRewrite(rule) => RuleRightSide::Rewrite(rule.action()),
            Self::AlwaysReturn(rule) => RuleRightSide::Return(rule.output()),
            Self::OnceReturn(rule) => RuleRightSide::Return(rule.output()),
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
