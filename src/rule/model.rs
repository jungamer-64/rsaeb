use crate::bytes::Payload;
use crate::inspect::{PayloadView, RewriteActionView, RuleAnchor, RulePosition};
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

/// Internal parsed rule.
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

/// Internal rule.
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

impl ParsedRewriteRule {
    /// Combines parsed match fields with a rewrite action.
    const fn from_parts(pattern: ParsedRulePattern, action: RewriteAction) -> Self {
        Self { pattern, action }
    }

    /// Source line used for diagnostics and public inspection.
    const fn line_number(&self) -> SourceLineNumber {
        self.pattern.line_number
    }

    /// Assigns execution position to this parsed rewrite rule.
    fn into_rule(self, position: RulePosition) -> RewriteRule {
        RewriteRule::from_parts(
            RulePattern::from_parsed(position, self.pattern),
            self.action,
        )
    }
}

impl ParsedReturnRule {
    /// Combines parsed match fields with return output.
    const fn from_parts(pattern: ParsedRulePattern, output: Payload) -> Self {
        Self { pattern, output }
    }

    /// Source line used for diagnostics and public inspection.
    const fn line_number(&self) -> SourceLineNumber {
        self.pattern.line_number
    }

    /// Assigns execution position to this parsed return rule.
    fn into_rule(self, position: RulePosition) -> ReturnRule {
        ReturnRule::from_parts(
            RulePattern::from_parsed(position, self.pattern),
            self.output,
        )
    }
}

impl ParsedRule {
    /// Builds a reusable rewrite rule.
    pub(crate) const fn always_rewrite(pattern: ParsedRulePattern, action: RewriteAction) -> Self {
        Self::AlwaysRewrite(ParsedRewriteRule::from_parts(pattern, action))
    }

    /// Builds a once-only rewrite rule.
    pub(crate) const fn once_rewrite(pattern: ParsedRulePattern, action: RewriteAction) -> Self {
        Self::OnceRewrite(ParsedRewriteRule::from_parts(pattern, action))
    }

    /// Builds a reusable return rule.
    pub(crate) const fn always_return(pattern: ParsedRulePattern, output: Payload) -> Self {
        Self::AlwaysReturn(ParsedReturnRule::from_parts(pattern, output))
    }

    /// Builds a once-only return rule.
    pub(crate) const fn once_return(pattern: ParsedRulePattern, output: Payload) -> Self {
        Self::OnceReturn(ParsedReturnRule::from_parts(pattern, output))
    }

    /// Source line used for diagnostics and public inspection.
    pub(crate) const fn line_number(&self) -> SourceLineNumber {
        match self {
            Self::AlwaysRewrite(rule) | Self::OnceRewrite(rule) => rule.line_number(),
            Self::AlwaysReturn(rule) | Self::OnceReturn(rule) => rule.line_number(),
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

    /// Execution-order position assigned by the parsed program.
    pub(crate) const fn position(&self) -> RulePosition {
        self.position
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
    /// Assigns execution position to a parsed rule.
    pub(crate) fn from_parsed(position: RulePosition, parsed: ParsedRule) -> Self {
        match parsed {
            ParsedRule::AlwaysRewrite(rule) => Self::AlwaysRewrite(rule.into_rule(position)),
            ParsedRule::OnceRewrite(rule) => Self::OnceRewrite(rule.into_rule(position)),
            ParsedRule::AlwaysReturn(rule) => Self::AlwaysReturn(rule.into_rule(position)),
            ParsedRule::OnceReturn(rule) => Self::OnceReturn(rule.into_rule(position)),
        }
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
}
