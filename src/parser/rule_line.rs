use alloc::vec::Vec;

use crate::allocation::{AllocationContext, AllocationError};
use crate::bytes::{CompactByte, Payload, PayloadByteCount, PayloadSyntax};
use crate::error::{
    LeftModifierKind, ParseError, ParseErrorKind, ParseInvariantError, ParseLimitError,
    PayloadKind, RightActionKind,
};
use crate::limits::PayloadByteLimit;
use crate::rule::{
    ParsedRule, RewriteAction, RuleAction, RuleAnchorSyntax, RuleBody, RuleHead, RuleRepeatSyntax,
};
use crate::source::{SourceLineNumber, SourcePosition};
use crate::syntax::SyntaxToken;

use super::location::parse_allocation_error;

/// Compact executable line split into left and right rule syntax.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct RuleSyntaxLine {
    /// Original source line for diagnostics.
    line_number: SourceLineNumber,
    /// Compact executable bytes for the whole rule line.
    bytes: Vec<CompactByte>,
    /// Proven separator indexes for left and right sides.
    sides: RuleSides,
}

impl RuleSyntaxLine {
    /// Splits one compact source line around its rule separator.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the line has no `=`, multiple `=` bytes,
    /// allocation fails, or separator arithmetic overflows.
    pub(super) fn new(
        line_number: SourceLineNumber,
        bytes: Vec<CompactByte>,
    ) -> Result<Self, ParseError> {
        let separator = RuleSeparator::find(line_number, &bytes)?;
        let sides = RuleSides::new(line_number, &bytes, separator)?;
        Ok(Self {
            line_number,
            bytes,
            sides,
        })
    }

    /// Parses this compact rule syntax into typed rule data.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if either rule side contains invalid modifier,
    /// action, or payload syntax.
    pub(super) fn parse(&self, payload_limit: PayloadByteLimit) -> Result<ParsedRule, ParseError> {
        let (left, right) = self.syntax_parts()?;
        let head = left.parse(payload_limit)?;
        let body = right.parse(payload_limit)?;

        Ok(ParsedRule::from_parts(self.line_number, head, body))
    }

    /// Borrows the compact line as left and right syntax slices.
    ///
    /// # Errors
    ///
    /// Returns `ParseError::InternalInvariant` if the stored rule-side witness
    /// no longer resolves inside this compact line.
    fn syntax_parts(&self) -> Result<(LeftSyntax<'_>, RightSyntax<'_>), ParseError> {
        let slices = self.sides.slices(self.line_number, &self.bytes)?;
        Ok((
            LeftSyntax {
                line_number: self.line_number,
                bytes: slices.left,
            },
            RightSyntax {
                line_number: self.line_number,
                bytes: slices.right,
            },
        ))
    }
}

/// Borrowed left and right syntax slices resolved from a separator witness.
#[derive(Clone, Copy)]
struct RuleSideSlices<'code> {
    /// Left-side syntax before `=`.
    left: CompactSyntax<'code>,
    /// Right-side syntax after `=`.
    right: CompactSyntax<'code>,
}

/// Borrowed compact syntax bytes for one parser phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CompactSyntax<'code> {
    /// Compact bytes in the current syntax domain.
    bytes: &'code [CompactByte],
}

/// Index into a compact executable line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CompactLineIndex {
    /// Zero-based byte index in compact syntax.
    zero_based: usize,
}

/// Location of the single rule separator and right-side start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RuleSeparator {
    /// Index of the `=` separator.
    equals: CompactLineIndex,
    /// First compact byte after the `=` separator.
    right_start: CompactLineIndex,
}

/// Witness that one compact line has valid rule-side ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RuleSides {
    /// Separator indexes checked against the compact line.
    separator: RuleSeparator,
}

impl CompactLineIndex {
    /// Builds an index from a zero-based offset.
    const fn from_zero_based(zero_based: usize) -> Self {
        Self { zero_based }
    }

    /// Returns the following compact-line index.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if advancing the index would overflow.
    fn checked_next(self, line_number: SourceLineNumber) -> Result<Self, ParseError> {
        let zero_based = self.zero_based.checked_add(1).ok_or_else(|| {
            parse_allocation_error(
                line_number,
                AllocationError::capacity_overflow(AllocationContext::ProgramCodeLine),
            )
        })?;
        Ok(Self { zero_based })
    }

    /// Returns the primitive stored value.
    const fn get(self) -> usize {
        self.zero_based
    }
}

impl<'code> CompactSyntax<'code> {
    /// Borrows compact bytes as a syntax slice.
    const fn new(bytes: &'code [CompactByte]) -> Self {
        Self { bytes }
    }

