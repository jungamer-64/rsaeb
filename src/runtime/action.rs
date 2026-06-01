use crate::bytes::ReturnOutputByteCount;
use crate::error::RunStepError;
use crate::limits::StepCount;
use crate::policy::ExecutionPolicy;
use crate::program::{ReturnOutput, ReturnOutputView};
use crate::rule::{ParsedRuleAction, Rule};

use super::budget::{RuntimeBudgetState, StepReservation};
use super::matcher::{MatchedRuleApplication, PreparedMatchedRule};
use super::rewrite::{PreparedRewrite, RewriteScratch};
use super::state::State;

/// Committed rule application reported back to the session layer.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AppliedRule<'program> {
    /// One rewrite rule committed and execution may continue.
    Rewrite(CommittedRewriteRule<'program>),
    /// One return rule committed and execution is terminal.
    Return(CommittedReturnRule<'program>),
}

/// Rule application after all failure-prone runtime preparation has succeeded.
#[derive(Debug)]
pub(crate) enum PreparedRuleApplication<'program, 'once, 'budget, E: ExecutionPolicy> {
    /// Prepared non-terminal rewrite.
    Rewrite(PreparedRewriteRule<'program, 'once, 'budget, E>),
    /// Prepared terminal return.
    Return(PreparedReturnRule<'program, 'once, 'budget, E>),
}

/// Committed non-terminal rewrite rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommittedRewriteRule<'program> {
    /// Step number assigned by the runtime budget.
    step: StepCount,
    /// Parsed rule whose rewrite committed.
    rule: &'program Rule,
}

/// Committed terminal return rule.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommittedReturnRule<'program> {
    /// Step number assigned by the runtime budget.
    step: StepCount,
    /// Parsed rule whose return committed.
    rule: &'program Rule,
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

impl CommittedRewriteRule<'_> {
    /// Step number assigned by the runtime budget.
    pub(crate) const fn step(self) -> StepCount {
        self.step
    }
}

impl<'program> CommittedRewriteRule<'program> {
    /// Parsed rule whose rewrite committed.
    pub(crate) const fn rule(self) -> &'program Rule {
        self.rule
    }
}

impl CommittedReturnRule<'_> {
    /// Step number assigned by the runtime budget.
    pub(crate) const fn step(&self) -> StepCount {
        self.step
    }
}

impl<'program> CommittedReturnRule<'program> {
    /// Parsed rule whose return committed.
    pub(crate) const fn rule(&self) -> &'program Rule {
        self.rule
    }

    /// Borrowed return output payload from the committed parsed rule.
    pub(crate) const fn output_view(&self) -> ReturnOutputView<'program> {
        self.output_view
    }

    /// Consumes this committed return rule into its materialized output.
    pub(crate) fn into_output(self) -> ReturnOutput {
        self.output
    }
}

impl<'program, E: ExecutionPolicy> PreparedRuleApplication<'program, '_, '_, E> {
    /// Parsed rule selected by this prepared application.
    pub(crate) const fn rule(&self) -> &'program Rule {
        match self {
            Self::Rewrite(prepared) => prepared.matched.rule(),
            Self::Return(prepared) => prepared.matched.rule(),
        }
    }

    /// Commits the prepared runtime side effects.
    ///
    pub(crate) fn commit(
        self,
        state: &mut State,
        scratch: &mut RewriteScratch,
    ) -> AppliedRule<'program> {
        match self {
            Self::Rewrite(prepared) => {
                let committed = prepared.matched.commit();
                let step = prepared.step.commit();
                state.commit_rewrite(prepared.rewrite, scratch);
                AppliedRule::Rewrite(CommittedRewriteRule {
                    step,
                    rule: committed.rule(),
                })
            }
            Self::Return(prepared) => {
                let committed = prepared.matched.commit();
                let step = prepared.step.commit();
                AppliedRule::Return(CommittedReturnRule {
                    step,
                    rule: committed.rule(),
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
) -> Result<ReturnOutput, RunStepError> {
    Ok(ReturnOutput::from_return_output_view(output)?)
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
) -> Result<PreparedRuleApplication<'program, 'once, 'budget, E>, RunStepError> {
    let (state_match, matched) = matched.into_prepare_parts();
    let step = budget.reserve_next_step(state_len)?;
    match matched.rule().action() {
        ParsedRuleAction::Rewrite(action) => {
            let rewrite = state_match.rewrite_into(action, scratch, &step)?;
            Ok(PreparedRuleApplication::Rewrite(PreparedRewriteRule {
                matched,
                step,
                rewrite,
            }))
        }
        ParsedRuleAction::Return(output) => {
            let output_view = ReturnOutputView::new(output);
            let output_len = ReturnOutputByteCount::from_payload_count(output.byte_count());
            RuntimeBudgetState::<E>::ensure_return_len(output_len)?;
            let materialized_output = materialize_return_output(output_view)?;

            Ok(PreparedRuleApplication::Return(PreparedReturnRule {
                matched,
                step,
                output_view,
                output: materialized_output,
            }))
        }
    }
}
