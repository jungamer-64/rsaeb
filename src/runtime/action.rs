use crate::allocation::AllocationContext;
use crate::bytes::ReturnOutputByteCount;
use crate::error::{LimitError, RunError};
use crate::inspect::PayloadView;
use crate::program::{ReturnOutput, RunLimits, StepCount};
use crate::rule::{Action, Rule};

use super::budget::StepBudget;
use super::matcher::MatchedRule;
use super::once::OnceStateSet;
use super::rewrite::{RewritePlacement, RewriteRequest, RewriteScratch};
use super::state::{MatchedStateSpan, State};
use crate::execution::ExecutionCore;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepApplication<'program> {
    Continue,
    Return(PayloadView<'program>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AppliedRule<'program> {
    pub(crate) step: StepCount,
    pub(crate) rule: &'program Rule,
    pub(crate) effect: StepApplication<'program>,
}

impl ExecutionCore<'_> {
    /// Materializes a return payload as public return output.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if return-output allocation fails.
    pub(crate) fn materialize_return_output(
        output: PayloadView<'_>,
    ) -> Result<ReturnOutput, RunError> {
        Ok(ReturnOutput::from_vec(
            output.to_vec_with_context(AllocationContext::ReturnOutput)?,
        ))
    }
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
    step_budget: &mut StepBudget,
    once_states: &mut OnceStateSet,
    limits: RunLimits,
    matched: MatchedRule<'program>,
) -> Result<AppliedRule<'program>, RunError> {
    let permit = step_budget.reserve_next_step(state.byte_count())?;
    let effect = apply_action_to_scratch(
        state,
        scratch,
        limits,
        matched.state_match,
        matched.rule.action(),
    )?;
    once_states.commit(matched.commit);

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

/// Applies a rule action into scratch storage without committing the state.
///
/// # Errors
///
/// Returns `RunError` if rewrite state or return output exceeds limits, or if
/// scratch/output allocation fails.
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