    /// Returns the compact bytes in this syntax domain.
    const fn as_slice(self) -> &'code [CompactByte] {
        self.bytes
    }

    /// Returns the source column of the first compact byte.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if this compact syntax slice is empty. Callers use
    /// this only after detecting a token prefix, so an empty slice here
    /// indicates an invariant failure at the syntax boundary.
    fn first_source_column(
        self,
        line_number: SourceLineNumber,
    ) -> Result<crate::source::SourceColumn, ParseError> {
        self.bytes
            .first()
            .copied()
            .map(CompactByte::source_column)
            .ok_or_else(|| {
                parse_allocation_error(
                    line_number,
                    AllocationError::capacity_overflow(AllocationContext::ProgramCodeLine),
                )
            })
    }

    /// Removes a leading syntax token when it is present.
    fn strip_token(self, token: SyntaxToken) -> Option<Self> {
        let token_bytes = token.bytes();

        if self.bytes.len() < token_bytes.len() {
            return None;
        }

        let starts_with_token = self
            .bytes
            .iter()
            .take(token_bytes.len())
            .copied()
            .map(CompactByte::as_u8)
            .eq(token_bytes.iter().copied());

        if starts_with_token {
            self.bytes.get(token_bytes.len()..).map(Self::new)
        } else {
            None
        }
    }

    /// Whether this syntax slice begins with the given token.
    fn starts_with_token(self, token: SyntaxToken) -> bool {
        self.strip_token(token).is_some()
    }
}

impl RuleSeparator {
    /// Finds the single rule separator in a compact source line.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the line has no `=`, more than one `=`, or the
    /// right-side start index cannot be represented.
    fn find(line_number: SourceLineNumber, bytes: &[CompactByte]) -> Result<Self, ParseError> {
        let mut found = None;

        for (index, byte) in bytes.iter().copied().enumerate() {
            if byte.as_u8() != b'=' {
                continue;
            }

            let equals = CompactLineIndex::from_zero_based(index);
            let right_start = equals.checked_next(line_number)?;
            if found
                .replace(Self {
                    equals,
                    right_start,
                })
                .is_some()
            {
                return Err(ParseError::at_position(
                    SourcePosition::new(line_number, byte.source_column()),
                    ParseErrorKind::MultipleEquals,
                ));
            }
        }

        found.ok_or_else(|| ParseError::at_line(line_number, ParseErrorKind::MissingEquals))
    }

    /// Index of the `=` separator.
    const fn equals(self) -> CompactLineIndex {
        self.equals
    }

    /// Index where right-side syntax begins.
    const fn right_start(self) -> CompactLineIndex {
        self.right_start
    }
}

impl RuleSides {
    /// Creates rule-side witnesses and validates them against the compact line.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the separator indexes do not describe valid
    /// side slices for this compact line.
    fn new(
        line_number: SourceLineNumber,
        bytes: &[CompactByte],
        separator: RuleSeparator,
    ) -> Result<Self, ParseError> {
        let sides = Self { separator };
        sides.slices(line_number, bytes)?;
        Ok(sides)
    }

    /// Borrows the proven left and right slices from compact line bytes.
    ///
    /// # Errors
    ///
    /// Returns `ParseError::InternalInvariant` if the separator witness no
    /// longer resolves inside the compact line bytes.
    fn slices(
        self,
        line_number: SourceLineNumber,
        bytes: &[CompactByte],
    ) -> Result<RuleSideSlices<'_>, ParseError> {
        let left = bytes
            .get(..self.separator.equals().get())
            .ok_or_else(|| invalid_rule_side_range(line_number))?;
        let right = bytes
            .get(self.separator.right_start().get()..)
            .ok_or_else(|| invalid_rule_side_range(line_number))?;

        Ok(RuleSideSlices {
            left: CompactSyntax::new(left),
            right: CompactSyntax::new(right),
        })
    }
}

/// Builds the invariant error for stale rule-side indexes.
fn invalid_rule_side_range(line_number: SourceLineNumber) -> ParseError {
    ParseError::at_line(
        line_number,
        ParseErrorKind::InternalInvariant(ParseInvariantError::invalid_rule_side_range()),
    )
}

/// Left-side syntax before repeat and anchor classification.
#[derive(Clone, Copy)]
struct LeftSyntax<'code> {
    /// Original source line for diagnostics.
    line_number: SourceLineNumber,
    /// Left-side compact syntax.
    bytes: CompactSyntax<'code>,
}

