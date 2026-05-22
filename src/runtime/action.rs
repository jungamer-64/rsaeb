use crate::bytes::ReturnOutputByteCount;
use crate::error::RunError;
use crate::inspect::RuleView;
use crate::program::{ReturnOutput, ReturnOutputView, StepCount};
use crate::rule::Action;

use super::budget::RuntimeBudgetState;
use super::matcher::MatchedRule;
use super::rewrite::{PreparedRewrite, RewriteRequest, RewriteScratch};
use super::state::{MatchedStateSpan, State};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppliedRuleEffect<'program> {
    Continue,
    Return(ReturnOutputView<'program>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AppliedRule<'program> {
    pub(crate) step: StepCount,
    pub(crate) rule: RuleView<'program>,
    pub(crate) effect: AppliedRuleEffect<'program>,
}

#[derive(Debug, PartialEq, Eq)]
enum PreparedAction<'program> {
    Rewrite(PreparedRewrite),
    Return(ReturnOutputView<'program>),
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
    matched: MatchedRule<'program, '_>,
) -> Result<AppliedRule<'program>, RunError> {
    let permit = budget.reserve_next_step(state.byte_count())?;
    let prepared = prepare_action(
        state,
        scratch,
        *budget,
        matched.state_match,
        matched.rule.action(),
    )?;
    matched.commit.commit();

    let step = budget.commit(permit);

    match prepared {
        PreparedAction::Rewrite(rewrite) => {
            state.commit_rewrite(rewrite, scratch);
            Ok(AppliedRule {
                step,
                rule: RuleView::new(matched.position, matched.rule),
                effect: AppliedRuleEffect::Continue,
            })
        }
        PreparedAction::Return(output) => Ok(AppliedRule {
            step,
            rule: RuleView::new(matched.position, matched.rule),
            effect: AppliedRuleEffect::Return(output),
        }),
    }
}

/// Applies a rule action into scratch storage without committing the state.
///
/// # Errors
///
/// Returns `RunError` if rewrite state or return output exceeds limits, or if
/// scratch/output allocation fails.
fn prepare_action<'program>(
    state: &State,
    scratch: &mut RewriteScratch,
    budget: RuntimeBudgetState,
    state_match: MatchedStateSpan,
    action: &'program Action,
) -> Result<PreparedAction<'program>, RunError> {
    match action {
        Action::Replace(rhs) => {
            let rewrite =
                state.rewrite_into(RewriteRequest::replace(state_match, rhs), scratch, budget)?;
            Ok(PreparedAction::Rewrite(rewrite))
        }
        Action::MoveStart(rhs) => {
            let rewrite = state.rewrite_into(
                RewriteRequest::move_start(state_match, rhs),
                scratch,
                budget,
            )?;
            Ok(PreparedAction::Rewrite(rewrite))
        }
        Action::MoveEnd(rhs) => {
            let rewrite =
                state.rewrite_into(RewriteRequest::move_end(state_match, rhs), scratch, budget)?;
            Ok(PreparedAction::Rewrite(rewrite))
        }
        Action::Return(output) => {
            let output_len = ReturnOutputByteCount::from_payload_count(output.byte_count());
            budget.ensure_return_len(output_len)?;

            Ok(PreparedAction::Return(ReturnOutputView::new(output)))
        }
    }
}
