use crate::input::RunSeed;
use crate::limits::RuleAttemptLimit;

/// Run-start witness for rule-attempt execution.
///
/// Rule-attempt execution consumes the same admitted runtime input as ordinary
/// execution, plus a separate rule-attempt budget. Grouping them prevents
/// callers from passing a detached limit beside an unrelated run seed.
#[derive(Debug, PartialEq, Eq)]
pub struct RuleAttemptSeed {
    /// Admitted runtime input and execution limits.
    seed: RunSeed,
    /// Budget for consumed executable rule-line attempts.
    limit: RuleAttemptLimit,
}

impl RuleAttemptSeed {
    /// Binds one admitted run seed to a rule-attempt budget.
    #[must_use]
    pub const fn new(seed: RunSeed, limit: RuleAttemptLimit) -> Self {
        Self { seed, limit }
    }

    /// Splits the seed into the ordinary run seed and the rule-attempt limit.
    pub(crate) fn into_parts(self) -> (RunSeed, RuleAttemptLimit) {
        (self.seed, self.limit)
    }
}
