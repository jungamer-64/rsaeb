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

/// Internal parsed rule.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ParsedRule {
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

/// Internal rule.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Rule {
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
