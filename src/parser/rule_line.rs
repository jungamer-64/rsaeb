use alloc::vec::Vec;

use crate::allocation::{AllocationContext, AllocationError};
use crate::bytes::{CompactByte, Payload, PayloadByteCount};
use crate::error::{
    LeftModifierKind, ParseError, ParseErrorKind, ParseLimitError, PayloadKind, RightActionKind,
};
use crate::inspect::{RuleAnchor, RuleRepeat};
use crate::program::PayloadByteLimit;
use crate::rule::{Action, ParsedRule, RuleBody, RuleHead};
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
    /// Returns `ParseError` if the stored side ranges no longer describe valid
    /// slices of this compact line. Construction validates that invariant, so
    /// this error path is an invariant guard rather than ordinary syntax
    /// rejection.
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
    left: &'code [CompactByte],
    right: &'code [CompactByte],
}

#[derive(Clone, Copy)]
struct LeftSyntax<'code> {
    line_number: SourceLineNumber,
    bytes: &'code [CompactByte],
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
        if let Some(rest) = strip_token(self.bytes, SyntaxToken::Once) {
            LeftAfterRepeat {
                line_number: self.line_number,
                bytes: rest,
                repeat: RuleRepeat::Once,
            }
        } else {
            LeftAfterRepeat {
                line_number: self.line_number,
                bytes: self.bytes,
                repeat: RuleRepeat::Always,
            }
        }
    }
}

#[derive(Clone, Copy)]
struct LeftAfterRepeat<'code> {
    line_number: SourceLineNumber,
    bytes: &'code [CompactByte],
    repeat: RuleRepeat,
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
        let (anchor, bytes) = if let Some(rest) = strip_token(self.bytes, SyntaxToken::Start) {
            (RuleAnchor::Start, rest)
        } else if let Some(rest) = strip_token(self.bytes, SyntaxToken::End) {
            (RuleAnchor::End, rest)
        } else {
            (RuleAnchor::Anywhere, self.bytes)
        };

        if let Some(modifier) = left_modifier_kind(bytes) {
            let column = bytes
                .first()
                .copied()
                .map(CompactByte::source_column)
                .ok_or_else(|| {
                    parse_allocation_error(
                        self.line_number,
                        AllocationError::capacity_overflow(AllocationContext::ProgramCodeLine),
                    )
                })?;
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
    bytes: &'code [CompactByte],
    repeat: RuleRepeat,
    anchor: RuleAnchor,
}

impl LeftPayloadSyntax<'_> {
    /// Parses left-side payload syntax into a typed rule head.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the left-side payload contains invalid
    /// executable payload bytes or allocation fails.
    fn parse(self, payload_limit: PayloadByteLimit) -> Result<RuleHead, ParseError> {
        ensure_payload_within_limit(self.line_number, self.bytes.len(), payload_limit)?;
        let payload = Payload::parse(self.bytes, self.line_number, PayloadKind::LeftSideData)?;
        Ok(RuleHead::new(self.repeat, self.anchor, payload))
    }
}

#[derive(Clone, Copy)]
struct RightSyntax<'code> {
    line_number: SourceLineNumber,
    bytes: &'code [CompactByte],
}

impl<'code> RightSyntax<'code> {
    /// Parses right-side syntax into a typed rule body.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if right-side action or payload syntax is invalid.
    fn parse(self, payload_limit: PayloadByteLimit) -> Result<RuleBody, ParseError> {
        self.into_payload_syntax().parse(payload_limit)
    }