impl<'code> LeftSyntax<'code> {
    /// Parses left-side syntax into a typed rule head.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if left-side modifier order or payload syntax is
    /// invalid.
    fn parse(self, payload_limit: PayloadByteLimit) -> Result<RuleHead, ParseError> {
        self.into_after_repeat().parse(payload_limit)
    }

    /// Classifies the optional `(once)` prefix.
    fn into_after_repeat(self) -> LeftAfterRepeat<'code> {
        if let Some(rest) = self.bytes.strip_token(SyntaxToken::Once) {
            LeftAfterRepeat {
                line_number: self.line_number,
                bytes: rest,
                repeat: RuleRepeatSyntax::Once,
            }
        } else {
            LeftAfterRepeat {
                line_number: self.line_number,
                bytes: self.bytes,
                repeat: RuleRepeatSyntax::Always,
            }
        }
    }
}

/// Left-side syntax after repeat classification.
#[derive(Clone, Copy)]
struct LeftAfterRepeat<'code> {
    /// Original source line for diagnostics.
    line_number: SourceLineNumber,
    /// Remaining compact syntax after optional repeat.
    bytes: CompactSyntax<'code>,
    /// Parsed repeat modifier.
    repeat: RuleRepeatSyntax,
}

impl<'code> LeftAfterRepeat<'code> {
    /// Parses left-side syntax after optional repeat classification.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the remaining left-side syntax cannot become a
    /// valid payload with its anchor.
    fn parse(self, payload_limit: PayloadByteLimit) -> Result<RuleHead, ParseError> {
        self.into_payload_syntax()?.parse(payload_limit)
    }

    /// Classifies the left-side anchor and payload slice.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` when modifiers appear after the anchor/payload
    /// boundary or source-column lookup overflows.
    fn into_payload_syntax(self) -> Result<LeftPayloadSyntax<'code>, ParseError> {
        let (anchor, bytes) = if let Some(rest) = self.bytes.strip_token(SyntaxToken::Start) {
            (RuleAnchorSyntax::Start, rest)
        } else if let Some(rest) = self.bytes.strip_token(SyntaxToken::End) {
            (RuleAnchorSyntax::End, rest)
        } else {
            (RuleAnchorSyntax::Anywhere, self.bytes)
        };

        if let Some(modifier) = left_modifier_kind(bytes) {
            let column = bytes.first_source_column(self.line_number)?;
            return Err(ParseError::at_position(
                SourcePosition::new(self.line_number, column),
                ParseErrorKind::UnsupportedLeftModifierOrder { modifier },
            ));
        }

        Ok(LeftPayloadSyntax {
            line_number: self.line_number,
            bytes,
            repeat: self.repeat,
            anchor,
        })
    }
}

/// Left-side payload syntax after modifier classification.
#[derive(Clone, Copy)]
struct LeftPayloadSyntax<'code> {
    /// Original source line for diagnostics.
    line_number: SourceLineNumber,
    /// Payload bytes after repeat and anchor modifiers.
    bytes: CompactSyntax<'code>,
    /// Parsed repeat modifier.
    repeat: RuleRepeatSyntax,
    /// Parsed match anchor.
    anchor: RuleAnchorSyntax,
}

impl LeftPayloadSyntax<'_> {
    /// Parses left-side payload syntax into a typed rule head.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the left-side payload contains invalid
    /// executable payload bytes or allocation fails.
    fn parse(self, payload_limit: PayloadByteLimit) -> Result<RuleHead, ParseError> {
        let payload = parse_payload(
            self.bytes.as_slice(),
            self.line_number,
            PayloadKind::LeftSideData,
            payload_limit,
        )?;
        Ok(RuleHead::new(self.repeat, self.anchor, payload))
    }
}

/// Right-side syntax before action classification.
#[derive(Clone, Copy)]
struct RightSyntax<'code> {
    /// Original source line for diagnostics.
    line_number: SourceLineNumber,
    /// Right-side compact syntax.
    bytes: CompactSyntax<'code>,
}

impl<'code> RightSyntax<'code> {
    /// Parses right-side syntax into a typed rule body.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if right-side action or payload syntax is invalid.
    fn parse(self, payload_limit: PayloadByteLimit) -> Result<RuleBody, ParseError> {
        self.into_payload_syntax()?.parse(payload_limit)
    }

