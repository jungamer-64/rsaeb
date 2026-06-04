use crate::bytes::Payload;
use crate::inspect::{PayloadView, RewriteActionView, RuleAnchor};
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

/// Positionless match fields shared by all executable rule variants.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RulePattern {
    /// Original source line for diagnostics and inspection.
    line_number: SourceLineNumber,
    /// Match anchor used by the runtime matcher.
    anchor: RuleAnchorSyntax,
    /// Left-side executable match payload.
    lhs: Payload,
}

/// Stored rewrite rule without repeat-axis erasure.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RewriteRule {
    /// Shared executable match fields.
    pattern: RulePattern,
    /// Right-side rewrite action applied after a match.
    action: RewriteAction,
}

/// Stored return rule without repeat-axis erasure.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ReturnRule {
    /// Shared executable match fields.
    pattern: RulePattern,
    /// Right-side output returned after a match.
    output: Payload,
}

/// Positionless executable rule produced directly by the parser.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Rule {
    /// Reusable non-terminal rewrite rule.
    AlwaysRewrite(RewriteRule),
    /// Once-only non-terminal rewrite rule.
    OnceRewrite(RewriteRule),
    /// Reusable terminal return rule.
    AlwaysReturn(ReturnRule),
    /// Once-only terminal return rule.
    OnceReturn(ReturnRule),
}

impl RewriteAction {
    /// Borrows the rewrite action as a public rewrite-only inspection view.
    pub(crate) fn view(&self) -> RewriteActionView<'_> {
        match self {
            Self::Replace(payload) => RewriteActionView::Replace(PayloadView::new(payload)),
            Self::MoveStart(payload) => RewriteActionView::MoveStart(PayloadView::new(payload)),
            Self::MoveEnd(payload) => RewriteActionView::MoveEnd(PayloadView::new(payload)),
        }
    }

    /// Borrows the rewrite shape used for canonical source generation.
    pub(crate) const fn canonical_action(&self) -> CanonicalRightSide<'_> {
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
    /// Groups parsed left-side rule fields.
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

    /// Source line used for diagnostics and public inspection.
    pub(crate) const fn line_number(&self) -> SourceLineNumber {
        self.line_number
    }

    /// Match anchor used by the matcher.
    pub(crate) const fn anchor(&self) -> RuleAnchorSyntax {
        self.anchor
    }

    /// Left-side executable match payload.
    pub(crate) const fn lhs(&self) -> &Payload {
        &self.lhs
    }
}

impl RewriteRule {
    /// Combines shared match fields with a rewrite action.
    const fn from_parts(pattern: RulePattern, action: RewriteAction) -> Self {
        Self { pattern, action }
    }

    /// Shared executable match fields.
    pub(crate) const fn pattern(&self) -> &RulePattern {
        &self.pattern
    }

    /// Right-side rewrite action.
    pub(crate) const fn rewrite_action(&self) -> &RewriteAction {
        &self.action
    }

    /// Borrows the right-side shape used for canonical source generation.
    pub(crate) const fn canonical_action(&self) -> CanonicalRightSide<'_> {
        self.action.canonical_action()
    }
}

impl ReturnRule {
    /// Combines shared match fields with return output.
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

    /// Borrows the right-side shape used for canonical source generation.
    pub(crate) const fn canonical_action(&self) -> CanonicalRightSide<'_> {
        CanonicalRightSide::Return(&self.output)
    }
}

impl Rule {
    /// Builds a reusable rewrite rule.
    pub(crate) const fn always_rewrite(pattern: RulePattern, action: RewriteAction) -> Self {
        Self::AlwaysRewrite(RewriteRule::from_parts(pattern, action))
    }

    /// Builds a once-only rewrite rule.
    pub(crate) const fn once_rewrite(pattern: RulePattern, action: RewriteAction) -> Self {
        Self::OnceRewrite(RewriteRule::from_parts(pattern, action))
    }

    /// Builds a reusable return rule.
    pub(crate) const fn always_return(pattern: RulePattern, output: Payload) -> Self {
        Self::AlwaysReturn(ReturnRule::from_parts(pattern, output))
    }

    /// Builds a once-only return rule.
    pub(crate) const fn once_return(pattern: RulePattern, output: Payload) -> Self {
        Self::OnceReturn(ReturnRule::from_parts(pattern, output))
    }

    /// Shared executable match fields.
    pub(crate) const fn pattern(&self) -> &RulePattern {
        match self {
            Self::AlwaysRewrite(rule) | Self::OnceRewrite(rule) => rule.pattern(),
            Self::AlwaysReturn(rule) | Self::OnceReturn(rule) => rule.pattern(),
        }
    }

    /// Source line used for diagnostics and public inspection.
    pub(crate) const fn line_number(&self) -> SourceLineNumber {
        self.pattern().line_number()
    }

    /// Returns whether this rule has once-only runtime availability.
    pub(crate) const fn is_once(&self) -> bool {
        matches!(self, Self::OnceRewrite(_) | Self::OnceReturn(_))
    }
}
