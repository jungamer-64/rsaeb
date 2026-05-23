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

#[derive(Debug, PartialEq, Eq)]
pub(super) struct RuleSyntaxLine {
    line_number: SourceLineNumber,
    bytes: Vec<CompactByte>,
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

#[derive(Clone, Copy)]
struct RuleSideSlices<'code> {
    left: CompactSyntax<'code>,
    right: CompactSyntax<'code>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CompactSyntax<'code> {
    bytes: &'code [CompactByte],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CompactLineIndex {
    zero_based: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RuleSeparator {
    equals: CompactLineIndex,
    right_start: CompactLineIndex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RuleSides {
    separator: RuleSeparator,
}

impl CompactLineIndex {
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

    const fn get(self) -> usize {
        self.zero_based
    }
}

impl<'code> CompactSyntax<'code> {
    const fn new(bytes: &'code [CompactByte]) -> Self {
        Self { bytes }
    }

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

    const fn equals(self) -> CompactLineIndex {
        self.equals
    }

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

fn invalid_rule_side_range(line_number: SourceLineNumber) -> ParseError {
    ParseError::at_line(
        line_number,
        ParseErrorKind::InternalInvariant(ParseInvariantError::invalid_rule_side_range()),
    )
}

#[derive(Clone, Copy)]
struct LeftSyntax<'code> {
    line_number: SourceLineNumber,
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

#[derive(Clone, Copy)]
struct LeftAfterRepeat<'code> {
    line_number: SourceLineNumber,
    bytes: CompactSyntax<'code>,
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

#[derive(Clone, Copy)]
struct LeftPayloadSyntax<'code> {
    line_number: SourceLineNumber,
    bytes: CompactSyntax<'code>,
    repeat: RuleRepeatSyntax,
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

#[derive(Clone, Copy)]
struct RightSyntax<'code> {
    line_number: SourceLineNumber,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RightActionSyntax {
    MoveStart,
    MoveEnd,
    Return,
}

impl RightActionSyntax {
    const fn payload_kind(self) -> PayloadKind {
        match self {
            Self::MoveStart => PayloadKind::RightSideMoveStartPayload,
            Self::MoveEnd => PayloadKind::RightSideMoveEndPayload,
            Self::Return => PayloadKind::RightSideReturnPayload,
        }
    }

    fn into_body(self, payload: Payload) -> RuleBody {
        let action = match self {
            Self::MoveStart => RuleAction::Rewrite(RewriteAction::MoveStart(payload)),
            Self::MoveEnd => RuleAction::Rewrite(RewriteAction::MoveEnd(payload)),
            Self::Return => RuleAction::Return(payload),
        };

        RuleBody::new(action)
    }
}

#[derive(Clone, Copy)]
enum RightPayloadSyntax<'code> {
    Replace(RightReplacePayloadSyntax<'code>),
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

#[derive(Clone, Copy)]
struct RightReplacePayloadSyntax<'code> {
    line_number: SourceLineNumber,
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

#[derive(Clone, Copy)]
struct RightActionPayloadSyntax<'code> {
    line_number: SourceLineNumber,
    bytes: CompactSyntax<'code>,
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
    if attempted_len.get() <= limit.get() {
        return Ok(());
    }

    Err(ParseError::at_line(
        line_number,
        ParseErrorKind::Limit(ParseLimitError::payload(limit, attempted_len)),
    ))
}

fn first_matching_token_kind<T: Copy>(
    input: CompactSyntax<'_>,
    mappings: &[(SyntaxToken, T)],
) -> Option<T> {
    mappings
        .iter()
        .find_map(|&(token, kind)| input.starts_with_token(token).then_some(kind))
}

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
