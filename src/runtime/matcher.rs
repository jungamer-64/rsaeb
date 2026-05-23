use super::once::{MatchedRuleCommit, OnceRuleUnavailable, OnceStateSet, RuleAvailability};
use super::state::{State, StateMatch};
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

#[derive(Debug, Clone, Copy)]
pub(crate) struct CommittedRule<'program> {
    position: RulePosition,
    rule: &'program Rule,
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
    once_states: &'once OnceStateSet,
    state: &State,
) -> RuleSearch<'program, 'once> {
    for (rule, position) in rules.iter().zip(RulePositions::new()) {
        let Some(application) = matched_application_for_rule(rule, position, once_states, state)
        else {
            continue;
        };

        return RuleSearch::Matched(application);
    }

    RuleSearch::Stable
}

fn matched_application_for_rule<'program, 'once>(
    rule: &'program Rule,
    position: RulePosition,
    once_states: &'once OnceStateSet,
    state: &State,
) -> Option<MatchedRuleApplication<'program, 'once>> {
    let state_match = find_match(state, rule)?;
    let commit = match once_states.availability_for_rule(rule) {
        RuleAvailability::Available(commit) => commit,
        RuleAvailability::Unavailable(
            OnceRuleUnavailable::Consumed | OnceRuleUnavailable::MissingSlot,
        ) => return None,
    };
    Some(MatchedRuleApplication::new(
        position,
        rule,
        state_match,
        commit,
    ))
}

fn find_match(state: &State, rule: &Rule) -> Option<StateMatch> {
    match rule.anchor() {
        RuleAnchorSyntax::Anywhere => state.find_payload(rule.lhs()),
        RuleAnchorSyntax::Start => state.starts_with_payload(rule.lhs()),
        RuleAnchorSyntax::End => state.ends_with_payload(rule.lhs()),
    }
}
