pub use crate::runtime::matcher::RuleMissReason;

/// Completed non-applying rule attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleMiss<RuleWitness> {
    /// Rule witness for the consumed rule line.
    rule: RuleWitness,
    /// Why the consumed rule did not apply.
    reason: RuleMissReason,
}

impl<RuleWitness> RuleMiss<RuleWitness> {
    /// Captures the rule and reason for one consumed non-applying rule line.
    pub(crate) const fn new(rule: RuleWitness, reason: RuleMissReason) -> Self {
        Self { rule, reason }
    }

    /// Rule witness for the consumed rule line.
    #[must_use]
    pub const fn rule(&self) -> &RuleWitness {
        &self.rule
    }

    /// Why the consumed rule did not apply.
    #[must_use]
    pub const fn reason(&self) -> RuleMissReason {
        self.reason
    }

    /// Splits this miss into its rule witness and miss reason.
    #[must_use]
    pub fn into_parts(self) -> (RuleWitness, RuleMissReason) {
        (self.rule, self.reason)
    }
}
