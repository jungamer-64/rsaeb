use super::once::{AvailableRuntimeRule, OnceMatchPermit};
use super::state::{State, StateMatch};
use crate::bytes::Payload;
use crate::inspect::{
    AlwaysReturnRuleView, AlwaysRewriteRuleView, OnceReturnRuleView, OnceRewriteRuleView, RuleView,
};
use crate::rule::{RewriteAction, RuleAnchorSyntax, RulePattern};

/// Outcome of evaluating one executable rule line against the current state.
#[derive(Debug)]
pub(crate) enum RuleAttempt<'program, 'state, 'once> {
    /// The rule matched and carries the commit permit needed after success.
    Matched(MatchedRuleApplication<'program, 'state, 'once>),
    /// The rule was consumed by the attempt but did not apply.
    Missed(RuleAttemptMiss<'program>),
}

/// Outcome of evaluating a rule that is already proven available.
#[derive(Debug)]
pub(crate) enum AvailableRuleAttempt<'program, 'state, 'once> {
    /// The available rule matched and carries the commit permit needed after success.
    Matched(MatchedRuleApplication<'program, 'state, 'once>),
    /// The available rule did not match the current runtime state.
    StateMismatch(RuleAttemptMiss<'program>),
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
    OnceReturn(MatchedOnceReturnApplication<'program, 'state, 'once>),
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
    commit: OnceMatchPermit<'once>,
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
pub(crate) struct MatchedOnceReturnApplication<'program, 'state, 'once> {
    /// Parsed rule selected by the matcher.
    rule: OnceReturnRuleView<'program>,
    /// Runtime-state range matched by the rule left side.
    state_match: StateMatch<'state>,
    /// Once-state side effect to apply only after successful return materialization.
    commit: OnceMatchPermit<'once>,
}

/// Non-applying rule consumed by a rule-attempt step.
#[derive(Debug, Clone, Copy)]
pub(crate) enum RuleAttemptMiss<'program> {
    /// Available rule did not match the current runtime state.
    StateMismatch(RuleView<'program>),
    /// Once-only rewrite rule had already committed in this run.
    OnceRewriteConsumed(OnceRewriteRuleView<'program>),
    /// Once-only return rule had already committed in this run.
    OnceReturnConsumed(OnceReturnRuleView<'program>),
}

/// Domain result of comparing one rule's left side with the runtime state.
enum RuleStateMatch<'state> {
    /// Rule left side matched and carries the matched state span.
    Matched(StateMatch<'state>),
    /// Rule left side did not match the runtime state.
    Mismatched,
}

impl<'program> RuleAttemptMiss<'program> {
    /// Captures an available rule that failed runtime-state matching.
    pub(crate) const fn state_mismatch(rule: RuleView<'program>) -> Self {
        Self::StateMismatch(rule)
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
        OnceMatchPermit<'once>,
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

impl<'program, 'state, 'once> MatchedOnceReturnApplication<'program, 'state, 'once> {
    /// Splits matched once-only return data into preparation parts.
    pub(crate) fn into_parts(
        self,
    ) -> (
        OnceReturnRuleView<'program>,
        StateMatch<'state>,
        &'program Payload,
        OnceMatchPermit<'once>,
    ) {
        let output = self.rule.into_rule().output();
        (self.rule, self.state_match, output, self.commit)
    }
}

/// Evaluates one already-available parsed rule line against the current runtime state.
pub(crate) fn attempt_available_rule<'program, 'state, 'once>(
    runtime_rule: AvailableRuntimeRule<'program, 'once>,
    state: &'state State,
) -> AvailableRuleAttempt<'program, 'state, 'once> {
    match runtime_rule {
        AvailableRuntimeRule::AlwaysRewrite(rule) => {
            attempt_always_rewrite_rule(rule.rule(), state)
        }
        AvailableRuntimeRule::OnceRewrite(rule) => {
            let (rule, commit) = rule.into_parts();
            attempt_once_rewrite_rule(rule, commit, state)
        }
        AvailableRuntimeRule::AlwaysReturn(rule) => attempt_always_return_rule(rule.rule(), state),
        AvailableRuntimeRule::OnceReturn(rule) => {
            let (rule, commit) = rule.into_parts();
            attempt_once_return_rule(rule, commit, state)
        }
    }
}

/// Evaluates an available reusable rewrite rule against the current runtime state.
fn attempt_always_rewrite_rule<'program, 'state, 'once>(
    rule: AlwaysRewriteRuleView<'program>,
    state: &'state State,
) -> AvailableRuleAttempt<'program, 'state, 'once> {
    let state_match = match match_rule_state(rule.into_rule().pattern(), state) {
        RuleStateMatch::Matched(state_match) => state_match,
        RuleStateMatch::Mismatched => {
            return AvailableRuleAttempt::StateMismatch(RuleAttemptMiss::state_mismatch(
                RuleView::AlwaysRewrite(rule),
            ));
        }
    };

    AvailableRuleAttempt::Matched(MatchedRuleApplication::AlwaysRewrite(
        MatchedAlwaysRewriteApplication { rule, state_match },
    ))
}

/// Evaluates an available once-only rewrite rule against the current runtime state.
fn attempt_once_rewrite_rule<'program, 'state, 'once>(
    rule: OnceRewriteRuleView<'program>,
    commit: OnceMatchPermit<'once>,
    state: &'state State,
) -> AvailableRuleAttempt<'program, 'state, 'once> {
    let state_match = match match_rule_state(rule.into_rule().pattern(), state) {
        RuleStateMatch::Matched(state_match) => state_match,
        RuleStateMatch::Mismatched => {
            return AvailableRuleAttempt::StateMismatch(RuleAttemptMiss::state_mismatch(
                RuleView::OnceRewrite(rule),
            ));
        }
    };

    AvailableRuleAttempt::Matched(MatchedRuleApplication::OnceRewrite(
        MatchedOnceRewriteApplication {
            rule,
            state_match,
            commit,
        },
    ))
}

/// Evaluates an available reusable return rule against the current runtime state.
fn attempt_always_return_rule<'program, 'state, 'once>(
    rule: AlwaysReturnRuleView<'program>,
    state: &'state State,
) -> AvailableRuleAttempt<'program, 'state, 'once> {
    let state_match = match match_rule_state(rule.into_rule().pattern(), state) {
        RuleStateMatch::Matched(state_match) => state_match,
        RuleStateMatch::Mismatched => {
            return AvailableRuleAttempt::StateMismatch(RuleAttemptMiss::state_mismatch(
                RuleView::AlwaysReturn(rule),
            ));
        }
    };

    AvailableRuleAttempt::Matched(MatchedRuleApplication::AlwaysReturn(
        MatchedAlwaysReturnApplication { rule, state_match },
    ))
}

