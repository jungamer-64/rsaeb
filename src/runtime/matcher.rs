use super::once::{MatchedRuleCommit, OnceRuleAvailability, OnceStateSet};
use super::state::{State, StateMatch};
use crate::error::RunError;
use crate::inspect::{RulePosition, RulePositions};
use crate::rule::{Rule, RuleAnchorSyntax};

/// Outcome of scanning the rule table for the next applicable rule.
#[derive(Debug)]
pub(crate) enum RuleSearch<'program, 'once> {
    /// A rule matched and carries the commit permit needed after success.
    Matched(MatchedRuleApplication<'program, 'once>),
    /// No currently available rule matched the runtime state.
    Stable,
}

/// Matched rule plus the state range and commit action needed to apply it.
#[derive(Debug)]
pub(crate) struct MatchedRuleApplication<'program, 'once> {
    /// Execution-order position of the matched rule.
    position: RulePosition,
    /// Parsed rule selected by the matcher.
    rule: &'program Rule,
    /// Once-state side effect to apply only after successful rewrite.
    commit: MatchedRuleCommit<'once>,
    /// Runtime-state range matched by the rule left side.
    state_match: StateMatch,
}

/// Rule candidate before a linear commit permit has been reserved.
struct MatchedRuleCandidate<'program> {
    /// Execution-order position of the candidate rule.
    position: RulePosition,
    /// Parsed rule selected as a candidate.
    rule: &'program Rule,
    /// Runtime-state range matched by the rule left side.
    state_match: StateMatch,
}

/// Rule position after all runtime side effects have committed.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CommittedRule {
    /// Execution-order position safe to expose in the transition.
    position: RulePosition,
}

impl<'program> MatchedRuleCandidate<'program> {
    /// Captures a rule match before once-state commit is permitted.
    const fn new(position: RulePosition, rule: &'program Rule, state_match: StateMatch) -> Self {
        Self {
            position,
            rule,
            state_match,
        }
    }

    /// Attaches the linear commit action to the matched candidate.
    const fn into_application<'once>(
        self,
        commit: MatchedRuleCommit<'once>,
    ) -> MatchedRuleApplication<'program, 'once> {
        MatchedRuleApplication::new(self.position, self.rule, self.state_match, commit)
    }
}

impl<'program, 'once> MatchedRuleApplication<'program, 'once> {
    /// Captures the complete data needed to apply a matched rule.
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

    /// Parsed rule selected by the matcher.
    pub(crate) const fn rule(&self) -> &'program Rule {
        self.rule
    }

    /// Runtime-state range matched by the selected rule.
    pub(crate) const fn state_match(&self) -> StateMatch {
        self.state_match
    }

    /// Commits the matched rule's deferred side effects.
    pub(crate) fn commit(self) -> CommittedRule {
        self.commit.commit();
        CommittedRule {
            position: self.position,
        }
    }
}

impl CommittedRule {
    /// Execution-order position of the committed rule.
    pub(crate) const fn position(self) -> RulePosition {
        self.position
    }
}

/// Finds the first currently available rule that matches `state`.
///
/// # Errors
///
/// Returns `RunError::InternalInvariant` if once-rule metadata no longer
/// resolves against its owning runtime structure.
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
    let Some(state_match) = find_match(state, rule) else {
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
fn find_match(state: &State, rule: &Rule) -> Option<StateMatch> {
    match rule.anchor() {
        RuleAnchorSyntax::Anywhere => state.find_payload(rule.lhs()),
        RuleAnchorSyntax::Start => state.starts_with_payload(rule.lhs()),
        RuleAnchorSyntax::End => state.ends_with_payload(rule.lhs()),
    }
}
