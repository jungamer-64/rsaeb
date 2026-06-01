use alloc::vec::Vec;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::bytes::Payload;
use crate::syntax::SyntaxToken;

use super::model::{CanonicalRightSide, Rule, RuleAnchorSyntax, RuleAvailability};

/// Materializes this rule's canonical source form.
///
/// # Errors
///
/// Returns `AllocationError` if canonical source length arithmetic
/// overflows or the output buffer cannot be allocated.
pub(crate) fn canonical_source(rule: &Rule) -> Result<Vec<u8>, AllocationError> {
    let mut output = Vec::new();
    try_reserve_total_exact(
        &mut output,
        RequestedCapacity::new(canonical_source_len(rule)?),
        AllocationContext::CanonicalSource,
    )?;

    if matches!(rule.availability(), RuleAvailability::Once(_)) {
        push_token(&mut output, SyntaxToken::Once)?;
    }

    match rule.anchor() {
        RuleAnchorSyntax::Anywhere => {}
        RuleAnchorSyntax::Start => push_token(&mut output, SyntaxToken::Start)?,
        RuleAnchorSyntax::End => push_token(&mut output, SyntaxToken::End)?,
    }

    push_payload(&mut output, rule.lhs())?;
    try_push(&mut output, b'=', AllocationContext::CanonicalSource)?;

    match rule.action().canonical_right_side() {
        CanonicalRightSide::Replace(payload) => {
            push_payload(&mut output, payload)?;
        }
        CanonicalRightSide::MoveStart(payload) => {
            push_token(&mut output, SyntaxToken::Start)?;
            push_payload(&mut output, payload)?;
        }
        CanonicalRightSide::MoveEnd(payload) => {
            push_token(&mut output, SyntaxToken::End)?;
            push_payload(&mut output, payload)?;
        }
        CanonicalRightSide::Return(payload) => {
            push_token(&mut output, SyntaxToken::Return)?;
            push_payload(&mut output, payload)?;
        }
    }

    Ok(output)
}

/// Computes the byte length of this rule's canonical source form.
///
/// # Errors
///
/// Returns `AllocationError` if canonical source length arithmetic overflows.
fn canonical_source_len(rule: &Rule) -> Result<usize, AllocationError> {
    let mut len = rule.lhs().byte_count().get();

    len = checked_source_len_add(len, repeat_token_len(rule.availability()))?;
    len = checked_source_len_add(len, anchor_token_len(rule.anchor()))?;
    len = checked_source_len_add(len, 1)?;
    len = checked_source_len_add(len, right_side_len(rule.action().canonical_right_side())?)?;

    Ok(len)
}

/// Returns the canonical `(once)` marker length for a rule availability.
fn repeat_token_len(availability: RuleAvailability) -> usize {
    match availability {
        RuleAvailability::Always => 0,
        RuleAvailability::Once(_) => SyntaxToken::Once.len(),
    }
}

/// Returns the canonical anchor marker length.
fn anchor_token_len(anchor: RuleAnchorSyntax) -> usize {
    match anchor {
        RuleAnchorSyntax::Anywhere => 0,
        RuleAnchorSyntax::Start => SyntaxToken::Start.len(),
        RuleAnchorSyntax::End => SyntaxToken::End.len(),
    }
}

/// Computes the canonical right-side byte length.
///
/// # Errors
///
/// Returns `AllocationError` if token-plus-payload length arithmetic overflows.
fn right_side_len(right_side: CanonicalRightSide<'_>) -> Result<usize, AllocationError> {
    let payload_len = right_side_payload(right_side).byte_count().get();

    match right_side_token(right_side) {
        Some(token) => checked_source_len_add(token.len(), payload_len),
        None => Ok(payload_len),
    }
}

/// Returns the canonical syntax token that prefixes a right-side payload.
fn right_side_token(right_side: CanonicalRightSide<'_>) -> Option<SyntaxToken> {
    match right_side {
        CanonicalRightSide::Replace(_) => None,
        CanonicalRightSide::MoveStart(_) => Some(SyntaxToken::Start),
        CanonicalRightSide::MoveEnd(_) => Some(SyntaxToken::End),
        CanonicalRightSide::Return(_) => Some(SyntaxToken::Return),
    }
}

/// Returns the right-side payload independent of its canonical prefix token.
fn right_side_payload(right_side: CanonicalRightSide<'_>) -> &Payload {
    match right_side {
        CanonicalRightSide::Replace(payload)
        | CanonicalRightSide::MoveStart(payload)
        | CanonicalRightSide::MoveEnd(payload)
        | CanonicalRightSide::Return(payload) => payload,
    }
}

/// Adds one canonical-source length segment.
///
/// # Errors
///
/// Returns `AllocationError` if the combined length cannot be represented.
fn checked_source_len_add(len: usize, segment_len: usize) -> Result<usize, AllocationError> {
    len.checked_add(segment_len)
        .ok_or_else(|| AllocationError::capacity_overflow(AllocationContext::CanonicalSource))
}

/// Appends one syntax token to canonical source output.
///
/// # Errors
///
/// Returns `AllocationError` if the output buffer cannot grow.
fn push_token(output: &mut Vec<u8>, token: SyntaxToken) -> Result<(), AllocationError> {
    for byte in token.bytes().iter().copied() {
        try_push(output, byte, AllocationContext::CanonicalSource)?;
    }

    Ok(())
}

/// Appends payload bytes to canonical source output.
///
/// # Errors
///
/// Returns `AllocationError` if the output buffer cannot grow.
fn push_payload(output: &mut Vec<u8>, payload: &Payload) -> Result<(), AllocationError> {
    payload.push_bytes_to(output, AllocationContext::CanonicalSource)
}