    fn into_payload_syntax(self) -> RightPayloadSyntax<'code> {
        if let Some(rest) = strip_token(self.bytes, SyntaxToken::Start) {
            RightPayloadSyntax {
                line_number: self.line_number,
                bytes: rest,
                action: RightActionSyntax::MoveStart,
            }
        } else if let Some(rest) = strip_token(self.bytes, SyntaxToken::End) {
            RightPayloadSyntax {
                line_number: self.line_number,
                bytes: rest,
                action: RightActionSyntax::MoveEnd,
            }
        } else if let Some(rest) = strip_token(self.bytes, SyntaxToken::Return) {
            RightPayloadSyntax {
                line_number: self.line_number,
                bytes: rest,
                action: RightActionSyntax::Return,
            }
        } else {
            RightPayloadSyntax {
                line_number: self.line_number,
                bytes: self.bytes,
                action: RightActionSyntax::Replace,
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RightActionSyntax {
    Replace,
    MoveStart,
    MoveEnd,
    Return,
}

impl RightActionSyntax {
    const fn payload_kind(self) -> PayloadKind {
        match self {
            Self::Replace => PayloadKind::RightSideData,
            Self::MoveStart => PayloadKind::RightSideMoveStartPayload,
            Self::MoveEnd => PayloadKind::RightSideMoveEndPayload,
            Self::Return => PayloadKind::RightSideReturnPayload,
        }
    }

    fn into_body(self, payload: Payload) -> RuleBody {
        let action = match self {
            Self::Replace => Action::Replace(payload),
            Self::MoveStart => Action::MoveStart(payload),
            Self::MoveEnd => Action::MoveEnd(payload),
            Self::Return => Action::Return(payload),
        };

        RuleBody::new(action)
    }
}

#[derive(Clone, Copy)]
struct RightPayloadSyntax<'code> {
    line_number: SourceLineNumber,
    bytes: &'code [CompactByte],
    action: RightActionSyntax,
}

impl RightPayloadSyntax<'_> {
    /// Parses classified right-side payload syntax into a typed rule body.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if action syntax is nested, payload bytes are
    /// invalid, or allocation fails.
    fn parse(self, payload_limit: PayloadByteLimit) -> Result<RuleBody, ParseError> {
        if self.action != RightActionSyntax::Replace {
            reject_nested_rhs_action(self.bytes, self.line_number)?;
        }

        ensure_payload_within_limit(self.line_number, self.bytes.len(), payload_limit)?;
        let payload = Payload::parse(self.bytes, self.line_number, self.action.payload_kind())?;
        Ok(self.action.into_body(payload))
    }
}

fn ensure_payload_within_limit(
    line_number: SourceLineNumber,
    len: usize,
    limit: PayloadByteLimit,
) -> Result<(), ParseError> {
    let attempted_len = PayloadByteCount::new(len);
    if attempted_len.get() <= limit.get() {
        return Ok(());
    }

    Err(ParseError::at_line(
        line_number,
        ParseErrorKind::Limit(ParseLimitError::payload(limit, attempted_len)),
    ))
}

fn strip_token(input: &[CompactByte], token: SyntaxToken) -> Option<&[CompactByte]> {
    let token_bytes = token.bytes();

    if input.len() < token_bytes.len() {
        return None;
    }

    let starts_with_token = input
        .iter()
        .take(token_bytes.len())
        .copied()
        .map(CompactByte::as_u8)
        .eq(token_bytes.iter().copied());

    if starts_with_token {
        input.get(token_bytes.len()..)
    } else {
        None
    }
}

fn starts_with_token(input: &[CompactByte], token: SyntaxToken) -> bool {
    strip_token(input, token).is_some()
}

fn first_matching_token_kind<T: Copy>(
    input: &[CompactByte],
    mappings: &[(SyntaxToken, T)],
) -> Option<T> {
    mappings
        .iter()
        .find_map(|&(token, kind)| starts_with_token(input, token).then_some(kind))
}

fn left_modifier_kind(input: &[CompactByte]) -> Option<LeftModifierKind> {
    first_matching_token_kind(
        input,
        &[
            (SyntaxToken::Once, LeftModifierKind::Once),
            (SyntaxToken::Start, LeftModifierKind::Start),
            (SyntaxToken::End, LeftModifierKind::End),
        ],
    )
}

fn right_action_kind(input: &[CompactByte]) -> Option<RightActionKind> {
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
    input: &[CompactByte],
    line_number: SourceLineNumber,
) -> Result<(), ParseError> {
    if let Some(action) = right_action_kind(input) {
        let column = input
            .first()
            .copied()
            .map(CompactByte::source_column)
            .ok_or_else(|| {
                parse_allocation_error(
                    line_number,
                    AllocationError::capacity_overflow(AllocationContext::ProgramCodeLine),
                )
            })?;
        return Err(ParseError::at_position(
            SourcePosition::new(line_number, column),
            ParseErrorKind::UnsupportedRightActionSyntax { action },
        ));
    }

    Ok(())
}
