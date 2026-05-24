use alloc::vec::Vec;

use crate::allocation::{AllocationContext, try_push};
use crate::bytes::{CompactByte, Payload, PayloadByteCount, PayloadSyntax};
use crate::error::{
    LeftModifierKind, ParseError, ParseErrorKind, ParseLimitError, PayloadKind, RightActionKind,
};
use crate::limits::PayloadByteLimit;
use crate::rule::{
    ParsedRule, RewriteAction, RuleAction, RuleAnchorSyntax, RuleBody, RuleHead, RuleRepeatSyntax,
};
use crate::source::{SourceColumn, SourceLineNumber, SourcePosition};
use crate::syntax::SyntaxToken;

/// Compact executable line split into left and right rule syntax.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct RuleSyntaxLine {
    /// Original source line for diagnostics.
    line_number: SourceLineNumber,
    /// Left-side syntax before `=`.
    left: OwnedCompactSyntax,
    /// Right-side syntax after `=`.
    right: OwnedCompactSyntax,
}

impl RuleSyntaxLine {
    /// Splits one compact source line around its rule separator.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the line has no `=`, multiple `=` bytes, or a
    /// rule-side buffer cannot be allocated.
    pub(super) fn new(
        line_number: SourceLineNumber,
        bytes: Vec<CompactByte>,
    ) -> Result<Self, ParseError> {
        let parts = RuleSyntaxParts::split(line_number, bytes)?;
        Ok(Self {
            line_number,
            left: parts.left,
            right: parts.right,
        })
    }

    /// Parses this compact rule syntax into typed rule data.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if either rule side contains invalid modifier,
    /// action, or payload syntax.
    pub(super) fn parse(&self, payload_limit: PayloadByteLimit) -> Result<ParsedRule, ParseError> {
        let (left, right) = self.syntax_parts();
        let head = left.parse(payload_limit)?;
        let body = right.parse(payload_limit)?;

        Ok(ParsedRule::from_parts(self.line_number, head, body))
    }

    /// Borrows the compact line as left and right syntax slices.
    fn syntax_parts(&self) -> (LeftSyntax<'_>, RightSyntax<'_>) {
        (
            LeftSyntax {
                line_number: self.line_number,
                bytes: self.left.as_syntax(),
            },
            RightSyntax {
                line_number: self.line_number,
                bytes: self.right.as_syntax(),
            },
        )
    }
}

/// Owned syntax split around the single rule separator.
#[derive(Debug, PartialEq, Eq)]
struct RuleSyntaxParts {
    /// Left-side syntax before `=`.
    left: OwnedCompactSyntax,
    /// Right-side syntax after `=`.
    right: OwnedCompactSyntax,
}

/// Owned compact syntax bytes for one rule side.
#[derive(Debug, PartialEq, Eq)]
struct OwnedCompactSyntax {
    /// Compact bytes in the current syntax domain.
    bytes: Vec<CompactByte>,
}

/// Borrowed compact syntax bytes for one parser phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CompactSyntax<'code> {
    /// Compact bytes in the current syntax domain.
    bytes: &'code [CompactByte],
}

/// Result of matching a concrete syntax token prefix.
#[derive(Clone, Copy)]
struct MatchedSyntaxPrefix<'code> {
    /// Source column of the matched token.
    column: SourceColumn,
    /// Remaining bytes after the matched token.
    rest: CompactSyntax<'code>,
}

/// Matched token classified into a parser-domain kind.
#[derive(Clone, Copy)]
struct MatchedToken<T> {
    /// Parser-domain token kind.
    kind: T,
    /// Source column where the token starts.
    column: SourceColumn,
}

impl RuleSyntaxParts {
    /// Splits compact syntax into owned left and right domains.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the line has no `=`, multiple `=` bytes, or a
    /// side buffer cannot be allocated.
    fn split(line_number: SourceLineNumber, bytes: Vec<CompactByte>) -> Result<Self, ParseError> {
        let mut left = OwnedCompactSyntax::new();
        let mut right = OwnedCompactSyntax::new();
        let mut seen_separator = false;

        for byte in bytes {
            if byte.as_u8() == b'=' {
                if seen_separator {
                    return Err(ParseError::at_position(
                        SourcePosition::new(line_number, byte.source_column()),
                        ParseErrorKind::MultipleEquals,
                    ));
                }
                seen_separator = true;
                continue;
            }

            if seen_separator {
                right.push(byte, line_number)?;
            } else {
                left.push(byte, line_number)?;
            }
        }

        if seen_separator {
            Ok(Self { left, right })
        } else {
            Err(ParseError::at_line(
                line_number,
                ParseErrorKind::MissingEquals,
            ))
        }
    }
}

