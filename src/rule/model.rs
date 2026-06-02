use core::marker::PhantomData;

use crate::bytes::Payload;
use crate::inspect::{
    AlwaysRepeat, OnceRepeat, PayloadView, RewriteActionView, RuleAnchor, RulePosition,
};
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
pub(crate) struct ParsedRewriteRule<R> {
    /// Positionless parsed match pattern.
    pattern: ParsedRulePattern,
    /// Right-side rewrite action.
    action: RewriteAction,
    /// Compile-time repeat axis carried by the parsed rule.
    repeat: PhantomData<fn() -> R>,
}

/// Parsed return rule before program-local position assignment.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParsedReturnRule<R> {
    /// Positionless parsed match pattern.
    pattern: ParsedRulePattern,
    /// Right-side return output.
    output: Payload,
    /// Compile-time repeat axis carried by the parsed rule.
    repeat: PhantomData<fn() -> R>,
}

/// Parsed rule under one repeat axis.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ParsedRepeatRule<R> {
    /// Parsed non-terminal rewrite rule.
    Rewrite(ParsedRewriteRule<R>),
    /// Parsed terminal return rule.
    Return(ParsedReturnRule<R>),
}

/// Internal parsed rule with repeat behavior preserved as the outer axis.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ParsedRule {
    /// Reusable rule.
    Always(ParsedRepeatRule<AlwaysRepeat>),
    /// Once-only rule.
    Once(ParsedRepeatRule<OnceRepeat>),
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

/// Stored rewrite rule for one repeat axis.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RewriteRule<R> {
    /// Shared executable match fields.
    pattern: RulePattern,
    /// Right-side rewrite action applied after a match.
    action: RewriteAction,
    /// Compile-time repeat axis carried by this rewrite rule.
    repeat: PhantomData<fn() -> R>,
}

/// Stored return rule for one repeat axis.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ReturnRule<R> {
    /// Shared executable match fields.
    pattern: RulePattern,
    /// Right-side output returned after a match.
    output: Payload,
    /// Compile-time repeat axis carried by this return rule.
    repeat: PhantomData<fn() -> R>,
}

/// Stored rule under one repeat axis.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RepeatRule<R> {
    /// Non-terminal rewrite rule.
    Rewrite(RewriteRule<R>),
    /// Terminal return rule.
    Return(ReturnRule<R>),
}

/// Internal rule split first by repeat behavior, then by terminal behavior.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Rule {
    /// Reusable rule.
    Always(RepeatRule<AlwaysRepeat>),
    /// Once-only rule.
    Once(RepeatRule<OnceRepeat>),
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

impl<R> ParsedRewriteRule<R> {
    /// Combines parsed match fields with a rewrite action under one repeat axis.
    const fn from_parts(pattern: ParsedRulePattern, action: RewriteAction) -> Self {
        Self {
            pattern,
            action,
            repeat: PhantomData,
        }
    }
}

impl<R> ParsedReturnRule<R> {
    /// Combines parsed match fields with return output under one repeat axis.
    const fn from_parts(pattern: ParsedRulePattern, output: Payload) -> Self {
        Self {
            pattern,
            output,
            repeat: PhantomData,
        }
    }
}

impl<R> ParsedRepeatRule<R> {
    /// Builds a parsed rewrite rule under this repeat axis.
    const fn rewrite(pattern: ParsedRulePattern, action: RewriteAction) -> Self {
        Self::Rewrite(ParsedRewriteRule::from_parts(pattern, action))
    }

    /// Builds a parsed return rule under this repeat axis.
    const fn return_rule(pattern: ParsedRulePattern, output: Payload) -> Self {
        Self::Return(ParsedReturnRule::from_parts(pattern, output))
    }

    /// Source line used for diagnostics and public inspection.
    const fn line_number(&self) -> SourceLineNumber {
        self.pattern().line_number
    }

    /// Borrows the positionless match pattern.
    const fn pattern(&self) -> &ParsedRulePattern {
        match self {
            Self::Rewrite(rule) => &rule.pattern,
            Self::Return(rule) => &rule.pattern,
        }
    }

