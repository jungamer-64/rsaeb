use alloc::vec::Vec;

use crate::allocation::AllocationError;
use crate::bytes::PayloadByteCount;
use crate::inspect::{RuleAction, RuleAnchor, RulePosition, RuleRepeat, RuleView};
use crate::materialized::{MaterializedBytes, OwnedRuleWitnessPayloadDomain};
use crate::source::SourceLineNumber;

/// Parsed payload bytes retained by owned execution rule witnesses.
#[derive(Debug, PartialEq, Eq)]
pub struct OwnedRulePayload {
    /// Owned bytes tagged as an owned execution rule witness payload.
    bytes: MaterializedBytes<OwnedRuleWitnessPayloadDomain>,
}

impl OwnedRulePayload {
    /// Borrow the materialized payload bytes.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    /// Consumes this value and returns the materialized host bytes.
    #[must_use]
    pub fn into_raw_bytes(self) -> Vec<u8> {
        self.bytes.into_raw_bytes()
    }

    /// Materialized payload length in bytes.
    #[must_use]
    pub fn byte_count(&self) -> PayloadByteCount {
        PayloadByteCount::new(self.bytes.len())
    }

    /// Returns whether this materialized payload contains no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// Owned parsed-rule witness retained by owned execution transitions.
#[derive(Debug, PartialEq, Eq)]
pub struct OwnedRuleWitness {
    /// Program-local parsed-rule position.
    position: RulePosition,
    /// One-based source line number.
    line_number: SourceLineNumber,
    /// Rule repeat policy.
    repeat: RuleRepeat,
    /// Rule match anchor.
    anchor: RuleAnchor,
    /// Materialized left-side match payload.
    lhs: OwnedRulePayload,
    /// Materialized right-side action payload.
    action: RuleAction<OwnedRulePayload>,
}

impl OwnedRuleWitness {
    /// Materializes an owned witness from the borrowed parsed rule boundary.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if retaining the parsed rule payload bytes for
    /// an owned execution transition cannot allocate.
    pub(crate) fn from_rule_view(rule: RuleView<'_>) -> Result<Self, AllocationError> {
        let lhs = materialize_owned_rule_payload(rule.lhs())?;
        let action = match rule.action() {
            RuleAction::Replace(payload) => {
                RuleAction::Replace(materialize_owned_rule_payload(payload)?)
            }
            RuleAction::MoveStart(payload) => {
                RuleAction::MoveStart(materialize_owned_rule_payload(payload)?)
            }
            RuleAction::MoveEnd(payload) => {
                RuleAction::MoveEnd(materialize_owned_rule_payload(payload)?)
            }
            RuleAction::Return(payload) => {
                RuleAction::Return(materialize_owned_rule_payload(payload)?)
            }
        };

        Ok(Self {
            position: rule.position(),
            line_number: rule.line_number(),
            repeat: rule.repeat(),
            anchor: rule.anchor(),
            lhs,
            action,
        })
    }

    /// Program-local parsed-rule position.
    #[must_use]
    pub const fn position(&self) -> RulePosition {
        self.position
    }

    /// One-based source line number.
    #[must_use]
    pub const fn line_number(&self) -> SourceLineNumber {
        self.line_number
    }

    /// Rule repeat policy.
    #[must_use]
    pub const fn repeat(&self) -> RuleRepeat {
        self.repeat
    }

    /// Rule match anchor.
    #[must_use]
    pub const fn anchor(&self) -> RuleAnchor {
        self.anchor
    }

    /// Materialized left-side match payload.
    #[must_use]
    pub const fn lhs(&self) -> &OwnedRulePayload {
        &self.lhs
    }

    /// Materialized right-side action payload.
    #[must_use]
    pub const fn action(&self) -> &RuleAction<OwnedRulePayload> {
        &self.action
    }
}

/// Materializes a payload for the owned execution rule-witness boundary.
///
/// # Errors
///
/// Returns `AllocationError` if the payload bytes cannot be retained for an
/// owned execution rule witness.
fn materialize_owned_rule_payload(
    payload: crate::inspect::PayloadView<'_>,
) -> Result<OwnedRulePayload, AllocationError> {
    Ok(OwnedRulePayload {
        bytes: MaterializedBytes::from_owned_rule_payload(payload)?,
    })
}