/// Evaluates an available once-only return rule against the current runtime state.
fn attempt_once_return_rule<'program, 'state, 'once>(
    rule: OnceReturnRuleView<'program>,
    commit: OnceMatchPermit<'once>,
    state: &'state State,
) -> AvailableRuleAttempt<'program, 'state, 'once> {
    let state_match = match match_rule_state(rule.into_rule().pattern(), state) {
        RuleStateMatch::Matched(state_match) => state_match,
        RuleStateMatch::Mismatched => {
            return AvailableRuleAttempt::StateMismatch(RuleAttemptMiss::state_mismatch(
                RuleView::OnceReturn(rule),
            ));
        }
    };

    AvailableRuleAttempt::Matched(MatchedRuleApplication::OnceReturn(
        MatchedOnceReturnApplication {
            rule,
            state_match,
            commit,
        },
    ))
}

/// Compares a single parsed rule pattern with the current runtime state.
fn match_rule_state<'state>(pattern: &RulePattern, state: &'state State) -> RuleStateMatch<'state> {
    match find_match(state, pattern) {
        Some(state_match) => RuleStateMatch::Matched(state_match),
        None => RuleStateMatch::Mismatched,
    }
}

/// Finds this rule pattern's match span in the current state.
fn find_match<'state>(state: &'state State, pattern: &RulePattern) -> Option<StateMatch<'state>> {
    match pattern.anchor() {
        RuleAnchorSyntax::Anywhere => state.find_payload(pattern.lhs()),
        RuleAnchorSyntax::Start => state.starts_with_payload(pattern.lhs()),
        RuleAnchorSyntax::End => state.ends_with_payload(pattern.lhs()),
    }
}
