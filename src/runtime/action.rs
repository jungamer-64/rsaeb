use crate::allocation::AllocationContext;
use crate::bytes::ReturnOutputByteCount;
use crate::error::RunError;
use crate::inspect::{PayloadView, RuleView};
use crate::program::{ReturnOutput, StepCount};
use crate::rule::Action;

use super::budget::RuntimeBudgetState;
use super::matcher::MatchedRule;
use super::once::OnceStateSet;
use super::rewrite::{RewritePlacement, RewriteRequest, RewriteScratch};
use super::state::{MatchedStateSpan, State};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppliedRuleEffect<'program> {
    Continue,
    Return(PayloadView<'program>),
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
    Return(PayloadView<'program>),
}

#[derive(Debug, PartialEq, Eq)]
struct PreparedRewrite {
    ready: (),
}

impl PreparedRewrite {
    const fn new() -> Self {
        Self { ready: () }
    }

    fn commit(self, state: &mut State, scratch: &mut RewriteScratch) {
        let Self { ready: () } = self;
        state.swap_with_scratch(scratch);
    }
}

/// Materializes a return payload as public return output.
///
/// # Errors
///
/// Returns `RunError` if return-output allocation fails.
pub(crate) fn materialize_return_output(output: PayloadView<'_>) -> Result<ReturnOutput, RunError> {
    Ok(ReturnOutput::from_vec(
        output.to_vec_with_context(AllocationContext::ReturnOutput)?,
    ))
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
    once_states: &mut OnceStateSet,
    matched: MatchedRule<'program>,
) -> Result<AppliedRule<'program>, RunError> {
    let permit = budget.reserve_next_step(state.byte_count())?;
    let prepared = prepare_action(
        state,
        scratch,
        *budget,
        matched.state_match,
        matched.rule.action(),
    )?;
    once_states.commit(matched.commit);

    let step = budget.commit(permit);

    match prepared {
        PreparedAction::Rewrite(rewrite) => {
            rewrite.commit(state, scratch);
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
            state.rewrite_into(
                RewriteRequest::new(state_match, rhs, RewritePlacement::Replace),
                scratch,
                budget,
            )?;
            Ok(PreparedAction::Rewrite(PreparedRewrite::new()))
        }
        Action::MoveStart(rhs) => {
            state.rewrite_into(
                RewriteRequest::new(state_match, rhs, RewritePlacement::MoveStart),
                scratch,
                budget,
            )?;
            Ok(PreparedAction::Rewrite(PreparedRewrite::new()))
        }
        Action::MoveEnd(rhs) => {
            state.rewrite_into(
                RewriteRequest::new(state_match, rhs, RewritePlacement::MoveEnd),
                scratch,
                budget,
            )?;
            Ok(PreparedAction::Rewrite(PreparedRewrite::new()))
        }
        Action::Return(output) => {
            let output_len = ReturnOutputByteCount::new(output.len());
            budget.ensure_return_len(output_len)?;

            Ok(PreparedAction::Return(PayloadView::new(output)))
        }
    }
}
