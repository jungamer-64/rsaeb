use super::once::{AvailableRuntimeRule, MatchedRuleCommit};
use super::state::{State, StateMatch};
use crate::bytes::Payload;
use crate::inspect::{AlwaysRepeat, RuleView};
use crate::rule::{RepeatRule, RewriteAction, RuleAnchorSyntax, RulePattern};

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

/// Reason a consumed executable rule line did not apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleMissReason {
    /// The rule is available, but its left side does not match the current state.
    StateMismatch,
    /// The rule is a `(once)` rule that has already committed in this run.
    OnceConsumed,
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
    /// Once-state side effect to apply only after successful rewrite.
    commit: MatchedRuleCommit<'once>,
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
    /// Once-state side effect to apply only after successful return materialization.
    commit: MatchedRuleCommit<'once>,
    /// Runtime-state range matched by the rule left side.
    state_match: StateMatch<'state>,
}

/// Matched rule after runtime-state match data has been consumed for preparation.
#[derive(Debug)]
pub(crate) struct PreparedMatchedRule<'program, 'once> {
    /// Parsed rule selected by the matcher.
    rule: RuleView<'program>,
    /// Once-state side effect to apply only after successful rewrite.
    commit: MatchedRuleCommit<'once>,
}

/// Action-specific data after runtime-state match data has been split out.
pub(crate) enum MatchedRuleAction<'program, 'once> {
    /// Prepared rewrite rule data.
    Rewrite {
        /// Matched rule and deferred once-state commit.
        matched: PreparedMatchedRule<'program, 'once>,
        /// Right-side rewrite action.
        action: &'program RewriteAction,
    },
    /// Prepared return rule data.
    Return {
        /// Matched rule and deferred once-state commit.
        matched: PreparedMatchedRule<'program, 'once>,
        /// Right-side return output.
        output: &'program Payload,
    },
}

/// Non-applying rule consumed by a rule-attempt step.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RuleAttemptMiss<'program> {
    /// Parsed rule selected as the attempted rule line.
    rule: RuleView<'program>,
    /// Reason the attempted rule did not apply.
    reason: RuleMissReason,
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
    pub(crate) const fn new(rule: RuleView<'program>, reason: RuleMissReason) -> Self {
        Self { rule, reason }
    }

    /// Parsed rule selected as the attempted rule line.
    pub(crate) const fn rule(self) -> RuleView<'program> {
        self.rule
    }

    /// Reason the attempted rule did not apply.
    pub(crate) const fn reason(self) -> RuleMissReason {
        self.reason
    }
}

impl<'program, 'state, 'once> MatchedRuleApplication<'program, 'state, 'once> {
    /// Splits the state-match witness from action-specific commit data.
    pub(crate) fn into_prepare_parts(
        self,
    ) -> (StateMatch<'state>, MatchedRuleAction<'program, 'once>) {
        match self {
            Self::Rewrite(matched) => (
                matched.state_match,
                MatchedRuleAction::Rewrite {
                    matched: PreparedMatchedRule {
                        rule: matched.rule,
                        commit: matched.commit,
                    },
                    action: matched.action,
                },
            ),
            Self::Return(matched) => (
                matched.state_match,
                MatchedRuleAction::Return {
                    matched: PreparedMatchedRule {
                        rule: matched.rule,
                        commit: matched.commit,
                    },
                    output: matched.output,
                },
            ),
        }
    }
}

impl<'program> PreparedMatchedRule<'program, '_> {
    /// Parsed rule selected by the matcher.
    pub(crate) const fn rule(&self) -> RuleView<'program> {
        self.rule
    }

    /// Commits the matched rule's deferred side effects.
    pub(crate) fn commit(self) {
        self.commit.commit();
    }
}

/// Evaluates one already-available parsed rule line against the current runtime state.
pub(crate) fn attempt_available_rule<'program, 'state, 'once>(
    runtime_rule: AvailableRuntimeRule<'program, 'once>,
    state: &'state State,
) -> AvailableRuleAttempt<'program, 'state, 'once> {
    match runtime_rule {
        AvailableRuntimeRule::Always(rule) => attempt_always_rule(rule.rule(), state),
        AvailableRuntimeRule::Once(rule) => {
            let rule_view = RuleView::from_once(rule.rule());
            let (rule, commit) = rule.into_parts();
            attempt_repeat_rule(rule, rule_view, MatchedRuleCommit::Once(commit), state)
        }
    }
}

/// Evaluates an always-available repeat-axis rule.
fn attempt_always_rule<'program, 'state, 'once>(
    rule: &'program RepeatRule<AlwaysRepeat>,
    state: &'state State,
) -> AvailableRuleAttempt<'program, 'state, 'once> {
    attempt_repeat_rule(
        rule,
        RuleView::from_always(rule),
        MatchedRuleCommit::Always,
        state,
    )
}

/// Evaluates an available repeat-axis rule against the current runtime state.
fn attempt_repeat_rule<'program, 'state, 'once, R>(
    rule: &'program RepeatRule<R>,
    rule_view: RuleView<'program>,
    commit: MatchedRuleCommit<'once>,
    state: &'state State,
) -> AvailableRuleAttempt<'program, 'state, 'once> {
    let state_match = match match_rule_state(rule.pattern(), state) {
        RuleStateMatch::Matched(state_match) => state_match,
        RuleStateMatch::Mismatched => {
            return AvailableRuleAttempt::StateMismatch(RuleAttemptMiss::new(
                rule_view,
                RuleMissReason::StateMismatch,
            ));
        }
    };

    match rule {
        RepeatRule::Rewrite(rule) => AvailableRuleAttempt::Matched(
            MatchedRuleApplication::Rewrite(MatchedRewriteApplication {
                rule: rule_view,
                action: rule.rewrite_action(),
                commit,
                state_match,
            }),
        ),
        RepeatRule::Return(rule) => AvailableRuleAttempt::Matched(MatchedRuleApplication::Return(
            MatchedReturnApplication {
                rule: rule_view,
                output: rule.output(),
                commit,
                state_match,
            },
        )),
    }
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
