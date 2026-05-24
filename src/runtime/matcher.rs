use super::once::{MatchedRuleCommit, OnceStateSet};
use super::state::{State, StateMatch};
use crate::error::{RunError, RunInvariantError};
use crate::inspect::RuleView;
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
    /// Parsed rule selected by the matcher.
    rule: &'program Rule,
    /// Once-state side effect to apply only after successful rewrite.
    commit: MatchedRuleCommit<'once>,
    /// Runtime-state range matched by the rule left side.
    state_match: StateMatch,
}

/// Rule candidate before a linear commit permit has been reserved.
struct MatchedRuleCandidate<'program> {
    /// Parsed rule selected as a candidate.
    rule: &'program Rule,
    /// Runtime-state range matched by the rule left side.
    state_match: StateMatch,
}

/// Rule view after all runtime side effects have committed.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CommittedRule<'program> {
    /// Structured view of the committed parsed rule.
    rule: RuleView<'program>,
}

impl<'program> MatchedRuleCandidate<'program> {
    /// Captures a rule match before once-state commit is permitted.
    const fn new(rule: &'program Rule, state_match: StateMatch) -> Self {
        Self { rule, state_match }
    }

    /// Attaches the linear commit action to the matched candidate.
    const fn into_application<'once>(
        self,
        commit: MatchedRuleCommit<'once>,
    ) -> MatchedRuleApplication<'program, 'once> {
        MatchedRuleApplication::new(self.rule, self.state_match, commit)
    }
}

impl<'program, 'once> MatchedRuleApplication<'program, 'once> {
    /// Captures the complete data needed to apply a matched rule.
    const fn new(
        rule: &'program Rule,
        state_match: StateMatch,
        commit: MatchedRuleCommit<'once>,
    ) -> Self {
        Self {
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
    pub(crate) fn commit(self) -> CommittedRule<'program> {
        self.commit.commit();
        CommittedRule {
            rule: RuleView::new(self.rule),
        }
    }
}

impl<'program> CommittedRule<'program> {
    /// Structured view of the committed rule.
    pub(crate) const fn rule(self) -> RuleView<'program> {
        self.rule
    }
}

/// Finds the first currently available rule that matches `state`.
///
/// # Errors
///
/// Returns `RunError` if the parsed rule table and per-run `(once)` state
/// table no longer have the same length.
pub(crate) fn find_next_match<'program, 'once>(
    rules: &'program [Rule],
    once_states: &'once mut OnceStateSet,
    state: &State,
) -> Result<RuleSearch<'program, 'once>, RunError> {
    let rule_count = crate::inspect::RuleCount::new(rules.len());
    let state_count = once_states.row_count();
    if rule_count != state_count {
        return Err(RunInvariantError::RuleStateLengthMismatch {
            rules: rule_count,
            states: state_count,
        }
        .into());
    }

    for (rule, rule_state) in rules.iter().zip(once_states.rows_mut()) {
        let Some(commit) = rule_state.reserve_commit() else {
            continue;
        };
        let Some(candidate) = matched_candidate_for_rule(rule, state) else {
            continue;
        };

        return Ok(RuleSearch::Matched(candidate.into_application(commit)));
    }

    Ok(RuleSearch::Stable)
}

/// Builds a committed-rule candidate for a single parsed rule.
fn matched_candidate_for_rule<'program>(
    rule: &'program Rule,
    state: &State,
) -> Option<MatchedRuleCandidate<'program>> {
    let state_match = find_match(state, rule)?;
    Some(MatchedRuleCandidate::new(rule, state_match))
}

/// Finds this rule's match span in the current state.
fn find_match(state: &State, rule: &Rule) -> Option<StateMatch> {
    match rule.anchor() {
        RuleAnchorSyntax::Anywhere => state.find_payload(rule.lhs()),
        RuleAnchorSyntax::Start => state.starts_with_payload(rule.lhs()),
        RuleAnchorSyntax::End => state.ends_with_payload(rule.lhs()),
    }
}