    /// Assigns execution position to a parsed rule under this repeat axis.
    fn into_rule(self, position: RulePosition) -> RepeatRule<R> {
        match self {
            Self::Rewrite(rule) => {
                let pattern = RulePattern::from_parsed(position, rule.pattern);
                RepeatRule::Rewrite(RewriteRule::from_parts(pattern, rule.action))
            }
            Self::Return(rule) => {
                let pattern = RulePattern::from_parsed(position, rule.pattern);
                RepeatRule::Return(ReturnRule::from_parts(pattern, rule.output))
            }
        }
    }
}

impl ParsedRule {
    /// Builds a reusable rewrite rule.
    pub(crate) const fn always_rewrite(pattern: ParsedRulePattern, action: RewriteAction) -> Self {
        Self::Always(ParsedRepeatRule::rewrite(pattern, action))
    }

    /// Builds a once-only rewrite rule.
    pub(crate) const fn once_rewrite(pattern: ParsedRulePattern, action: RewriteAction) -> Self {
        Self::Once(ParsedRepeatRule::rewrite(pattern, action))
    }

    /// Builds a reusable return rule.
    pub(crate) const fn always_return(pattern: ParsedRulePattern, output: Payload) -> Self {
        Self::Always(ParsedRepeatRule::return_rule(pattern, output))
    }

    /// Builds a once-only return rule.
    pub(crate) const fn once_return(pattern: ParsedRulePattern, output: Payload) -> Self {
        Self::Once(ParsedRepeatRule::return_rule(pattern, output))
    }

    /// Source line used for diagnostics and public inspection.
    pub(crate) const fn line_number(&self) -> SourceLineNumber {
        match self {
            Self::Always(rule) => rule.line_number(),
            Self::Once(rule) => rule.line_number(),
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

impl<R> RewriteRule<R> {
    /// Combines shared match fields with a rewrite action.
    const fn from_parts(pattern: RulePattern, action: RewriteAction) -> Self {
        Self {
            pattern,
            action,
            repeat: PhantomData,
        }
    }

    /// Shared executable match fields.
    pub(crate) const fn pattern(&self) -> &RulePattern {
        &self.pattern
    }

    /// Right-side rewrite action.
    pub(crate) const fn rewrite_action(&self) -> &RewriteAction {
        &self.action
    }
}

impl<R> ReturnRule<R> {
    /// Combines shared match fields with a return output.
    const fn from_parts(pattern: RulePattern, output: Payload) -> Self {
        Self {
            pattern,
            output,
            repeat: PhantomData,
        }
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

impl<R> RepeatRule<R> {
    /// Shared executable match fields.
    pub(crate) const fn pattern(&self) -> &RulePattern {
        match self {
            Self::Rewrite(rule) => rule.pattern(),
            Self::Return(rule) => rule.pattern(),
        }
    }

    /// Match anchor used by the matcher.
    pub(crate) const fn anchor(&self) -> RuleAnchorSyntax {
        self.pattern().anchor()
    }

    /// Left-side executable match payload.
    pub(crate) const fn lhs(&self) -> &Payload {
        self.pattern().lhs()
    }

    /// Borrows the right-side shape used for canonical source generation.
    pub(crate) const fn canonical_action(&self) -> CanonicalRightSide<'_> {
        match self {
            Self::Rewrite(rule) => rule.rewrite_action().canonical_action(),
            Self::Return(rule) => CanonicalRightSide::Return(rule.output()),
        }
    }
}

impl Rule {
    /// Assigns execution position to a parsed rule.
    pub(crate) fn from_parsed(position: RulePosition, parsed: ParsedRule) -> Self {
        match parsed {
            ParsedRule::Always(rule) => Self::Always(rule.into_rule(position)),
            ParsedRule::Once(rule) => Self::Once(rule.into_rule(position)),
        }
    }

    /// Shared executable match fields.
    pub(crate) const fn pattern(&self) -> &RulePattern {
        match self {
            Self::Always(rule) => rule.pattern(),
            Self::Once(rule) => rule.pattern(),
        }
    }

    /// Source line used for diagnostics and public inspection.
    pub(crate) const fn line_number(&self) -> SourceLineNumber {
        self.pattern().line_number()
    }
}