impl OwnedCompactSyntax {
    /// Starts an empty owned syntax side.
    const fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    /// Adds one compact byte to this side.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if storing the side byte fails.
    fn push(&mut self, byte: CompactByte, line_number: SourceLineNumber) -> Result<(), ParseError> {
        try_push(&mut self.bytes, byte, AllocationContext::ProgramCodeLine)
            .map_err(|error| ParseError::at_line(line_number, ParseErrorKind::Allocation(error)))
    }

    /// Borrows this owned syntax side.
    fn as_syntax(&self) -> CompactSyntax<'_> {
        CompactSyntax { bytes: &self.bytes }
    }
}

impl<'code> CompactSyntax<'code> {
    /// Returns the compact bytes in this syntax domain.
    const fn as_slice(self) -> &'code [CompactByte] {
        self.bytes
    }

    /// Removes a leading syntax token when it is present.
    fn strip_token(self, token: SyntaxToken) -> Option<MatchedSyntaxPrefix<'code>> {
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
            let column = self.bytes.first().copied()?.source_column();
            self.bytes
                .get(token_bytes.len()..)
                .map(|bytes| MatchedSyntaxPrefix {
                    column,
                    rest: Self { bytes },
                })
        } else {
            None
        }
    }
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
        if let Some(matched) = self.bytes.strip_token(SyntaxToken::Once) {
            LeftAfterRepeat {
                line_number: self.line_number,
                bytes: matched.rest,
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
        let (anchor, bytes) = if let Some(matched) = self.bytes.strip_token(SyntaxToken::Start) {
            (RuleAnchorSyntax::Start, matched.rest)
        } else if let Some(matched) = self.bytes.strip_token(SyntaxToken::End) {
            (RuleAnchorSyntax::End, matched.rest)
        } else {
            (RuleAnchorSyntax::Anywhere, self.bytes)
        };

        if let Some(modifier) = left_modifier_kind(bytes) {
            return Err(ParseError::at_position(
                SourcePosition::new(self.line_number, modifier.column),
                ParseErrorKind::UnsupportedLeftModifierOrder {
                    modifier: modifier.kind,
                },
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
        if let Some(matched) = self.bytes.strip_token(SyntaxToken::Start) {
            return Ok(RightPayloadSyntax::Action(RightActionPayloadSyntax::new(
                self.line_number,
                matched.rest,
                RightActionSyntax::MoveStart,
            )?));
        } else if let Some(matched) = self.bytes.strip_token(SyntaxToken::End) {
            return Ok(RightPayloadSyntax::Action(RightActionPayloadSyntax::new(
                self.line_number,
                matched.rest,
                RightActionSyntax::MoveEnd,
            )?));
        } else if let Some(matched) = self.bytes.strip_token(SyntaxToken::Return) {
            return Ok(RightPayloadSyntax::Action(RightActionPayloadSyntax::new(
                self.line_number,
                matched.rest,
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
    Ok(syntax.validate()?.into_payload())
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
) -> Option<MatchedToken<T>> {
    mappings.iter().find_map(|&(token, kind)| {
        input.strip_token(token).map(|matched| MatchedToken {
            kind,
            column: matched.column,
        })
    })
}

/// Classifies a left-side modifier token at the current syntax boundary.
fn left_modifier_kind(input: CompactSyntax<'_>) -> Option<MatchedToken<LeftModifierKind>> {
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
fn right_action_kind(input: CompactSyntax<'_>) -> Option<MatchedToken<RightActionKind>> {
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
        return Err(ParseError::at_position(
            SourcePosition::new(line_number, action.column),
            ParseErrorKind::UnsupportedRightActionSyntax {
                action: action.kind,
            },
        ));
    }

    Ok(())
}
