use alloc::vec::Vec;

use crate::allocation::{AllocationContext, AllocationError, try_push, try_reserve_total_exact};
use crate::bytes::{CompactByte, Payload};
use crate::error::{LeftModifierKind, ParseError, ParseErrorKind, PayloadKind, RightActionKind};
use crate::rule::{Action, ParsedRule, RuleAnchor, RuleBody, RuleHead, RuleRepeat};
use crate::source::{SourceLineNumber, SourcePosition};
use crate::syntax::SyntaxToken;

use super::location::parse_allocation_error;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct RuleSyntaxLine {
    line_number: SourceLineNumber,
    left: Vec<CompactByte>,
    right: Vec<CompactByte>,
}

impl RuleSyntaxLine {
    pub(super) fn new(
        line_number: SourceLineNumber,
        bytes: Vec<CompactByte>,
    ) -> Result<Self, ParseError> {
        let equals = EqualsPosition::find(line_number, &bytes)?;
        let parts = equals.split(line_number, bytes)?;
        Ok(Self {
            line_number,
            left: parts.left,
            right: parts.right,
        })
    }

    pub(super) fn parse(&self) -> Result<ParsedRule, ParseError> {
        let (left, right) = self.syntax_parts();
        let head = left.parse()?;
        let body = right.parse()?;

        Ok(ParsedRule::from_parts(self.line_number, head, body))
    }

    fn syntax_parts(&self) -> (LeftSyntax<'_>, RightSyntax<'_>) {
        (
            LeftSyntax {
                line_number: self.line_number,
                bytes: &self.left,
            },
            RightSyntax {
                line_number: self.line_number,
                bytes: &self.right,
            },
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EqualsPosition {
    equals_index: usize,
    right_start: usize,
}

struct RuleSyntaxParts {
    left: Vec<CompactByte>,
    right: Vec<CompactByte>,
}

impl EqualsPosition {
    fn find(line_number: SourceLineNumber, bytes: &[CompactByte]) -> Result<Self, ParseError> {
        let mut found = None;

        for (index, byte) in bytes.iter().copied().enumerate() {
            if byte.as_u8() != b'=' {
                continue;
            }

            let right_start = index.checked_add(1).ok_or_else(|| {
                parse_allocation_error(
                    line_number,
                    AllocationError::capacity_overflow(AllocationContext::ProgramCodeLine),
                )
            })?;

            if found
                .replace(Self {
                    equals_index: index,
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

    fn split(
        self,
        line_number: SourceLineNumber,
        bytes: Vec<CompactByte>,
    ) -> Result<RuleSyntaxParts, ParseError> {
        let right_len = bytes.len().checked_sub(self.right_start).ok_or_else(|| {
            parse_allocation_error(
                line_number,
                AllocationError::capacity_overflow(AllocationContext::ProgramCodeLine),
            )
        })?;

        let mut parts = RuleSyntaxParts::new(line_number, self.equals_index, right_len)?;
        for (index, byte) in bytes.into_iter().enumerate() {
            if index < self.equals_index {
                parts.push_left(line_number, byte)?;
            } else if index >= self.right_start {
                parts.push_right(line_number, byte)?;
            }
        }

        Ok(parts)
    }
}

impl RuleSyntaxParts {
    fn new(
        line_number: SourceLineNumber,
        left_capacity: usize,
        right_capacity: usize,
    ) -> Result<Self, ParseError> {
        let mut left = Vec::new();
        try_reserve_total_exact(&mut left, left_capacity, AllocationContext::ProgramCodeLine)
            .map_err(|error| parse_allocation_error(line_number, error))?;

        let mut right = Vec::new();
        try_reserve_total_exact(
            &mut right,
            right_capacity,
            AllocationContext::ProgramCodeLine,
        )
        .map_err(|error| parse_allocation_error(line_number, error))?;

        Ok(Self { left, right })
    }

    fn push_left(
        &mut self,
        line_number: SourceLineNumber,
        byte: CompactByte,
    ) -> Result<(), ParseError> {
        push_syntax_byte(line_number, &mut self.left, byte)
    }

    fn push_right(
        &mut self,
        line_number: SourceLineNumber,
        byte: CompactByte,
    ) -> Result<(), ParseError> {
        push_syntax_byte(line_number, &mut self.right, byte)
    }
}

fn push_syntax_byte(
    line_number: SourceLineNumber,
    output: &mut Vec<CompactByte>,
    byte: CompactByte,
) -> Result<(), ParseError> {
    try_push(output, byte, AllocationContext::ProgramCodeLine)
        .map_err(|error| parse_allocation_error(line_number, error))
}

#[derive(Clone, Copy)]
struct LeftSyntax<'code> {
    line_number: SourceLineNumber,
    bytes: &'code [CompactByte],
}

impl<'code> LeftSyntax<'code> {
    fn parse(self) -> Result<RuleHead, ParseError> {
        self.into_after_repeat().parse()
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
    fn parse(self) -> Result<RuleHead, ParseError> {
        self.into_payload_syntax()?.parse()
    }

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
    fn parse(self) -> Result<RuleHead, ParseError> {
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
    fn parse(self) -> Result<RuleBody, ParseError> {
        self.into_payload_syntax().parse()
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
    fn parse(self) -> Result<RuleBody, ParseError> {
        if self.action != RightActionSyntax::Replace {
            reject_nested_rhs_action(self.bytes, self.line_number)?;
        }

        let payload = Payload::parse(self.bytes, self.line_number, self.action.payload_kind())?;
        Ok(self.action.into_body(payload))
    }
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
