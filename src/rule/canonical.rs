use alloc::vec::Vec;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::bytes::Payload;
use crate::syntax::SyntaxToken;

use super::model::{CanonicalRightSide, Rule, RuleAnchorSyntax, RuleRepeatState};

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

    if matches!(rule.repeat_state(), RuleRepeatState::Once(_)) {
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

    if matches!(rule.repeat_state(), RuleRepeatState::Once(_)) {
        len = len.checked_add(SyntaxToken::Once.len()).ok_or_else(|| {
            AllocationError::capacity_overflow(AllocationContext::CanonicalSource)
        })?;
    }

    let anchor_len = match rule.anchor() {
        RuleAnchorSyntax::Anywhere => 0,
        RuleAnchorSyntax::Start => SyntaxToken::Start.len(),
        RuleAnchorSyntax::End => SyntaxToken::End.len(),
    };

    let right_side_len = match rule.action().canonical_right_side() {
        CanonicalRightSide::Replace(payload) => payload.byte_count().get(),
        CanonicalRightSide::MoveStart(payload) => SyntaxToken::Start
            .len()
            .checked_add(payload.byte_count().get())
            .ok_or_else(|| {
                AllocationError::capacity_overflow(AllocationContext::CanonicalSource)
            })?,
        CanonicalRightSide::MoveEnd(payload) => SyntaxToken::End
            .len()
            .checked_add(payload.byte_count().get())
            .ok_or_else(|| {
                AllocationError::capacity_overflow(AllocationContext::CanonicalSource)
            })?,
        CanonicalRightSide::Return(payload) => SyntaxToken::Return
            .len()
            .checked_add(payload.byte_count().get())
            .ok_or_else(|| {
                AllocationError::capacity_overflow(AllocationContext::CanonicalSource)
            })?,
    };

    len = len
        .checked_add(anchor_len)
        .and_then(|len| len.checked_add(1))
        .and_then(|len| len.checked_add(right_side_len))
        .ok_or_else(|| AllocationError::capacity_overflow(AllocationContext::CanonicalSource))?;

    Ok(len)
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