    /// Classifies right-side syntax into replacement payload or action payload.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if an action payload starts with another
    /// right-side action token.
    fn into_payload_syntax(self) -> Result<RightPayloadSyntax<'code>, ParseError> {
        if let Some(rest) = self.bytes.strip_token(SyntaxToken::Start) {
            return Ok(RightPayloadSyntax::Action(RightActionPayloadSyntax::new(
                self.line_number,
                rest,
                RightActionSyntax::MoveStart,
            )?));
        } else if let Some(rest) = self.bytes.strip_token(SyntaxToken::End) {
            return Ok(RightPayloadSyntax::Action(RightActionPayloadSyntax::new(
                self.line_number,
                rest,
                RightActionSyntax::MoveEnd,
            )?));
        } else if let Some(rest) = self.bytes.strip_token(SyntaxToken::Return) {
            return Ok(RightPayloadSyntax::Action(RightActionPayloadSyntax::new(
                self.line_number,
                rest,
                RightActionSyntax::Return,
            )?));
        }

        Ok(RightPayloadSyntax::Replace(RightReplacePayloadSyntax {
            line_number: self.line_number,
            bytes: self.bytes,
        }))
    }
}

/// Right-side action token classified before payload parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RightActionSyntax {
    /// `(start)` action moving output to the beginning.
    MoveStart,
    /// `(end)` action moving output to the end.
    MoveEnd,
    /// `(return)` action ending execution.
    Return,
}

impl RightActionSyntax {
    /// Payload diagnostic domain implied by this action token.
    const fn payload_kind(self) -> PayloadKind {
        match self {
            Self::MoveStart => PayloadKind::RightSideMoveStartPayload,
            Self::MoveEnd => PayloadKind::RightSideMoveEndPayload,
            Self::Return => PayloadKind::RightSideReturnPayload,
        }
    }

    /// Combines this action token with its parsed payload.
    fn into_body(self, payload: Payload) -> RuleBody {
        let action = match self {
            Self::MoveStart => RuleAction::Rewrite(RewriteAction::MoveStart(payload)),
            Self::MoveEnd => RuleAction::Rewrite(RewriteAction::MoveEnd(payload)),
            Self::Return => RuleAction::Return(payload),
        };

        RuleBody::new(action)
    }
}

/// Right-side payload domain after action-token classification.
#[derive(Clone, Copy)]
enum RightPayloadSyntax<'code> {
    /// Plain replacement payload with no action token.
    Replace(RightReplacePayloadSyntax<'code>),
    /// Payload belonging to a right-side action token.
    Action(RightActionPayloadSyntax<'code>),
}

impl RightPayloadSyntax<'_> {
    /// Parses classified right-side payload syntax into a typed rule body.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if payload bytes are invalid or allocation fails.
    fn parse(self, payload_limit: PayloadByteLimit) -> Result<RuleBody, ParseError> {
        match self {
            Self::Replace(payload) => payload.parse(payload_limit),
            Self::Action(payload) => payload.parse(payload_limit),
        }
    }
}

/// Right-side replacement payload syntax.
#[derive(Clone, Copy)]
struct RightReplacePayloadSyntax<'code> {
    /// Original source line for diagnostics.
    line_number: SourceLineNumber,
    /// Replacement payload bytes.
    bytes: CompactSyntax<'code>,
}

impl RightReplacePayloadSyntax<'_> {
    /// Parses direct replacement payload syntax into a typed rule body.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if payload bytes are invalid or allocation fails.
    fn parse(self, payload_limit: PayloadByteLimit) -> Result<RuleBody, ParseError> {
        let payload = parse_payload(
            self.bytes.as_slice(),
            self.line_number,
            PayloadKind::RightSideData,
            payload_limit,
        )?;
        Ok(RuleBody::new(RuleAction::Rewrite(RewriteAction::Replace(
            payload,
        ))))
    }
}

/// Right-side action payload syntax after nested-action rejection.
#[derive(Clone, Copy)]
struct RightActionPayloadSyntax<'code> {
    /// Original source line for diagnostics.
    line_number: SourceLineNumber,
    /// Action payload bytes.
    bytes: CompactSyntax<'code>,
    /// Action token that owns this payload.
    action: RightActionSyntax,
}

impl<'code> RightActionPayloadSyntax<'code> {
    /// Builds a right-side action payload after rejecting nested action tokens.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the action payload starts with another
    /// right-side action token.
    fn new(
        line_number: SourceLineNumber,
        bytes: CompactSyntax<'code>,
        action: RightActionSyntax,
    ) -> Result<Self, ParseError> {
        reject_nested_rhs_action(bytes, line_number)?;
        Ok(Self {
            line_number,
            bytes,
            action,
        })
    }

