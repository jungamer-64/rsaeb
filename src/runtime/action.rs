use crate::allocation::AllocationContext;
use crate::bytes::ReturnOutputByteCount;
use crate::error::{LimitError, RunError};
use crate::program::{ReturnOutput, RunLimits, StepCount};
use crate::rule::{Action, PayloadView, Rule};

use super::budget::StepBudget;
use super::execution::ExecutionCore;
use super::matcher::MatchedRule;
use super::rewrite::{RewritePlacement, RewriteRequest, RewriteScratch};
use super::state::{MatchedStateSpan, State};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StepApplication<'program> {
    Continue,
    Return(PayloadView<'program>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AppliedRule<'program> {
    pub(super) step: StepCount,
    pub(super) rule: &'program Rule,
    pub(super) effect: StepApplication<'program>,
}

impl ExecutionCore<'_> {
    pub(super) fn materialize_return_output(
        output: PayloadView<'_>,
    ) -> Result<ReturnOutput, RunError> {
        Ok(ReturnOutput::from_vec(
            output.to_vec_with_context(AllocationContext::ReturnOutput)?,
        ))
    }
}

pub(super) fn apply_matched_rule<'program>(
    state: &mut State,
    scratch: &mut RewriteScratch,
    step_budget: &mut StepBudget,
    limits: RunLimits,
    matched: MatchedRule<'program, '_>,
) -> Result<AppliedRule<'program>, RunError> {
    let permit = step_budget.reserve_next_step(state.byte_count())?;
    let effect = apply_action_to_scratch(
        state,
        scratch,
        limits,
        matched.state_match,
        matched.rule.action(),
    )?;
    matched.commit.commit();

    let step = step_budget.commit(permit);

    match effect {
        StepApplication::Continue => {
            state.swap_with_scratch(scratch);
            Ok(AppliedRule {
                step,
                rule: matched.rule,
                effect: StepApplication::Continue,
            })
        }
        StepApplication::Return(output) => Ok(AppliedRule {
            step,
            rule: matched.rule,
            effect: StepApplication::Return(output),
        }),
    }
}

fn apply_action_to_scratch<'program>(
    state: &State,
    scratch: &mut RewriteScratch,
    limits: RunLimits,
    state_match: MatchedStateSpan,
    action: &'program Action,
) -> Result<StepApplication<'program>, RunError> {
    match action {
        Action::Replace(rhs) => {
            state.rewrite_into(
                RewriteRequest::new(state_match, rhs, RewritePlacement::Replace),
                scratch,
                limits,
            )?;
            Ok(StepApplication::Continue)
        }
        Action::MoveStart(rhs) => {
            state.rewrite_into(
                RewriteRequest::new(state_match, rhs, RewritePlacement::MoveStart),
                scratch,
                limits,
            )?;
            Ok(StepApplication::Continue)
        }
        Action::MoveEnd(rhs) => {
            state.rewrite_into(
                RewriteRequest::new(state_match, rhs, RewritePlacement::MoveEnd),
                scratch,
                limits,
            )?;
            Ok(StepApplication::Continue)
        }
        Action::Return(output) => {
            let output_len = ReturnOutputByteCount::new(output.len());
            if output_len.get() > limits.return_byte_limit().get() {
                return Err(
                    LimitError::return_output(limits.return_byte_limit(), output_len).into(),
                );
            }

            Ok(StepApplication::Return(PayloadView::new(output)))
        }
    }
}
