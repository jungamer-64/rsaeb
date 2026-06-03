use super::once::AvailableRuntimeRule;
use super::state::{State, StateMatch};
use crate::bytes::Payload;
use crate::inspect::RuleView;
use crate::rule::{ReturnRule, RewriteAction, RewriteRule, RuleAnchorSyntax, RulePattern};

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
    /// Matched non-terminal rewrite rule.
    Rewrite(MatchedRewriteApplication<'program, 'state, 'once>),
    /// Matched terminal return rule.
    Return(MatchedReturnApplication<'program, 'state, 'once>),
}

/// Matched non-terminal rewrite rule.
#[derive(Debug)]
pub(crate) struct MatchedRewriteApplication<'program, 'state, 'once> {
    /// Parsed rule selected by the matcher.
    rule: RuleView<'program>,
    /// Right-side rewrite action selected by the matched rule.
    action: &'program RewriteAction,
    /// Runtime-state range matched by the rule left side.
    state_match: StateMatch<'state>,
}

/// Matched terminal return rule.
#[derive(Debug)]
pub(crate) struct MatchedReturnApplication<'program, 'state, 'once> {
    /// Parsed rule selected by the matcher.
    rule: RuleView<'program>,
    /// Right-side return output selected by the matched rule.
    output: &'program Payload,
    /// Runtime-state range matched by the rule left side.
    state_match: StateMatch<'state>,
}

/// Non-applying rule consumed by a rule-attempt step.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RuleAttemptMiss<'program> {
    /// Parsed rule selected as the attempted rule line.
    rule: RuleView<'program>,
}

/// Domain result of comparing one rule's left side with the runtime state.
enum RuleStateMatch<'state> {
    /// Rule left side matched and carries the matched state span.
    Matched(StateMatch<'state>),
    /// Rule left side did not match the runtime state.
    Mismatched,
}

impl<'program> RuleAttemptMiss<'program> {
    /// Captures a consumed non-applying rule line.
    pub(crate) const fn new(rule: RuleView<'program>) -> Self {
        Self { rule }
    }

    /// Parsed rule selected as the attempted rule line.
    pub(crate) const fn rule(self) -> RuleView<'program> {
        self.rule
    }
}

/// Evaluates one already-available parsed rule line against the current runtime state.
pub(crate) fn attempt_available_rule<'program, 'state, 'once>(
    runtime_rule: AvailableRuntimeRule<'program, 'once>,
    state: &'state State,
) -> AvailableRuleAttempt<'program, 'state, 'once> {
    match runtime_rule {
        AvailableRuntimeRule::AlwaysRewrite(rule) => attempt_rewrite_rule(
            rule.rule(),
            RuleView::from_always_rewrite(rule.rule()),
            state,
        ),
        AvailableRuntimeRule::OnceRewrite(rule) => {
            let (rule, _commit) = rule.into_parts();
            attempt_rewrite_rule(
                rule,
                RuleView::from_once_rewrite(rule),
                state,
            )
        }
        AvailableRuntimeRule::AlwaysReturn(rule) => attempt_return_rule(
            rule.rule(),
            RuleView::from_always_return(rule.rule()),
            state,
        ),
        AvailableRuntimeRule::OnceReturn(rule) => {
            let (rule, _commit) = rule.into_parts();
            attempt_return_rule(
                rule,
                RuleView::from_once_return(rule),
                state,
            )
        }
    }
}

/// Evaluates an available rewrite rule against the current runtime state.
fn attempt_rewrite_rule<'program, 'state, 'once>(
    rule: &'program RewriteRule,
    rule_view: RuleView<'program>,
    state: &'state State,
) -> AvailableRuleAttempt<'program, 'state, 'once> {
    let state_match = match match_rule_state(rule.pattern(), state) {
        RuleStateMatch::Matched(state_match) => state_match,
        RuleStateMatch::Mismatched => {
            return AvailableRuleAttempt::StateMismatch(RuleAttemptMiss::new(rule_view));
        }
    };

    AvailableRuleAttempt::Matched(MatchedRuleApplication::Rewrite(MatchedRewriteApplication {
        rule: rule_view,
        action: rule.rewrite_action(),
        state_match,
    }))
}

/// Evaluates an available return rule against the current runtime state.
fn attempt_return_rule<'program, 'state, 'once>(
    rule: &'program ReturnRule,
    rule_view: RuleView<'program>,
    state: &'state State,
) -> AvailableRuleAttempt<'program, 'state, 'once> {
    let state_match = match match_rule_state(rule.pattern(), state) {
        RuleStateMatch::Matched(state_match) => state_match,
        RuleStateMatch::Mismatched => {
            return AvailableRuleAttempt::StateMismatch(RuleAttemptMiss::new(rule_view));
        }
    };

    AvailableRuleAttempt::Matched(MatchedRuleApplication::Return(MatchedReturnApplication {
        rule: rule_view,
        output: rule.output(),
        state_match,
    }))
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
