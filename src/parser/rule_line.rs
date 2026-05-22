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

    const fn len(self) -> usize {
        self.bytes.len()
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
    /// Returns `ParseError` if the stored separator no longer fits the compact
    /// line. Normal construction prevents this; the error keeps the invariant
    /// checked without panicking.
    fn slices(
        self,
        line_number: SourceLineNumber,
        bytes: &[CompactByte],
    ) -> Result<RuleSideSlices<'_>, ParseError> {
        let left = bytes.get(..self.separator.equals().get()).ok_or_else(|| {
            parse_allocation_error(
                line_number,
                AllocationError::capacity_overflow(AllocationContext::ProgramCodeLine),
            )
        })?;
        let right = bytes
            .get(self.separator.right_start().get()..)
            .ok_or_else(|| {
                parse_allocation_error(
                    line_number,
                    AllocationError::capacity_overflow(AllocationContext::ProgramCodeLine),
                )
            })?;

        Ok(RuleSideSlices {
            left: CompactSyntax::new(left),
            right: CompactSyntax::new(right),
        })
    }
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
    bytes: CompactSyntax<'code>,
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
        let (anchor, bytes) = if let Some(rest) = self.bytes.strip_token(SyntaxToken::Start) {
            (RuleAnchor::Start, rest)
        } else if let Some(rest) = self.bytes.strip_token(SyntaxToken::End) {
            (RuleAnchor::End, rest)
        } else {
            (RuleAnchor::Anywhere, self.bytes)
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
        let payload = Payload::parse(
            self.bytes.as_slice(),
            self.line_number,
            PayloadKind::LeftSideData,
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
        self.into_payload_syntax().parse(payload_limit)
    }

    fn into_payload_syntax(self) -> RightPayloadSyntax<'code> {
        if let Some(rest) = self.bytes.strip_token(SyntaxToken::Start) {
            RightPayloadSyntax {
                line_number: self.line_number,
                bytes: rest,
                action: RightActionSyntax::MoveStart,
            }
        } else if let Some(rest) = self.bytes.strip_token(SyntaxToken::End) {
            RightPayloadSyntax {
                line_number: self.line_number,
                bytes: rest,
                action: RightActionSyntax::MoveEnd,
            }
        } else if let Some(rest) = self.bytes.strip_token(SyntaxToken::Return) {
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
    bytes: CompactSyntax<'code>,
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
        let payload = Payload::parse(
            self.bytes.as_slice(),
            self.line_number,
            self.action.payload_kind(),
        )?;
        Ok(self.action.into_body(payload))
    }
}

/// Checks one parsed payload length against parser limits.
///
/// # Errors
///
/// Returns `ParseError` if the payload length exceeds `limit`.
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
