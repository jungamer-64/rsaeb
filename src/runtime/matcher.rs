use super::once::{MatchedRuleCommit, OnceRuleAvailability, OnceStateSet};
use super::state::{State, StateMatch};
use crate::error::RunError;
use crate::inspect::{RulePosition, RulePositions};
use crate::rule::{Rule, RuleAnchorSyntax};

#[derive(Debug)]
pub(crate) enum RuleSearch<'program, 'once> {
    Matched(MatchedRuleApplication<'program, 'once>),
    Stable,
}

#[derive(Debug)]
pub(crate) struct MatchedRuleApplication<'program, 'once> {
    position: RulePosition,
    rule: &'program Rule,
    commit: MatchedRuleCommit<'once>,
    state_match: StateMatch,
}

struct MatchedRuleCandidate<'program> {
    position: RulePosition,
    rule: &'program Rule,
    state_match: StateMatch,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CommittedRule<'program> {
    position: RulePosition,
    rule: &'program Rule,
}

impl<'program> MatchedRuleCandidate<'program> {
    const fn new(position: RulePosition, rule: &'program Rule, state_match: StateMatch) -> Self {
        Self {
            position,
            rule,
            state_match,
        }
    }

    const fn into_application<'once>(
        self,
        commit: MatchedRuleCommit<'once>,
    ) -> MatchedRuleApplication<'program, 'once> {
        MatchedRuleApplication::new(self.position, self.rule, self.state_match, commit)
    }
}

impl<'program, 'once> MatchedRuleApplication<'program, 'once> {
    const fn new(
        position: RulePosition,
        rule: &'program Rule,
        state_match: StateMatch,
        commit: MatchedRuleCommit<'once>,
    ) -> Self {
        Self {
            position,
            rule,
            commit,
            state_match,
        }
    }

    pub(crate) const fn rule(&self) -> &'program Rule {
        self.rule
    }

    pub(crate) const fn state_match(&self) -> StateMatch {
        self.state_match
    }

    pub(crate) fn commit(self) -> CommittedRule<'program> {
        self.commit.commit();
        CommittedRule {
            position: self.position,
            rule: self.rule,
        }
    }
}

impl<'program> CommittedRule<'program> {
    pub(crate) const fn view(self) -> crate::inspect::RuleView<'program> {
        crate::inspect::RuleView::new(self.position, self.rule)
    }
}

pub(crate) fn find_next_match<'program, 'once>(
    rules: &'program [Rule],
    once_states: &'once mut OnceStateSet,
    state: &State,
) -> Result<RuleSearch<'program, 'once>, RunError> {
    for (rule, position) in rules.iter().zip(RulePositions::new()) {
        let Some(candidate) = matched_candidate_for_rule(rule, position, once_states, state)?
        else {
            continue;
        };

        let commit = once_states.commit_for_available_rule(rule)?;
        return Ok(RuleSearch::Matched(candidate.into_application(commit)));
    }

    Ok(RuleSearch::Stable)
}

fn matched_candidate_for_rule<'program>(
    rule: &'program Rule,
    position: RulePosition,
    once_states: &OnceStateSet,
    state: &State,
) -> Result<Option<MatchedRuleCandidate<'program>>, RunError> {
    let Some(state_match) = find_match(state, rule) else {
        return Ok(None);
    };
    match once_states.availability_for_rule(rule)? {
        OnceRuleAvailability::Available => {}
        OnceRuleAvailability::Consumed => return Ok(None),
    };
    Ok(Some(MatchedRuleCandidate::new(position, rule, state_match)))
}

fn find_match(state: &State, rule: &Rule) -> Option<StateMatch> {
    match rule.anchor() {
        RuleAnchorSyntax::Anywhere => state.find_payload(rule.lhs()),
        RuleAnchorSyntax::Start => state.starts_with_payload(rule.lhs()),
        RuleAnchorSyntax::End => state.ends_with_payload(rule.lhs()),
    }
}