    /// Parses action payload syntax into a typed rule body.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if payload bytes are invalid or allocation fails.
    fn parse(self, payload_limit: PayloadByteLimit) -> Result<RuleBody, ParseError> {
        let payload = parse_payload(
            self.bytes.as_slice(),
            self.line_number,
            self.action.payload_kind(),
            payload_limit,
        )?;
        Ok(self.action.into_body(payload))
    }
}

/// Checks payload length and validates bytes before owned payload construction.
///
/// # Errors
///
/// Returns `ParseError` if the payload exceeds `limit`, contains invalid bytes,
/// or cannot allocate owned payload storage.
fn parse_payload(
    bytes: &[CompactByte],
    line_number: SourceLineNumber,
    payload_kind: PayloadKind,
    limit: PayloadByteLimit,
) -> Result<Payload, ParseError> {
    let syntax = PayloadSyntax::new(bytes, line_number, payload_kind);
    ensure_payload_within_limit(line_number, syntax.byte_count(), limit)?;
    syntax.validate()?.into_payload()
}

/// Checks one parsed payload length against parser limits.
///
/// # Errors
///
/// Returns `ParseError` if the payload length exceeds `limit`.
fn ensure_payload_within_limit(
    line_number: SourceLineNumber,
    attempted_len: PayloadByteCount,
    limit: PayloadByteLimit,
) -> Result<(), ParseError> {
    if limit.accepts(attempted_len) {
        return Ok(());
    }

    Err(ParseError::at_line(
        line_number,
        ParseErrorKind::Limit(ParseLimitError::payload(limit, attempted_len)),
    ))
}

/// Classifies the first token from an ordered token map.
fn first_matching_token_kind<T: Copy>(
    input: CompactSyntax<'_>,
    mappings: &[(SyntaxToken, T)],
) -> Option<T> {
    mappings
        .iter()
        .find_map(|&(token, kind)| input.starts_with_token(token).then_some(kind))
}

/// Classifies a left-side modifier token at the current syntax boundary.
fn left_modifier_kind(input: CompactSyntax<'_>) -> Option<LeftModifierKind> {
    first_matching_token_kind(
        input,
        &[
            (SyntaxToken::Once, LeftModifierKind::Once),
            (SyntaxToken::Start, LeftModifierKind::Start),
            (SyntaxToken::End, LeftModifierKind::End),
        ],
    )
}

/// Classifies a right-side action token at the current syntax boundary.
fn right_action_kind(input: CompactSyntax<'_>) -> Option<RightActionKind> {
    first_matching_token_kind(
        input,
        &[
            (SyntaxToken::Start, RightActionKind::Start),
            (SyntaxToken::End, RightActionKind::End),
            (SyntaxToken::Return, RightActionKind::Return),
        ],
    )
}

/// Rejects action tokens that appear inside a right-side action payload.
///
/// # Errors
///
/// Returns `ParseError` if the payload starts with another right-side action
/// token or its source column cannot be represented.
fn reject_nested_rhs_action(
    input: CompactSyntax<'_>,
    line_number: SourceLineNumber,
) -> Result<(), ParseError> {
    if let Some(action) = right_action_kind(input) {
        let column = input.first_source_column(line_number)?;
        return Err(ParseError::at_position(
            SourcePosition::new(line_number, column),
            ParseErrorKind::UnsupportedRightActionSyntax { action },
        ));
    }

    Ok(())
}

#[cfg(test)]
mod invariant_tests {
    use super::*;
    use crate::test_support::{TestFailure, ensure_matches, source_column, source_line_number};

    type TestResult = Result<(), TestFailure>;

    /// # Errors
    ///
    /// Returns `TestFailure` if invalid rule-side witnesses are not reported as
    /// structured parser invariants.
    #[test]
    fn rule_side_witness_rechecks_compact_line_range() -> TestResult {
        let line_number = source_line_number(1)?;
        let bytes = [
            CompactByte::new(b'a', source_column(1)?),
            CompactByte::new(b'=', source_column(2)?),
            CompactByte::new(b'b', source_column(3)?),
        ];
        let sides = RuleSides {
            separator: RuleSeparator {
                equals: CompactLineIndex::from_zero_based(3),
                right_start: CompactLineIndex::from_zero_based(4),
            },
        };

        let Err(error) = sides.slices(line_number, &bytes) else {
            return Err(TestFailure::message("expected invalid rule-side range"));
        };

        ensure_matches(
            matches!(
                error.kind(),
                ParseErrorKind::InternalInvariant(ParseInvariantError::InvalidRuleSideRange)
            ),
            "expected rule-side range invariant",
        )
    }
}
