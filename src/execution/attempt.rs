use crate::inspect::{
    AlwaysReturnRuleView, AlwaysRewriteRuleView, OnceReturnRuleView, OnceRewriteRuleView,
};

/// Completed non-applying rule attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleMiss<'program> {
    /// A reusable rewrite rule was available but did not match the current runtime state.
    AlwaysRewriteStateMismatch(AlwaysRewriteRuleView<'program>),
    /// A once-only rewrite rule was available but did not match the current runtime state.
    OnceRewriteStateMismatch(OnceRewriteRuleView<'program>),
    /// A reusable return rule was available but did not match the current runtime state.
    AlwaysReturnStateMismatch(AlwaysReturnRuleView<'program>),
    /// A once-only return rule was available but did not match the current runtime state.
    OnceReturnStateMismatch(OnceReturnRuleView<'program>),
    /// A once-only rewrite rule had already committed in this run.
    OnceRewriteConsumed(OnceRewriteRuleView<'program>),
    /// A once-only return rule had already committed in this run.
    OnceReturnConsumed(OnceReturnRuleView<'program>),
}

impl<'program> RuleMiss<'program> {
    /// Captures an available reusable rewrite rule that failed runtime-state matching.
    pub(crate) const fn always_rewrite_state_mismatch(
        rule: AlwaysRewriteRuleView<'program>,
    ) -> Self {
        Self::AlwaysRewriteStateMismatch(rule)
    }

    /// Captures an available once-only rewrite rule that failed runtime-state matching.
    pub(crate) const fn once_rewrite_state_mismatch(rule: OnceRewriteRuleView<'program>) -> Self {
        Self::OnceRewriteStateMismatch(rule)
    }

    /// Captures an available reusable return rule that failed runtime-state matching.
    pub(crate) const fn always_return_state_mismatch(rule: AlwaysReturnRuleView<'program>) -> Self {
        Self::AlwaysReturnStateMismatch(rule)
    }

    /// Captures an available once-only return rule that failed runtime-state matching.
    pub(crate) const fn once_return_state_mismatch(rule: OnceReturnRuleView<'program>) -> Self {
        Self::OnceReturnStateMismatch(rule)
    }

    /// Captures a consumed once-only rewrite rule.
    pub(crate) const fn once_rewrite_consumed(rule: OnceRewriteRuleView<'program>) -> Self {
        Self::OnceRewriteConsumed(rule)
    }

    /// Captures a consumed once-only return rule.
    pub(crate) const fn once_return_consumed(rule: OnceReturnRuleView<'program>) -> Self {
        Self::OnceReturnConsumed(rule)
    }
}
