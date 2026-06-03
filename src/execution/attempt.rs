use crate::inspect::{OnceReturnRuleView, OnceRewriteRuleView, RuleView};

/// Completed non-applying rule attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleMiss<'program> {
    /// The attempted rule was available but did not match the current runtime state.
    StateMismatch(StateMismatchRuleMiss<'program>),
    /// The attempted `(once)` rule had already committed in this run.
    OnceConsumed(OnceConsumedRuleMiss<'program>),
}

/// Available rule that did not match the current runtime state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateMismatchRuleMiss<'program> {
    /// Rule selected for the non-applying attempt.
    rule: RuleView<'program>,
}

/// Consumed `(once)` rule selected for a non-applying attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnceConsumedRuleMiss<'program> {
    /// Consumed once-only rewrite rule.
    Rewrite(OnceRewriteRuleView<'program>),
    /// Consumed once-only return rule.
    Return(OnceReturnRuleView<'program>),
}

impl<'program> RuleMiss<'program> {
    /// Captures an available rule that failed runtime-state matching.
    pub(crate) const fn state_mismatch(rule: RuleView<'program>) -> Self {
        Self::StateMismatch(StateMismatchRuleMiss { rule })
    }

    /// Captures a consumed once-only rewrite rule.
    pub(crate) const fn once_rewrite_consumed(rule: OnceRewriteRuleView<'program>) -> Self {
        Self::OnceConsumed(OnceConsumedRuleMiss::Rewrite(rule))
    }

    /// Captures a consumed once-only return rule.
    pub(crate) const fn once_return_consumed(rule: OnceReturnRuleView<'program>) -> Self {
        Self::OnceConsumed(OnceConsumedRuleMiss::Return(rule))
    }

    /// Rule witness for the consumed rule line.
    #[must_use]
    pub const fn rule(self) -> RuleView<'program> {
        match self {
            Self::StateMismatch(miss) => miss.rule(),
            Self::OnceConsumed(miss) => miss.rule(),
        }
    }
}

impl<'program> StateMismatchRuleMiss<'program> {
    /// Rule selected for the non-applying attempt.
    #[must_use]
    pub const fn rule(self) -> RuleView<'program> {
        self.rule
    }
}

impl<'program> OnceConsumedRuleMiss<'program> {
    /// Rule selected for the non-applying attempt.
    #[must_use]
    pub const fn rule(self) -> RuleView<'program> {
        match self {
            Self::Rewrite(rule) => RuleView::OnceRewrite(rule),
            Self::Return(rule) => RuleView::OnceReturn(rule),
        }
    }
}
