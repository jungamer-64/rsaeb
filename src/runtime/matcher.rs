use super::once::{MatchedRuleCommit, OnceRuleAvailability, OnceStateSet};
use super::state::{State, StateMatch};
use crate::error::RunError;
use crate::inspect::{RulePosition, RulePositions};
use crate::rule::{Rule, RuleAnchorSyntax};

/// Internal rule search alternatives.
#[derive(Debug)]
pub(crate) enum RuleSearch<'program, 'once> {
    /// Matched case.
    Matched(MatchedRuleApplication<'program, 'once>),
    /// Stable case.
    Stable,
}

/// Internal matched rule application.
#[derive(Debug)]
pub(crate) struct MatchedRuleApplication<'program, 'once> {
    /// Stored position.
    position: RulePosition,
    /// Stored rule.
    rule: &'program Rule,
    /// Stored commit.
    commit: MatchedRuleCommit<'once>,
    /// Stored state match.
    state_match: StateMatch,
}

/// Internal matched rule candidate.
struct MatchedRuleCandidate<'program> {
    /// Stored position.
    position: RulePosition,
    /// Stored rule.
    rule: &'program Rule,
    /// Stored state match.
    state_match: StateMatch,
}

/// Internal committed rule.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CommittedRule {
    /// Stored position.
    position: RulePosition,
}

impl<'program> MatchedRuleCandidate<'program> {
    /// Constructs the value from validated parts.
    const fn new(position: RulePosition, rule: &'program Rule, state_match: StateMatch) -> Self {
        Self {
            position,
            rule,
            state_match,
        }
    }

    /// Runs the into application operation.
    const fn into_application<'once>(
        self,
        commit: MatchedRuleCommit<'once>,
    ) -> MatchedRuleApplication<'program, 'once> {
        MatchedRuleApplication::new(self.position, self.rule, self.state_match, commit)
    }
}

impl<'program, 'once> MatchedRuleApplication<'program, 'once> {
    /// Constructs the value from validated parts.
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

    /// Runs the rule operation.
    pub(crate) const fn rule(&self) -> &'program Rule {
        self.rule
    }

    /// Runs the state match operation.
    pub(crate) const fn state_match(&self) -> StateMatch {
        self.state_match
    }

    /// Runs the commit operation.
    pub(crate) fn commit(self) -> CommittedRule {
        self.commit.commit();
        CommittedRule {
            position: self.position,
        }
    }
}

impl CommittedRule {
    /// Runs the position operation.
    pub(crate) const fn position(self) -> RulePosition {
        self.position
    }
}

/// Finds the first currently available rule that matches `state`.
///
/// # Errors
///
/// Returns `RunError::InternalInvariant` if once-rule metadata or state-match
/// ranges no longer resolve against their owning runtime structures.
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

/// Builds a committed-rule candidate for a single parsed rule.
///
/// # Errors
///
/// Returns `RunError::InternalInvariant` if the rule's once slot or matched
/// state range is invalid for this run.
fn matched_candidate_for_rule<'program>(
    rule: &'program Rule,
    position: RulePosition,
    once_states: &OnceStateSet,
    state: &State,
) -> Result<Option<MatchedRuleCandidate<'program>>, RunError> {
    let Some(state_match) = find_match(state, rule)? else {
        return Ok(None);
    };
    match once_states.availability_for_rule(rule)? {
        OnceRuleAvailability::Available => {}
        OnceRuleAvailability::Consumed => return Ok(None),
    }
    Ok(Some(MatchedRuleCandidate::new(position, rule, state_match)))
}

/// Finds this rule's match span in the current state.
///
/// # Errors
///
/// Returns `RunError::InternalInvariant` if a derived state-match range no
/// longer resolves inside the current runtime state.
fn find_match(state: &State, rule: &Rule) -> Result<Option<StateMatch>, RunError> {
    match rule.anchor() {
        RuleAnchorSyntax::Anywhere => state.find_payload(rule.lhs()),
        RuleAnchorSyntax::Start => state.starts_with_payload(rule.lhs()),
        RuleAnchorSyntax::End => state.ends_with_payload(rule.lhs()),
    }
}
