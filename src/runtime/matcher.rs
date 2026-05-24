use super::once::{MatchedRuleCommit, OnceRuleAvailability, OnceStateSet};
use super::state::{State, StateMatch};
use crate::error::RunError;
use crate::inspect::RulePosition;
use crate::rule::{Rule, RuleAnchorSyntax};

/// Outcome of scanning the rule table for the next applicable rule.
#[derive(Debug)]
pub(crate) enum RuleSearch<'program, 'once> {
    /// A rule matched and carries the commit permit needed after success.
    Matched(MatchedRuleApplication<'program, 'once>),
    /// No currently available rule matched the runtime state.
    Stable,
}

/// Outcome of evaluating one executable rule line against the current state.
#[derive(Debug)]
pub(crate) enum RuleAttempt<'program, 'once> {
    /// The rule matched and carries the commit permit needed after success.
    Matched(MatchedRuleApplication<'program, 'once>),
    /// The rule was consumed by the attempt but did not apply.
    Missed(RuleAttemptMiss<'program>),
}

/// Reason a consumed executable rule line did not apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleMissReason {
    /// The rule is available, but its left side does not match the current state.
    StateMismatch,
    /// The rule is a `(once)` rule that has already committed in this run.
    OnceConsumed,
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

/// Non-applying rule consumed by a rule-attempt step.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RuleAttemptMiss<'program> {
    /// Parsed rule selected as the attempted rule line.
    rule: &'program Rule,
    /// Reason the attempted rule did not apply.
    reason: RuleMissReason,
}

/// Rule candidate before a linear commit permit has been reserved.
struct MatchedRuleCandidate<'program> {
    /// Parsed rule selected as a candidate.
    rule: &'program Rule,
    /// Runtime-state range matched by the rule left side.
    state_match: StateMatch,
}

/// Applicability of one executable rule before a once commit permit is reserved.
enum RuleInspection<'program> {
    /// The rule matched runtime state and can reserve a commit permit.
    Candidate(MatchedRuleCandidate<'program>),
    /// The rule did not apply.
    Missed(RuleAttemptMiss<'program>),
}

/// Rule view after all runtime side effects have committed.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CommittedRule<'program> {
    /// Program-local position of the committed parsed rule.
    rule_position: RulePosition,
    /// Marker tying this witness to the parsed rule lifetime.
    _program: core::marker::PhantomData<&'program Rule>,
}

impl<'program> RuleAttemptMiss<'program> {
    /// Captures a consumed non-applying rule line.
    const fn new(rule: &'program Rule, reason: RuleMissReason) -> Self {
        Self { rule, reason }
    }

    /// Parsed rule selected as the attempted rule line.
    pub(crate) const fn rule(self) -> &'program Rule {
        self.rule
    }

    /// Reason the attempted rule did not apply.
    pub(crate) const fn reason(self) -> RuleMissReason {
        self.reason
    }
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
            rule_position: self.rule.position(),
            _program: core::marker::PhantomData,
        }
    }
}

impl CommittedRule<'_> {
    /// Program-local position of the committed rule.
    pub(crate) const fn rule_position(self) -> RulePosition {
        self.rule_position
    }
}

/// Finds the first currently available rule that matches `state`.
///
/// # Errors
///
/// Returns `RunError` if a parsed `(once)` rule references a missing runtime
/// once-state slot.
pub(crate) fn find_next_match<'program, 'once>(
    rules: &'program [Rule],
    once_states: &'once mut OnceStateSet,
    state: &State,
) -> Result<RuleSearch<'program, 'once>, RunError> {
    for rule in rules {
        match inspect_rule(rule, once_states, state)? {
            RuleInspection::Candidate(candidate) => {
                let commit = once_states.reserve_available_commit(rule)?;
                return Ok(RuleSearch::Matched(candidate.into_application(commit)));
            }
            RuleInspection::Missed(_) => {}
        }
    }

    Ok(RuleSearch::Stable)
}

/// Evaluates exactly one parsed rule line against the current runtime state.
///
/// # Errors
///
/// Returns `RunError` if a parsed `(once)` rule references a missing runtime
/// once-state slot.
pub(crate) fn attempt_rule<'program, 'once>(
    rule: &'program Rule,
    once_states: &'once mut OnceStateSet,
    state: &State,
) -> Result<RuleAttempt<'program, 'once>, RunError> {
    match inspect_rule(rule, once_states, state)? {
        RuleInspection::Candidate(candidate) => {
            let commit = once_states.reserve_available_commit(rule)?;
            Ok(RuleAttempt::Matched(candidate.into_application(commit)))
        }
        RuleInspection::Missed(missed) => Ok(RuleAttempt::Missed(missed)),
    }
}

/// Inspects exactly one parsed rule line without reserving a commit permit.
///
/// # Errors
///
/// Returns `RunError` if a parsed `(once)` rule references a missing runtime
/// once-state slot.
fn inspect_rule<'program>(
    rule: &'program Rule,
    once_states: &OnceStateSet,
    state: &State,
) -> Result<RuleInspection<'program>, RunError> {
    if matches!(
        once_states.availability(rule)?,
        OnceRuleAvailability::Consumed
    ) {
        return Ok(RuleInspection::Missed(RuleAttemptMiss::new(
            rule,
            RuleMissReason::OnceConsumed,
        )));
    }

    let Some(candidate) = matched_candidate_for_rule(rule, state) else {
        return Ok(RuleInspection::Missed(RuleAttemptMiss::new(
            rule,
            RuleMissReason::StateMismatch,
        )));
    };

    Ok(RuleInspection::Candidate(candidate))
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
