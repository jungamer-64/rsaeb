use super::once::{AvailableRuleCommit, MatchedRuleCommit, OnceStateSet};
use super::state::{MatchedStateSpan, State};
use crate::inspect::{RulePosition, RulePositions};
use crate::rule::{Rule, RuleAnchorSyntax};

#[derive(Debug)]
pub(crate) enum RuleSearch<'program, 'once> {
    Matched(MatchedRule<'program, 'once>),
    Stable,
}

#[derive(Debug)]
pub(crate) struct MatchedRule<'program, 'once> {
    pub(crate) position: RulePosition,
    pub(crate) rule: &'program Rule,
    pub(crate) commit: MatchedRuleCommit<'once>,
    pub(crate) state_match: MatchedStateSpan,
}

#[derive(Debug, Clone, Copy)]
struct MatchedRuleCandidate<'program> {
    position: RulePosition,
    rule: &'program Rule,
    commit: AvailableRuleCommit,
    state_match: MatchedStateSpan,
}

pub(crate) fn find_next_match<'program, 'once>(
    rules: &'program [Rule],
    once_states: &'once mut OnceStateSet,
    state: &State,
) -> RuleSearch<'program, 'once> {
    let Some(candidate) = find_next_match_candidate(rules, once_states, state) else {
        return RuleSearch::Stable;
    };

    let Some(commit) = once_states.commit_token(candidate.commit) else {
        return RuleSearch::Stable;
    };

    RuleSearch::Matched(MatchedRule {
        position: candidate.position,
        rule: candidate.rule,
        commit,
        state_match: candidate.state_match,
    })
}

fn find_next_match_candidate<'program>(
    rules: &'program [Rule],
    once_states: &OnceStateSet,
    state: &State,
) -> Option<MatchedRuleCandidate<'program>> {
    for (rule, position) in rules.iter().zip(RulePositions::new()) {
        let Some(available_commit) = once_states.available_commit_for_rule(rule) else {
            continue;
        };

        let Some(state_match) = find_match(state, rule) else {
            continue;
        };

        return Some(MatchedRuleCandidate {
            position,
            rule,
            commit: available_commit,
            state_match,
        });
    }

    None
}

fn find_match(state: &State, rule: &Rule) -> Option<MatchedStateSpan> {
    match rule.anchor() {
        RuleAnchorSyntax::Anywhere => state.find_payload(rule.lhs()),
        RuleAnchorSyntax::Start => state.starts_with_payload(rule.lhs()),
        RuleAnchorSyntax::End => state.ends_with_payload(rule.lhs()),
    }
}
