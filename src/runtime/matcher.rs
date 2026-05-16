use super::once::{MatchedRuleCommit, OnceStateSet};
use super::state::{MatchedStateSpan, State};
use crate::inspect::{RuleAnchor, RulePosition, RulePositions};
use crate::rule::Rule;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RuleSearch<'program> {
    Matched(MatchedRule<'program>),
    Stable,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MatchedRule<'program> {
    pub(crate) position: RulePosition,
    pub(crate) rule: &'program Rule,
    pub(crate) commit: MatchedRuleCommit,
    pub(crate) state_match: MatchedStateSpan,
}

pub(crate) fn find_next_match<'program>(
    rules: &'program [Rule],
    once_states: &OnceStateSet,
    state: &State,
) -> RuleSearch<'program> {
    for (rule, position) in rules.iter().zip(RulePositions::new()) {
        let Some(commit) = once_states.commit_token_for_rule(rule) else {
            continue;
        };

        let Some(state_match) = find_match(state, rule) else {
            continue;
        };

        return RuleSearch::Matched(MatchedRule {
            position,
            rule,
            commit,
            state_match,
        });
    }

    RuleSearch::Stable
}

fn find_match(state: &State, rule: &Rule) -> Option<MatchedStateSpan> {
    match rule.anchor() {
        RuleAnchor::Anywhere => state.find_payload(rule.lhs()),
        RuleAnchor::Start => state.starts_with_payload(rule.lhs()),
        RuleAnchor::End => state.ends_with_payload(rule.lhs()),
    }
}
