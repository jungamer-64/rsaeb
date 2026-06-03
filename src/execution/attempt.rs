/// Completed non-applying rule attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleMiss<RuleWitness> {
    /// Rule witness for the consumed rule line.
    rule: RuleWitness,
}

impl<RuleWitness> RuleMiss<RuleWitness> {
    /// Captures the rule for one consumed non-applying rule line.
    pub(crate) const fn new(rule: RuleWitness) -> Self {
        Self { rule }
    }

    /// Rule witness for the consumed rule line.
    #[must_use]
    pub const fn rule(&self) -> &RuleWitness {
        &self.rule
    }
}
