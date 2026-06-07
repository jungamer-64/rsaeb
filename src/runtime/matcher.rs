use super::once::OnceRewriteCommitPermit;
use super::state::{State, StateMatch, StatePayloadMatch};
use crate::bytes::Payload;
use crate::inspect::{
    AlwaysReturnRuleView, AlwaysRewriteRuleView, OnceReturnRuleView, OnceRewriteRuleView,
};
use crate::rule::{RewriteAction, RulePattern};

/// Outcome of evaluating one executable rule line against the current state.
#[derive(Debug)]
pub(crate) enum RuleAttemptEvaluation<'program, 'state, 'once> {
    /// The rule matched and carries the commit permit needed after success.
    Matched(MatchedRuleApplication<'program, 'state, 'once>),
    /// The rule did not apply and carries the exact miss shape.
    Miss(EvaluatedRuleMiss<'program>),
}

/// Exact non-applying result of evaluating one executable rule line.
#[derive(Debug)]
pub(crate) enum EvaluatedRuleMiss<'program> {
    /// Available reusable rewrite rule did not match the current runtime state.
    AlwaysRewriteStateMismatch(AlwaysRewriteRuleView<'program>),
    /// Available once-only rewrite rule did not match the current runtime state.
    OnceRewriteStateMismatch(OnceRewriteRuleView<'program>),
    /// Available reusable return rule did not match the current runtime state.
    AlwaysReturnStateMismatch(AlwaysReturnRuleView<'program>),
    /// Available once-only return rule did not match the current runtime state.
    OnceReturnStateMismatch(OnceReturnRuleView<'program>),
    /// Once-only rewrite rule had already committed in this run.
    OnceRewriteConsumed(OnceRewriteRuleView<'program>),
}

/// Matched rule plus the state range and action-specific commit data.
#[derive(Debug)]
pub(crate) enum MatchedRuleApplication<'program, 'state, 'once> {
    /// Matched reusable non-terminal rewrite rule.
    AlwaysRewrite(MatchedAlwaysRewriteApplication<'program, 'state>),
    /// Matched once-only non-terminal rewrite rule.
    OnceRewrite(MatchedOnceRewriteApplication<'program, 'state, 'once>),
    /// Matched reusable terminal return rule.
    AlwaysReturn(MatchedAlwaysReturnApplication<'program, 'state>),
    /// Matched once-only terminal return rule.
    OnceReturn(MatchedOnceReturnApplication<'program, 'state>),
}

/// Matched reusable non-terminal rewrite rule.
#[derive(Debug)]
pub(crate) struct MatchedAlwaysRewriteApplication<'program, 'state> {
    /// Parsed rule selected by the matcher.
    rule: AlwaysRewriteRuleView<'program>,
    /// Runtime-state range matched by the rule left side.
    state_match: StateMatch<'state>,
}

/// Matched once-only non-terminal rewrite rule.
#[derive(Debug)]
pub(crate) struct MatchedOnceRewriteApplication<'program, 'state, 'once> {
    /// Parsed rule selected by the matcher.
    rule: OnceRewriteRuleView<'program>,
    /// Runtime-state range matched by the rule left side.
    state_match: StateMatch<'state>,
    /// Once-state side effect to apply only after successful rewrite.
    commit: OnceRewriteCommitPermit<'program, 'once>,
}

/// Matched reusable terminal return rule.
#[derive(Debug)]
pub(crate) struct MatchedAlwaysReturnApplication<'program, 'state> {
    /// Parsed rule selected by the matcher.
    rule: AlwaysReturnRuleView<'program>,
    /// Runtime-state range matched by the rule left side.
    state_match: StateMatch<'state>,
}

/// Matched once-only terminal return rule.
#[derive(Debug)]
pub(crate) struct MatchedOnceReturnApplication<'program, 'state> {
    /// Parsed rule selected by the matcher.
    rule: OnceReturnRuleView<'program>,
    /// Runtime-state range matched by the rule left side.
    state_match: StateMatch<'state>,
}

impl<'program, 'state> MatchedAlwaysRewriteApplication<'program, 'state> {
    /// Splits matched reusable rewrite data into preparation parts.
    pub(crate) fn into_parts(
        self,
    ) -> (
        AlwaysRewriteRuleView<'program>,
        StateMatch<'state>,
        &'program RewriteAction,
    ) {
        let action = self.rule.into_rule().rewrite_action();
        (self.rule, self.state_match, action)
    }
}

impl<'program, 'state, 'once> MatchedOnceRewriteApplication<'program, 'state, 'once> {
    /// Splits matched once-only rewrite data into preparation parts.
    pub(crate) fn into_parts(
        self,
    ) -> (
        OnceRewriteRuleView<'program>,
        StateMatch<'state>,
        &'program RewriteAction,
        OnceRewriteCommitPermit<'program, 'once>,
    ) {
        let action = self.rule.into_rule().rewrite_action();
        (self.rule, self.state_match, action, self.commit)
    }
}

impl<'program, 'state> MatchedAlwaysReturnApplication<'program, 'state> {
    /// Splits matched reusable return data into preparation parts.
    pub(crate) fn into_parts(
        self,
    ) -> (
        AlwaysReturnRuleView<'program>,
        StateMatch<'state>,
        &'program Payload,
    ) {
        let output = self.rule.into_rule().output();
        (self.rule, self.state_match, output)
    }
}

impl<'program, 'state> MatchedOnceReturnApplication<'program, 'state> {
    /// Splits matched once-only return data into preparation parts.
    pub(crate) fn into_parts(
        self,
    ) -> (
        OnceReturnRuleView<'program>,
        StateMatch<'state>,
        &'program Payload,
    ) {
        let output = self.rule.into_rule().output();
        (self.rule, self.state_match, output)
    }
}

/// Declares one exact rule-attempt evaluator whose success needs no once-state
/// commit permit.
macro_rules! impl_rule_attempt_without_once_state_commit {
    (
        $(#[$meta:meta])*
        $function:ident,
        $rule:ident,
        $miss:ident,
        $matched_variant:ident,
        $matched:ident
    ) => {
        $(#[$meta])*
        pub(crate) fn $function<'program, 'state, 'once>(
            rule: $rule<'program>,
            state: &'state State,
        ) -> RuleAttemptEvaluation<'program, 'state, 'once> {
            let state_match = match match_rule_state(rule.into_rule().pattern(), state) {
                StatePayloadMatch::Matched(state_match) => state_match,
                StatePayloadMatch::Mismatched => {
                    return RuleAttemptEvaluation::Miss(EvaluatedRuleMiss::$miss(rule));
                }
            };

            RuleAttemptEvaluation::Matched(MatchedRuleApplication::$matched_variant($matched {
                rule,
                state_match,
            }))
        }
    };
}

impl_rule_attempt_without_once_state_commit!(
    /// Evaluates a reusable rewrite rule against the current runtime state.
    attempt_always_rewrite_rule,
    AlwaysRewriteRuleView,
    AlwaysRewriteStateMismatch,
    AlwaysRewrite,
    MatchedAlwaysRewriteApplication
);

/// Evaluates a fresh once-only rewrite rule against the current runtime state.
pub(crate) fn attempt_once_rewrite_rule<'program, 'state, 'once>(
    rule: OnceRewriteRuleView<'program>,
    commit: OnceRewriteCommitPermit<'program, 'once>,
    state: &'state State,
) -> RuleAttemptEvaluation<'program, 'state, 'once> {
    let state_match = match match_rule_state(rule.into_rule().pattern(), state) {
        StatePayloadMatch::Matched(state_match) => state_match,
        StatePayloadMatch::Mismatched => {
            return RuleAttemptEvaluation::Miss(EvaluatedRuleMiss::OnceRewriteStateMismatch(rule));
        }
    };

    RuleAttemptEvaluation::Matched(MatchedRuleApplication::OnceRewrite(
        MatchedOnceRewriteApplication {
            rule,
            state_match,
            commit,
        },
    ))
}

impl_rule_attempt_without_once_state_commit!(
    /// Evaluates a reusable return rule against the current runtime state.
    attempt_always_return_rule,
    AlwaysReturnRuleView,
    AlwaysReturnStateMismatch,
    AlwaysReturn,
    MatchedAlwaysReturnApplication
);

impl_rule_attempt_without_once_state_commit!(
    /// Evaluates a fresh once-only terminal return rule against the current runtime state.
    attempt_once_return_rule,
    OnceReturnRuleView,
    OnceReturnStateMismatch,
    OnceReturn,
    MatchedOnceReturnApplication
);

/// Compares a single parsed rule pattern with the current runtime state.
pub(crate) fn match_rule_state<'state>(
    pattern: &RulePattern,
    state: &'state State,
) -> StatePayloadMatch<'state> {
    state.match_payload(pattern.anchor(), pattern.lhs())
}
