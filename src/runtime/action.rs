use crate::bytes::ReturnOutputByteCount;
use crate::error::RunStepError;
use crate::inspect::{
    AlwaysReturnRuleView, AlwaysRewriteRuleView, OnceReturnRuleView, OnceRewriteRuleView, RuleView,
};
use crate::limits::StepCount;
use crate::policy::ExecutionPolicy;
use crate::program::limits::ReturnOutputBytePermit;
use crate::program::{ReturnOutput, ReturnOutputView};

use super::budget::{RuntimeBudgetState, StepReservation};
use super::matcher::MatchedRuleApplication;
use super::once::OnceMatchPermit;
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
    /// Prepared reusable non-terminal rewrite.
    AlwaysRewrite(PreparedAlwaysRewriteRule<'program, 'budget, E>),
    /// Prepared once-only non-terminal rewrite.
    OnceRewrite(PreparedOnceRewriteRule<'program, 'once, 'budget, E>),
    /// Prepared reusable terminal return.
    AlwaysReturn(PreparedAlwaysReturnRule<'program, 'budget, E>),
    /// Prepared once-only terminal return.
    OnceReturn(PreparedOnceReturnRule<'program, 'once, 'budget, E>),
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

/// Prepared reusable rewrite before its step side effects commit.
#[derive(Debug)]
pub(crate) struct PreparedAlwaysRewriteRule<'program, 'budget, E: ExecutionPolicy> {
    /// Matched reusable rewrite rule.
    rule: AlwaysRewriteRuleView<'program>,
    /// Step reservation required before this rewrite can commit.
    step: StepReservation<'budget, E>,
    /// Runtime bytes ready to become the next state.
    rewrite: PreparedRewrite,
}

/// Prepared once-only rewrite before its step and once-state side effects commit.
#[derive(Debug)]
pub(crate) struct PreparedOnceRewriteRule<'program, 'once, 'budget, E: ExecutionPolicy> {
    /// Matched once-only rewrite rule.
    rule: OnceRewriteRuleView<'program>,
    /// Once-state commit permit owned only by this matched once rule.
    once_commit: OnceMatchPermit<'once>,
    /// Step reservation required before this rewrite can commit.
    step: StepReservation<'budget, E>,
    /// Runtime bytes ready to become the next state.
    rewrite: PreparedRewrite,
}

/// Prepared reusable return before its step side effects commit.
#[derive(Debug)]
pub(crate) struct PreparedAlwaysReturnRule<'program, 'budget, E: ExecutionPolicy> {
    /// Matched reusable return rule.
    rule: AlwaysReturnRuleView<'program>,
    /// Step reservation required before this return can commit.
    step: StepReservation<'budget, E>,
    /// Borrowed return output payload from the matched parsed rule.
    output_view: ReturnOutputView<'program>,
    /// Materialized return output.
    output: ReturnOutput,
}

/// Prepared once-only return before its step and once-state side effects commit.
#[derive(Debug)]
pub(crate) struct PreparedOnceReturnRule<'program, 'once, 'budget, E: ExecutionPolicy> {
    /// Matched once-only return rule.
    rule: OnceReturnRuleView<'program>,
    /// Once-state commit permit owned only by this matched once rule.
    once_commit: OnceMatchPermit<'once>,
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
    pub(crate) const fn rule(&self) -> RuleView<'program> {
        match self {
            Self::AlwaysRewrite(prepared) => RuleView::AlwaysRewrite(prepared.rule),
            Self::OnceRewrite(prepared) => RuleView::OnceRewrite(prepared.rule),
            Self::AlwaysReturn(prepared) => RuleView::AlwaysReturn(prepared.rule),
            Self::OnceReturn(prepared) => RuleView::OnceReturn(prepared.rule),
        }
    }

    /// Commits the prepared runtime side effects.
    pub(crate) fn commit(
        self,
        state: &mut State,
        scratch: &mut RewriteScratch,
    ) -> AppliedRule<'program> {
        match self {
            Self::AlwaysRewrite(prepared) => {
                let step = prepared.step.commit();
                state.commit_rewrite(prepared.rewrite, scratch);
                AppliedRule::Continued(CommittedRewriteRule { step })
            }
            Self::OnceRewrite(prepared) => {
                prepared.once_commit.commit();
                let step = prepared.step.commit();
                state.commit_rewrite(prepared.rewrite, scratch);
                AppliedRule::Continued(CommittedRewriteRule { step })
            }
            Self::AlwaysReturn(prepared) => {
                let step = prepared.step.commit();
                AppliedRule::Terminal(CommittedReturnRule {
                    step,
                    output_view: prepared.output_view,
                    output: prepared.output,
                })
            }
            Self::OnceReturn(prepared) => {
                prepared.once_commit.commit();
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
    match matched {
        MatchedRuleApplication::AlwaysRewrite(matched) => {
            let (rule, state_match, action) = matched.into_parts();
            let step = budget.reserve_next_step(state_len)?;
            let rewrite = state_match.rewrite_into(action, scratch, &step)?;
            Ok(PreparedRuleStep::AlwaysRewrite(PreparedAlwaysRewriteRule {
                rule,
                step,
                rewrite,
            }))
        }
        MatchedRuleApplication::OnceRewrite(matched) => {
            let (rule, state_match, action, once_commit) = matched.into_parts();
            let step = budget.reserve_next_step(state_len)?;
            let rewrite = state_match.rewrite_into(action, scratch, &step)?;
            Ok(PreparedRuleStep::OnceRewrite(PreparedOnceRewriteRule {
                rule,
                once_commit,
                step,
                rewrite,
            }))
        }
        MatchedRuleApplication::AlwaysReturn(matched) => {
            let (rule, _state_match, output) = matched.into_parts();
            let step = budget.reserve_next_step(state_len)?;
            let output_view = ReturnOutputView::new(output);
            let output_len = ReturnOutputByteCount::from_payload_count(output.byte_count());
            let output_permit = RuntimeBudgetState::<E>::ensure_return_len(output_len)?;
            let materialized_output = materialize_return_output(output_view, output_permit)?;

            Ok(PreparedRuleStep::AlwaysReturn(PreparedAlwaysReturnRule {
                rule,
                step,
                output_view,
                output: materialized_output,
            }))
        }
        MatchedRuleApplication::OnceReturn(matched) => {
            let (rule, _state_match, output, once_commit) = matched.into_parts();
            let step = budget.reserve_next_step(state_len)?;
            let output_view = ReturnOutputView::new(output);
            let output_len = ReturnOutputByteCount::from_payload_count(output.byte_count());
            let output_permit = RuntimeBudgetState::<E>::ensure_return_len(output_len)?;
            let materialized_output = materialize_return_output(output_view, output_permit)?;

            Ok(PreparedRuleStep::OnceReturn(PreparedOnceReturnRule {
                rule,
                once_commit,
                step,
                output_view,
                output: materialized_output,
            }))
        }
    }
}
