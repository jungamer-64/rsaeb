use super::once::{MatchedRuleCommit, RuntimeRule, RuntimeRuleReadiness};
use super::state::{State, StateMatch};
use crate::rule::{Rule, RuleAnchorSyntax};

/// Outcome of evaluating one executable rule line against the current state.
#[derive(Debug)]
pub(crate) enum RuleAttempt<'program, 'state, 'once> {
    /// The rule matched and carries the commit permit needed after success.
    Matched(MatchedRuleApplication<'program, 'state, 'once>),
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
pub(crate) struct MatchedRuleApplication<'program, 'state, 'once> {
    /// Parsed rule selected by the matcher.
    rule: &'program Rule,
    /// Once-state side effect to apply only after successful rewrite.
    commit: MatchedRuleCommit<'once>,
    /// Runtime-state range matched by the rule left side.
    state_match: StateMatch<'state>,
}

/// Matched rule after runtime-state match data has been consumed for preparation.
#[derive(Debug)]
pub(crate) struct PreparedMatchedRule<'program, 'once> {
    /// Parsed rule selected by the matcher.
    rule: &'program Rule,
    /// Once-state side effect to apply only after successful rewrite.
    commit: MatchedRuleCommit<'once>,
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
struct MatchedRuleCandidate<'program, 'state> {
    /// Parsed rule selected as a candidate.
    rule: &'program Rule,
    /// Runtime-state range matched by the rule left side.
    state_match: StateMatch<'state>,
}

/// Domain result of comparing one rule's left side with the runtime state.
enum RuleStateMatch<'program, 'state> {
    /// Rule left side matched and carries the matched state span.
    Matched(MatchedRuleCandidate<'program, 'state>),
    /// Rule left side did not match the runtime state.
    Mismatched,
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

impl<'program, 'state> MatchedRuleCandidate<'program, 'state> {
    /// Captures a rule match before once-state commit is permitted.
    const fn new(rule: &'program Rule, state_match: StateMatch<'state>) -> Self {
        Self { rule, state_match }
    }

    /// Attaches the linear commit action to the matched candidate.
    const fn into_application<'once>(
        self,
        commit: MatchedRuleCommit<'once>,
    ) -> MatchedRuleApplication<'program, 'state, 'once> {
        MatchedRuleApplication::new(self.rule, self.state_match, commit)
    }
}

impl<'program, 'state, 'once> MatchedRuleApplication<'program, 'state, 'once> {
    /// Captures the complete data needed to apply a matched rule.
    const fn new(
        rule: &'program Rule,
        state_match: StateMatch<'state>,
        commit: MatchedRuleCommit<'once>,
    ) -> Self {
        Self {
            rule,
            commit,
            state_match,
        }
    }

    /// Splits the state-match witness from the rule commit witness.
    pub(crate) fn into_prepare_parts(
        self,
    ) -> (StateMatch<'state>, PreparedMatchedRule<'program, 'once>) {
        (
            self.state_match,
            PreparedMatchedRule {
                rule: self.rule,
                commit: self.commit,
            },
        )
    }
}

impl<'program> PreparedMatchedRule<'program, '_> {
    /// Parsed rule selected by the matcher.
    pub(crate) const fn rule(&self) -> &'program Rule {
        self.rule
    }

    /// Commits the matched rule's deferred side effects.
    pub(crate) fn commit(self) {
        self.commit.commit();
    }
}

/// Evaluates exactly one parsed rule line against the current runtime state.
pub(crate) fn attempt_rule<'program, 'state, 'once>(
    runtime_rule: RuntimeRule<'program, 'once>,
    state: &'state State,
) -> RuleAttempt<'program, 'state, 'once> {
    let rule = runtime_rule.rule();
    let commit_seed = match runtime_rule.readiness() {
        RuntimeRuleReadiness::Available(commit_seed) => commit_seed,
        RuntimeRuleReadiness::Consumed => {
            return RuleAttempt::Missed(RuleAttemptMiss::new(rule, RuleMissReason::OnceConsumed));
        }
    };

    let candidate = match match_rule_state(rule, state) {
        RuleStateMatch::Matched(candidate) => candidate,
        RuleStateMatch::Mismatched => {
            return RuleAttempt::Missed(RuleAttemptMiss::new(rule, RuleMissReason::StateMismatch));
        }
    };
    let commit = commit_seed.into_matched_commit();

    RuleAttempt::Matched(candidate.into_application(commit))
}

/// Compares a single parsed rule with the current runtime state.
fn match_rule_state<'program, 'state>(
    rule: &'program Rule,
    state: &'state State,
) -> RuleStateMatch<'program, 'state> {
    match find_match(state, rule) {
        Some(state_match) => RuleStateMatch::Matched(MatchedRuleCandidate::new(rule, state_match)),
        None => RuleStateMatch::Mismatched,
    }
}

/// Finds this rule's match span in the current state.
fn find_match<'state>(state: &'state State, rule: &Rule) -> Option<StateMatch<'state>> {
    match rule.anchor() {
        RuleAnchorSyntax::Anywhere => state.find_payload(rule.lhs()),
        RuleAnchorSyntax::Start => state.starts_with_payload(rule.lhs()),
        RuleAnchorSyntax::End => state.ends_with_payload(rule.lhs()),
    }
}
