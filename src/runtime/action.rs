use crate::allocation::AllocationContext;
use crate::bytes::ReturnOutputByteCount;
use crate::error::{LimitError, RunError};
use crate::program::{ReturnOutput, StepCount};
use crate::rule::{Action, PayloadView, Rule};

use super::execution::Execution;
use super::matcher::MatchedRule;
use super::rewrite::{RewritePlacement, RewriteRequest};
use super::state::MatchedStateSpan;
use super::terminal::ExecutionTerminal;

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

impl<'program> Execution<'program> {
    pub(super) fn apply_matched_rule(
        &mut self,
        matched: MatchedRule<'program>,
    ) -> Result<AppliedRule<'program>, RunError> {
        let permit = self
            .step_budget
            .reserve_next_step(self.state.byte_count())
            .map_err(RunError::from)?;

        let effect = self.apply_action_to_scratch(matched.state_match, matched.rule.action())?;
        self.once_states
            .consume(matched.schedule)
            .map_err(RunError::from)?;

        let step = self.step_budget.commit(permit);

        match effect {
            StepApplication::Continue => {
                self.state.swap_with_scratch(&mut self.scratch);
                Ok(AppliedRule {
                    step,
                    rule: matched.rule,
                    effect: StepApplication::Continue,
                })
            }
            StepApplication::Return(output) => {
                self.terminal = ExecutionTerminal::Return {
                    step,
                    rule: matched.rule,
                    output,
                };
                Ok(AppliedRule {
                    step,
                    rule: matched.rule,
                    effect: StepApplication::Return(output),
                })
            }
        }
    }

    pub(super) fn materialize_return_output(
        output: PayloadView<'program>,
    ) -> Result<ReturnOutput, RunError> {
        Ok(ReturnOutput::from_vec(
            output.to_vec_with_context(AllocationContext::ReturnOutput)?,
        ))
    }

    fn apply_action_to_scratch(
        &mut self,
        state_match: MatchedStateSpan,
        action: &'program Action,
    ) -> Result<StepApplication<'program>, RunError> {
        match action {
            Action::Replace(rhs) => {
                self.state.rewrite_into(
                    RewriteRequest::new(state_match, rhs, RewritePlacement::Replace),
                    &mut self.scratch,
                    self.limits,
                )?;
                Ok(StepApplication::Continue)
            }
            Action::MoveStart(rhs) => {
                self.state.rewrite_into(
                    RewriteRequest::new(state_match, rhs, RewritePlacement::MoveStart),
                    &mut self.scratch,
                    self.limits,
                )?;
                Ok(StepApplication::Continue)
            }
            Action::MoveEnd(rhs) => {
                self.state.rewrite_into(
                    RewriteRequest::new(state_match, rhs, RewritePlacement::MoveEnd),
                    &mut self.scratch,
                    self.limits,
                )?;
                Ok(StepApplication::Continue)
            }
            Action::Return(output) => {
                let output_len = ReturnOutputByteCount::new(output.len());
                if output_len.get() > self.limits.return_byte_limit().get() {
                    return Err(LimitError::return_output(
                        self.limits.return_byte_limit(),
                        output_len,
                    )
                    .into());
                }

                Ok(StepApplication::Return(PayloadView::new(output)))
            }
        }
    }
}
