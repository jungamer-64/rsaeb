use crate::bytes::ReturnOutputByteCount;
use crate::error::RunError;
use crate::limits::StepCount;
use crate::program::{ReturnOutput, ReturnOutputView};
use crate::rule::{Rule, RuleAction};

use super::budget::RuntimeBudgetState;
use super::budget::StepPermit;
use super::matcher::MatchedRuleApplication;
use super::once::OnceStateSet;
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
pub(crate) enum PreparedRuleApplication<'program> {
    /// Prepared non-terminal rewrite.
    Rewrite(PreparedRewriteRule<'program>),
    /// Prepared terminal return.
    Return(PreparedReturnRule<'program>),
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
pub(crate) struct PreparedRewriteRule<'program> {
    /// Reserved step number.
    permit: StepPermit,
    /// Matched rule and once-state commit permit.
    matched: MatchedRuleApplication<'program>,
    /// Runtime bytes ready to become the next state.
    rewrite: PreparedRewrite,
}

/// Prepared terminal return before its step and once-state side effects commit.
#[derive(Debug)]
pub(crate) struct PreparedReturnRule<'program> {
    /// Reserved step number.
    permit: StepPermit,
    /// Matched rule and once-state commit permit.
    matched: MatchedRuleApplication<'program>,
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

impl<'program> PreparedRuleApplication<'program> {
    /// Parsed rule selected by this prepared application.
    pub(crate) const fn rule(&self) -> &'program Rule {
        match self {
            Self::Rewrite(prepared) => prepared.matched.rule(),
            Self::Return(prepared) => prepared.matched.rule(),
        }
    }

    /// Commits the prepared runtime side effects.
    ///
    /// # Errors
    ///
    /// Returns `RunError` if a prepared once-rule commit permit no longer
    /// points at a valid runtime once-state slot.
    pub(crate) fn commit(
        self,
        state: &mut State,
        scratch: &mut RewriteScratch,
        budget: &mut RuntimeBudgetState,
        once_states: &mut OnceStateSet,
    ) -> Result<AppliedRule<'program>, RunError> {
        match self {
            Self::Rewrite(prepared) => {
                let committed = prepared.matched.commit(once_states)?;
                let step = budget.commit(prepared.permit);
                state.commit_rewrite(prepared.rewrite, scratch);
                Ok(AppliedRule::Rewrite(CommittedRewriteRule {
                    step,
                    rule: committed.rule(),
                }))
            }
            Self::Return(prepared) => {
                let committed = prepared.matched.commit(once_states)?;
                let step = budget.commit(prepared.permit);
                Ok(AppliedRule::Return(CommittedReturnRule {
                    step,
                    rule: committed.rule(),
                    output_view: prepared.output_view,
                    output: prepared.output,
                }))
            }
        }
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
pub(crate) fn apply_matched_rule<'program>(
    state: &mut State,
    scratch: &mut RewriteScratch,
    budget: &mut RuntimeBudgetState,
    once_states: &mut OnceStateSet,
    matched: MatchedRuleApplication<'program>,
) -> Result<AppliedRule<'program>, RunError> {
    let prepared = prepare_matched_rule(state, scratch, *budget, matched)?;
    prepared.commit(state, scratch, budget, once_states)
}

/// Prepares one matched rule without committing state, budget, or once-rule side effects.
///
/// # Errors
///
/// Returns `RunError` if the next step exceeds limits, the rewrite would
/// exceed state limits, return output exceeds limits, or allocation fails.
pub(crate) fn prepare_matched_rule<'program>(
    state: &State,
    scratch: &mut RewriteScratch,
    budget: RuntimeBudgetState,
    matched: MatchedRuleApplication<'program>,
) -> Result<PreparedRuleApplication<'program>, RunError> {
    let permit = budget.reserve_next_step(state.byte_count())?;
    match matched.rule().action() {
        RuleAction::Rewrite(action) => {
            let rewrite = state.rewrite_into(matched.state_match(), action, scratch, budget)?;
            Ok(PreparedRuleApplication::Rewrite(PreparedRewriteRule {
                permit,
                matched,
                rewrite,
            }))
        }
        RuleAction::Return(output) => {
            let output_view = ReturnOutputView::new(output);
            let output_len = ReturnOutputByteCount::from_payload_count(output.byte_count());
            budget.ensure_return_len(output_len)?;
            let materialized_output = materialize_return_output(output_view)?;

            Ok(PreparedRuleApplication::Return(PreparedReturnRule {
                permit,
                matched,
                output_view,
                output: materialized_output,
            }))
        }
    }
}
