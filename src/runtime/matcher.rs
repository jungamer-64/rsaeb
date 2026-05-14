use super::once::{MatchedRuleSchedule, OnceRunStates, RuleEligibility};
use super::state::{MatchedStateSpan, State};
use crate::error::RunError;
use crate::rule::{Rule, RuleAnchor};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuleSearch<'program> {
    Matched(MatchedRule<'program>),
    Stable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MatchedRule<'program> {
    pub(super) rule: &'program Rule,
    pub(super) schedule: MatchedRuleSchedule,
    pub(super) state_match: MatchedStateSpan,
}

pub(super) fn find_next_match<'program>(
    rules: &'program [Rule],
    state: &State,
    once_states: &OnceRunStates,
) -> Result<RuleSearch<'program>, RunError> {
    for rule in rules {
        let schedule = match once_states.eligibility(rule.schedule())? {
            RuleEligibility::Eligible(schedule) => schedule,
            RuleEligibility::ConsumedOnce => continue,
        };

        let Some(state_match) = find_match(state, rule) else {
            continue;
        };

        return Ok(RuleSearch::Matched(MatchedRule {
            rule,
            schedule,
            state_match,
        }));
    }

    Ok(RuleSearch::Stable)
}

fn find_match(state: &State, rule: &Rule) -> Option<MatchedStateSpan> {
    match rule.anchor() {
        RuleAnchor::Anywhere => state.find_payload(rule.lhs()),
        RuleAnchor::Start => state.starts_with_payload(rule.lhs()),
        RuleAnchor::End => state.ends_with_payload(rule.lhs()),
    }
}
