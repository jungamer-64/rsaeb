use crate::bytes::ReturnOutputByteCount;
use crate::error::RunError;
use crate::inspect::RulePosition;
use crate::limits::StepCount;
use crate::program::{ReturnOutput, ReturnOutputView};
use crate::rule::RuleAction;

use super::budget::RuntimeBudgetState;
use super::matcher::MatchedRuleApplication;
use super::rewrite::RewriteScratch;
use super::state::State;

/// Committed rule application reported back to the session layer.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AppliedRule {
    /// One rewrite rule committed and execution may continue.
    Rewrite(CommittedRewriteRule),
    /// One return rule committed and execution is terminal.
    Return(CommittedReturnRule),
}

/// Committed non-terminal rewrite rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommittedRewriteRule {
    /// Step number assigned by the runtime budget.
    step: StepCount,
    /// Program-local position of the committed rewrite rule.
    rule_position: RulePosition,
}

/// Committed terminal return rule.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommittedReturnRule {
    /// Step number assigned by the runtime budget.
    step: StepCount,
    /// Program-local position of the committed return rule.
    rule_position: RulePosition,
    /// Materialized return output produced before committing the terminal step.
    output: ReturnOutput,
}

impl CommittedRewriteRule {
    /// Step number assigned by the runtime budget.
    pub(crate) const fn step(self) -> StepCount {
        self.step
    }
}

impl CommittedRewriteRule {
    /// Program-local position of the committed rewrite rule.
    pub(crate) const fn rule_position(self) -> RulePosition {
        self.rule_position
    }
}

impl CommittedReturnRule {
    /// Step number assigned by the runtime budget.
    pub(crate) const fn step(&self) -> StepCount {
        self.step
    }
}

impl CommittedReturnRule {
    /// Program-local position of the committed return rule.
    pub(crate) const fn rule_position(&self) -> RulePosition {
        self.rule_position
    }

    /// Consumes this committed return rule into its materialized output.
    pub(crate) fn into_output(self) -> ReturnOutput {
        self.output
    }
}

/// Materializes a return payload as public return output.
///
/// # Errors
///
/// Returns `RunError` if return-output allocation fails.
pub(crate) fn materialize_return_output(
    output: ReturnOutputView<'_>,
) -> Result<ReturnOutput, RunError> {
    Ok(output.materialize()?)
}

/// Applies one matched rule and commits its once-rule state on success.
///
/// # Errors
///
/// Returns `RunError` if the next step exceeds limits, the rewrite would
/// exceed state limits, return output exceeds limits, or allocation fails.
pub(crate) fn apply_matched_rule(
    state: &mut State,
    scratch: &mut RewriteScratch,
    budget: &mut RuntimeBudgetState,
    matched: MatchedRuleApplication<'_, '_>,
) -> Result<AppliedRule, RunError> {
    let permit = budget.reserve_next_step(state.byte_count())?;
    match matched.rule().action() {
        RuleAction::Rewrite(action) => {
            let rewrite = state.rewrite_into(matched.state_match(), action, scratch, *budget)?;
            let committed = matched.commit();
            let step = budget.commit(permit);
            state.commit_rewrite(rewrite, scratch);
            Ok(AppliedRule::Rewrite(CommittedRewriteRule {
                step,
                rule_position: committed.rule_position(),
            }))
        }
        RuleAction::Return(output) => {
            let output_view = ReturnOutputView::new(output);
            let output_len = ReturnOutputByteCount::from_payload_count(output.byte_count());
            (*budget).ensure_return_len(output_len)?;
            let materialized_output = materialize_return_output(output_view)?;

            let committed = matched.commit();
            let step = budget.commit(permit);
            Ok(AppliedRule::Return(CommittedReturnRule {
                step,
                rule_position: committed.rule_position(),
                output: materialized_output,
            }))
        }
    }
}
