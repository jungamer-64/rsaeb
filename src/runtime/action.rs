use crate::bytes::ReturnOutputByteCount;
use crate::error::RunError;
use crate::limits::StepCount;
use crate::program::{ReturnOutput, ReturnOutputView};
use crate::rule::{ParsedRuleAction, Rule};

use super::budget::RuntimeBudgetState;
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
pub(crate) enum PreparedRuleApplication<'program, 'once> {
    /// Prepared non-terminal rewrite.
    Rewrite(PreparedRewriteRule<'program, 'once>),
    /// Prepared terminal return.
    Return(PreparedReturnRule<'program, 'once>),
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
pub(crate) struct PreparedRewriteRule<'program, 'once> {
    /// Matched rule and once-state commit permit.
    matched: PreparedMatchedRule<'program, 'once>,
    /// Runtime bytes ready to become the next state.
    rewrite: PreparedRewrite,
}

/// Prepared terminal return before its step and once-state side effects commit.
#[derive(Debug)]
pub(crate) struct PreparedReturnRule<'program, 'once> {
    /// Matched rule and once-state commit permit.
    matched: PreparedMatchedRule<'program, 'once>,
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

impl<'program> PreparedRuleApplication<'program, '_> {
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
    /// Returns `RunError` if the next step exceeds the configured step limit.
    pub(crate) fn commit(
        self,
        state: &mut State,
        scratch: &mut RewriteScratch,
        budget: &mut RuntimeBudgetState,
    ) -> Result<AppliedRule<'program>, RunError> {
        let reservation = budget.reserve_next_step(state.byte_count())?;
        match self {
            Self::Rewrite(prepared) => {
                let committed = prepared.matched.commit();
                let step = reservation.commit();
                state.commit_rewrite(prepared.rewrite, scratch);
                Ok(AppliedRule::Rewrite(CommittedRewriteRule {
                    step,
                    rule: committed.rule(),
                }))
            }
            Self::Return(prepared) => {
                let committed = prepared.matched.commit();
                let step = reservation.commit();
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
    Ok(ReturnOutput::from_return_output_view(output)?)
}

/// Prepares one matched rule without committing state, budget, or once-rule side effects.
///
/// # Errors
///
/// Returns `RunError` if the next step exceeds limits, the rewrite would
/// exceed state limits, return output exceeds limits, or allocation fails.
pub(crate) fn prepare_matched_rule<'program, 'once>(
    scratch: &mut RewriteScratch,
    budget: &RuntimeBudgetState,
    matched: MatchedRuleApplication<'program, '_, 'once>,
) -> Result<PreparedRuleApplication<'program, 'once>, RunError> {
    let (state_match, matched) = matched.into_prepare_parts();
    match matched.rule().action() {
        ParsedRuleAction::Rewrite(action) => {
            let rewrite = state_match.rewrite_into(action, scratch, budget)?;
            Ok(PreparedRuleApplication::Rewrite(PreparedRewriteRule {
                matched,
                rewrite,
            }))
        }
        ParsedRuleAction::Return(output) => {
            let output_view = ReturnOutputView::new(output);
            let output_len = ReturnOutputByteCount::from_payload_count(output.byte_count());
            budget.ensure_return_len(output_len)?;
            let materialized_output = materialize_return_output(output_view)?;

            Ok(PreparedRuleApplication::Return(PreparedReturnRule {
                matched,
                output_view,
                output: materialized_output,
            }))
        }
    }
}
