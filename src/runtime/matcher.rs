use super::once::{MatchedRuleCommit, RuntimeRules};
use super::state::{MatchedStateSpan, State};
use crate::inspect::RuleAnchor;
use crate::rule::Rule;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RuleSearch<'program, 'runtime> {
    Matched(MatchedRule<'program, 'runtime>),
    Stable,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MatchedRule<'program, 'runtime> {
    pub(crate) rule: &'program Rule,
    pub(crate) commit: MatchedRuleCommit<'runtime>,
    pub(crate) state_match: MatchedStateSpan,
}

pub(crate) fn find_next_match<'program, 'runtime>(
    rules: &'runtime mut RuntimeRules<'program>,
    state: &State,
) -> RuleSearch<'program, 'runtime> {
    for (rule, commit) in rules.iter_available_mut() {
        let Some(state_match) = find_match(state, rule) else {
            continue;
        };

        return RuleSearch::Matched(MatchedRule {
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
