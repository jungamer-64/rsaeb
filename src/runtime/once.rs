use alloc::vec::Vec;

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_reserve_total_exact,
};
use crate::inspect::OnceRuleCount;
use crate::rule::{OnceRuleSlot, Rule, RuleAvailability};

/// Per-run execution state for parsed `(once)` slots.
#[derive(Debug)]
pub(crate) struct OnceStateSet {
    /// Parser-assigned `(once)` slots already consumed during this run.
    consumed_slots: Vec<OnceRuleSlot>,
}

/// Linear commit action for a matched rule.
#[derive(Debug)]
pub(super) enum MatchedRuleCommit<'once> {
    /// Rule has no once-state side effect.
    Always,
    /// Rule owns the unique permit to consume its once state.
    Once(OnceMatchPermit<'once>),
}

/// Availability of a parsed rule together with the only valid commit path.
#[derive(Debug)]
pub(super) enum OnceRuleReadiness<'once> {
    /// Rule is available and carries the commit action for a later successful application.
    Available(MatchedRuleCommit<'once>),
    /// Rule has already committed during this runtime invocation.
    Consumed,
}

/// Availability of a scanned parsed rule without reserving mutable once state yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScannedRuleReadiness {
    /// Rule is available and carries the data needed to mint its commit action.
    Available(ScannedRuleCommit),
    /// Rule has already committed during this runtime invocation.
    Consumed,
}

/// Commit seed for a scanned rule already proven available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScannedRuleCommit {
    /// Rule has no once-state side effect.
    Always,
    /// Rule owns this fresh parser-assigned `(once)` slot.
    Once(OnceRuleSlot),
}

/// Private permit that consumes one fresh once-rule state on commit.
#[derive(Debug)]
pub(super) struct OnceMatchPermit<'once> {
    /// Set that will own the consumed slot after commit.
    consumed: &'once mut OnceStateSet,
    /// Fresh parser-assigned slot reserved for the matched rule.
    slot: OnceRuleSlot,
    /// Non-copy token that keeps the permit linear even though its witnesses are copyable.
    linearity: OnceMatchPermitLinearity,
}

/// Parsed rule paired with its runtime availability state.
#[derive(Debug)]
pub(crate) struct RuntimeRule<'program, 'once> {
    /// Parsed executable rule.
    rule: &'program Rule,
    /// Runtime availability selected by this rule's parsed shape.
    availability: RuntimeRuleAvailability<'once>,
}

/// Runtime availability paired with one parsed rule.
#[derive(Debug)]
enum RuntimeRuleAvailability<'once> {
    /// Rule has no per-run state.
    Always,
    /// Rule owns this parser-assigned `(once)` slot.
    Once {
        /// Set that records consumed once slots.
        consumed: &'once mut OnceStateSet,
        /// Parser-assigned once slot for this rule.
        slot: OnceRuleSlot,
    },
}

/// Non-copy marker carried by once-rule commit permits.
#[derive(Debug)]
struct OnceMatchPermitLinearity;

impl OnceMatchPermitLinearity {
    /// Creates the linearity marker for one permit.
    const fn new() -> Self {
        Self
    }
}

impl<'once> OnceMatchPermit<'once> {
    /// Creates the commit permit after availability has been checked.
    fn new(consumed: &'once mut OnceStateSet, slot: OnceRuleSlot) -> Self {
        Self {
            consumed,
            slot,
            linearity: OnceMatchPermitLinearity::new(),
        }
    }
}

impl MatchedRuleCommit<'_> {
    /// Applies the rule's once-state side effect after rewrite success.
    pub(super) fn commit(self) {
        match self {
            Self::Always => {}
            Self::Once(commit) => commit.commit(),
        }
    }
}

impl OnceStateSet {
    /// Builds per-execution rule availability state from the parsed rule table.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the per-execution rule-state table cannot
    /// be allocated.
    pub(crate) fn new(once_count: OnceRuleCount) -> Result<Self, AllocationError> {
        let mut consumed_slots = Vec::new();
        try_reserve_total_exact(
            &mut consumed_slots,
            RequestedCapacity::from_once_rule_count(once_count),
            AllocationContext::RuntimeOnceRuleState,
        )?;

        Ok(Self { consumed_slots })
    }

    /// Pairs one parsed rule with its parser-assigned runtime availability.
    pub(super) fn runtime_rule_mut<'program, 'once>(
        &'once mut self,
        rule: &'program Rule,
    ) -> RuntimeRule<'program, 'once> {
        let availability = match rule.availability() {
            RuleAvailability::Always => RuntimeRuleAvailability::Always,
            RuleAvailability::Once(slot) => RuntimeRuleAvailability::Once {
                consumed: self,
                slot,
            },
        };

        RuntimeRule { rule, availability }
    }

    /// Returns the scanned rule's readiness without reserving mutable once state.
    pub(super) fn scanned_rule_readiness(&self, rule: &Rule) -> ScannedRuleReadiness {
        match rule.availability() {
            RuleAvailability::Always => ScannedRuleReadiness::Available(ScannedRuleCommit::Always),
            RuleAvailability::Once(slot) if self.contains(slot) => ScannedRuleReadiness::Consumed,
            RuleAvailability::Once(slot) => {
                ScannedRuleReadiness::Available(ScannedRuleCommit::Once(slot))
            }
        }
    }

    /// Returns whether a parser-assigned `(once)` slot has already committed.
    fn contains(&self, slot: OnceRuleSlot) -> bool {
        self.consumed_slots.contains(&slot)
    }
}

impl<'program, 'once> RuntimeRule<'program, 'once> {
    /// Parsed rule selected with its runtime state.
    pub(super) const fn rule(&self) -> &'program Rule {
        self.rule
    }

    /// Returns this rule's current per-run readiness and commit action.
    pub(super) fn readiness(self) -> OnceRuleReadiness<'once> {
        match self.availability {
            RuntimeRuleAvailability::Always => {
                OnceRuleReadiness::Available(MatchedRuleCommit::Always)
            }
            RuntimeRuleAvailability::Once { consumed, slot } if consumed.contains(slot) => {
                OnceRuleReadiness::Consumed
            }
            RuntimeRuleAvailability::Once { consumed, slot } => OnceRuleReadiness::Available(
                MatchedRuleCommit::Once(OnceMatchPermit::new(consumed, slot)),
            ),
        }
    }
}

impl ScannedRuleCommit {
    /// Mints the linear commit action for this already selected rule.
    pub(super) fn into_matched_commit(self, consumed: &mut OnceStateSet) -> MatchedRuleCommit<'_> {
        match self {
            Self::Always => MatchedRuleCommit::Always,
            Self::Once(slot) => MatchedRuleCommit::Once(OnceMatchPermit::new(consumed, slot)),
        }
    }
}

impl OnceMatchPermit<'_> {
    /// Consumes this permit and marks the owning once-rule state as consumed.
    fn commit(self) {
        let Self {
            consumed,
            slot,
            linearity: _linearity,
        } = self;
        consumed.consumed_slots.push(slot);
    }
}
