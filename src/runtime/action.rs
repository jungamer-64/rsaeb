use crate::bytes::ReturnOutputByteCount;
use crate::error::RunStepError;
use crate::limits::StepCount;
use crate::policy::ExecutionPolicy;
use crate::program::limits::ReturnOutputBytePermit;
use crate::program::{ReturnOutput, ReturnOutputView};
use crate::rule::Rule;

use super::budget::{RuntimeBudgetState, StepReservation};
use super::matcher::{MatchedRuleAction, MatchedRuleApplication, PreparedMatchedRule};
use super::rewrite::{PreparedRewrite, RewriteScratch};
use super::state::State;

/// Committed rule application reported back to the session layer.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AppliedRule<'program> {
    /// One rewrite rule committed and execution may continue.
    Continued(CommittedRewriteRule),
    /// One return rule committed and execution is terminal.
    Terminal(CommittedReturnRule<'program>),
}

/// Prepared rule step after action-specific runtime preparation succeeds.
#[derive(Debug)]
pub(crate) enum PreparedRuleStep<'program, 'once, 'budget, E: ExecutionPolicy> {
    /// Prepared non-terminal rewrite.
    Rewrite(PreparedRewriteRule<'program, 'once, 'budget, E>),
    /// Prepared terminal return.
    Return(PreparedReturnRule<'program, 'once, 'budget, E>),
}

/// Committed non-terminal rewrite rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommittedRewriteRule {
    /// Step number assigned by the runtime budget.
    step: StepCount,
}

/// Committed terminal return rule.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommittedReturnRule<'program> {
    /// Step number assigned by the runtime budget.
    step: StepCount,
    /// Borrowed return output payload from the committed parsed rule.
    output_view: ReturnOutputView<'program>,
    /// Materialized return output produced before committing the terminal step.
    output: ReturnOutput,
}

/// Prepared non-terminal rewrite before its step and once-state side effects commit.
#[derive(Debug)]
pub(crate) struct PreparedRewriteRule<'program, 'once, 'budget, E: ExecutionPolicy> {
    /// Matched rule and once-state commit permit.
    matched: PreparedMatchedRule<'program, 'once>,
    /// Step reservation required before this rewrite can commit.
    step: StepReservation<'budget, E>,
    /// Runtime bytes ready to become the next state.
    rewrite: PreparedRewrite,
}

/// Prepared terminal return before its step and once-state side effects commit.
#[derive(Debug)]
pub(crate) struct PreparedReturnRule<'program, 'once, 'budget, E: ExecutionPolicy> {
    /// Matched rule and once-state commit permit.
    matched: PreparedMatchedRule<'program, 'once>,
    /// Step reservation required before this return can commit.
    step: StepReservation<'budget, E>,
    /// Borrowed return output payload from the matched parsed rule.
    output_view: ReturnOutputView<'program>,
    /// Materialized return output.
    output: ReturnOutput,
}

impl CommittedRewriteRule {
    /// Step number assigned by the runtime budget.
    pub(crate) const fn step(self) -> StepCount {
        self.step
    }
}

impl CommittedReturnRule<'_> {
    /// Step number assigned by the runtime budget.
    pub(crate) const fn step(&self) -> StepCount {
        self.step
    }
}

impl<'program> CommittedReturnRule<'program> {
    /// Borrowed return output payload from the committed parsed rule.
    pub(crate) const fn output_view(&self) -> ReturnOutputView<'program> {
        self.output_view
    }

    /// Consumes this committed return rule into its materialized output.
    pub(crate) fn into_output(self) -> ReturnOutput {
        self.output
    }
}

impl<'program, E: ExecutionPolicy> PreparedRuleStep<'program, '_, '_, E> {
    /// Parsed rule selected by this prepared step.
    pub(crate) const fn rule(&self) -> &'program Rule {
        match self {
            Self::Rewrite(prepared) => prepared.matched.rule(),
            Self::Return(prepared) => prepared.matched.rule(),
        }
    }

    /// Commits the prepared runtime side effects.
    pub(crate) fn commit(
        self,
        state: &mut State,
        scratch: &mut RewriteScratch,
    ) -> AppliedRule<'program> {
        match self {
            Self::Rewrite(prepared) => {
                prepared.matched.commit();
                let step = prepared.step.commit();
                state.commit_rewrite(prepared.rewrite, scratch);
                AppliedRule::Continued(CommittedRewriteRule { step })
            }
            Self::Return(prepared) => {
                prepared.matched.commit();
                let step = prepared.step.commit();
                AppliedRule::Terminal(CommittedReturnRule {
                    step,
                    output_view: prepared.output_view,
                    output: prepared.output,
                })
            }
        }
    }
}

/// Materializes a return payload as public return output.
///
/// # Errors
///
/// Returns `RunStepError` if return-output allocation fails.
pub(crate) fn materialize_return_output(
    output: ReturnOutputView<'_>,
    permit: ReturnOutputBytePermit,
) -> Result<ReturnOutput, RunStepError> {
    Ok(ReturnOutput::from_permitted_return_output_view(
        output, permit,
    )?)
}

/// Prepares one matched rule without committing state, completed-step count, or once-rule side effects.
///
/// # Errors
///
/// Returns `RunStepError` if the next step cannot be reserved, the rewrite would
/// exceed state limits, return output exceeds limits, or allocation fails.
pub(crate) fn prepare_matched_rule<'program, 'once, 'budget, E: ExecutionPolicy>(
    scratch: &mut RewriteScratch,
    budget: &'budget mut RuntimeBudgetState<E>,
    state_len: crate::bytes::RuntimeStateByteCount,
    matched: MatchedRuleApplication<'program, '_, 'once>,
) -> Result<PreparedRuleStep<'program, 'once, 'budget, E>, RunStepError> {
    let (state_match, matched) = matched.into_prepare_parts();
    let step = budget.reserve_next_step(state_len)?;
    match matched {
        MatchedRuleAction::Rewrite { matched, action } => {
            let rewrite = state_match.rewrite_into(action, scratch, &step)?;
            Ok(PreparedRuleStep::Rewrite(PreparedRewriteRule {
                matched,
                step,
                rewrite,
            }))
        }
        MatchedRuleAction::Return { matched, output } => {
            let output_view = ReturnOutputView::new(output);
            let output_len = ReturnOutputByteCount::from_payload_count(output.byte_count());
            let output_permit = RuntimeBudgetState::<E>::ensure_return_len(output_len)?;
            let materialized_output = materialize_return_output(output_view, output_permit)?;

            Ok(PreparedRuleStep::Return(PreparedReturnRule {
                matched,
                step,
                output_view,
                output: materialized_output,
            }))
        }
    }
}
