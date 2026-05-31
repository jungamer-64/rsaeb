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

    /// Returns whether a parsed rule is unavailable because its `(once)` slot
    /// has already committed.
    pub(super) fn is_rule_consumed(&self, rule: &Rule) -> bool {
        match rule.availability() {
            RuleAvailability::Always => false,
            RuleAvailability::Once(slot) => self.contains(slot),
        }
    }

    /// Creates the linear commit action for a rule already known to be fresh.
    pub(super) fn commit_for_fresh_rule(&mut self, rule: &Rule) -> MatchedRuleCommit<'_> {
        match rule.availability() {
            RuleAvailability::Always => MatchedRuleCommit::Always,
            RuleAvailability::Once(slot) => {
                debug_assert!(!self.contains(slot));
                MatchedRuleCommit::Once(OnceMatchPermit::new(self, slot))
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
