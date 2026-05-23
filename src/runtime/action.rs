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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppliedRuleEffect {
    Continue,
    Return,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AppliedRule {
    pub(crate) step: StepCount,
    pub(crate) rule: RulePosition,
    pub(crate) effect: AppliedRuleEffect,
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
pub(crate) fn apply_matched_rule<'program>(
    state: &mut State,
    scratch: &mut RewriteScratch,
    budget: &mut RuntimeBudgetState,
    matched: MatchedRuleApplication<'program, '_>,
) -> Result<AppliedRule, RunError> {
    let permit = budget.reserve_next_step(state.byte_count())?;
    match matched.rule().action() {
        RuleAction::Rewrite(action) => {
            let rewrite = state.rewrite_into(matched.state_match(), action, scratch, *budget)?;
            let committed = matched.commit();
            let step = budget.commit(permit);
            state.commit_rewrite(rewrite, scratch);
            Ok(AppliedRule {
                step,
                rule: committed.position(),
                effect: AppliedRuleEffect::Continue,
            })
        }
        RuleAction::Return(output) => {
            let output_len = ReturnOutputByteCount::from_payload_count(output.byte_count());
            (*budget).ensure_return_len(output_len)?;

            let committed = matched.commit();
            let step = budget.commit(permit);
            Ok(AppliedRule {
                step,
                rule: committed.position(),
                effect: AppliedRuleEffect::Return,
            })
        }
    }
}
