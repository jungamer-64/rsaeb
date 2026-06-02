use alloc::vec::Vec;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::bytes::Payload;
use crate::inspect::{AlwaysRepeat, OnceRepeat};
use crate::syntax::SyntaxToken;

use super::model::{CanonicalRightSide, RepeatRule, RuleAnchorSyntax};

/// Materializes a reusable rule's canonical source form.
///
/// # Errors
///
/// Returns `AllocationError` if canonical source length arithmetic
/// overflows or the output buffer cannot be allocated.
pub(crate) fn canonical_always_source(
    rule: &RepeatRule<AlwaysRepeat>,
) -> Result<Vec<u8>, AllocationError> {
    canonical_repeat_source(rule, RepeatPrefix::Always)
}

/// Materializes a once-only rule's canonical source form.
///
/// # Errors
///
/// Returns `AllocationError` if canonical source length arithmetic
/// overflows or the output buffer cannot be allocated.
pub(crate) fn canonical_once_source(
    rule: &RepeatRule<OnceRepeat>,
) -> Result<Vec<u8>, AllocationError> {
    canonical_repeat_source(rule, RepeatPrefix::Once)
}

/// Canonical repeat marker emitted before the left-side pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RepeatPrefix {
    /// No repeat marker.
    Always,
    /// Emit the `(once)` marker.
    Once,
}

/// Materializes a typed repeat-axis rule's canonical source form.
///
/// # Errors
///
/// Returns `AllocationError` if canonical source length arithmetic
/// overflows or the output buffer cannot be allocated.
fn canonical_repeat_source<R>(
    rule: &RepeatRule<R>,
    repeat: RepeatPrefix,
) -> Result<Vec<u8>, AllocationError> {
    let mut output = Vec::new();
    try_reserve_total_exact(
        &mut output,
        RequestedCapacity::new(canonical_source_len(rule, repeat)?),
        AllocationContext::CanonicalSource,
    )?;

    if matches!(repeat, RepeatPrefix::Once) {
        push_token(&mut output, SyntaxToken::Once)?;
    }

    match rule.anchor() {
        RuleAnchorSyntax::Anywhere => {}
        RuleAnchorSyntax::Start => push_token(&mut output, SyntaxToken::Start)?,
        RuleAnchorSyntax::End => push_token(&mut output, SyntaxToken::End)?,
    }

    push_payload(&mut output, rule.lhs())?;
    try_push(&mut output, b'=', AllocationContext::CanonicalSource)?;

    match rule.canonical_action() {
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
fn canonical_source_len<R>(
    rule: &RepeatRule<R>,
    repeat: RepeatPrefix,
) -> Result<usize, AllocationError> {
    let mut len = rule.lhs().byte_count().get();

    len = checked_source_len_add(len, repeat_token_len(repeat))?;
    len = checked_source_len_add(len, anchor_token_len(rule.anchor()))?;
    len = checked_source_len_add(len, 1)?;
    len = checked_source_len_add(len, action_source_len(rule.canonical_action())?)?;

    Ok(len)
}

/// Returns the canonical repeat marker length for a rule availability.
fn repeat_token_len(repeat: RepeatPrefix) -> usize {
    match repeat {
        RepeatPrefix::Always => 0,
        RepeatPrefix::Once => SyntaxToken::Once.len(),
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
fn action_source_len(action: CanonicalRightSide<'_>) -> Result<usize, AllocationError> {
    let payload_len = action_payload(action).byte_count().get();

    match action_prefix_token(action) {
        Some(token) => checked_source_len_add(token.len(), payload_len),
        None => Ok(payload_len),
    }
}

/// Returns the canonical syntax token that prefixes a right-side payload.
fn action_prefix_token(action: CanonicalRightSide<'_>) -> Option<SyntaxToken> {
    match action {
        CanonicalRightSide::Replace(_) => None,
        CanonicalRightSide::MoveStart(_) => Some(SyntaxToken::Start),
        CanonicalRightSide::MoveEnd(_) => Some(SyntaxToken::End),
        CanonicalRightSide::Return(_) => Some(SyntaxToken::Return),
    }
}

/// Returns the right-side payload independent of its canonical prefix token.
fn action_payload(action: CanonicalRightSide<'_>) -> &Payload {
    match action {
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
