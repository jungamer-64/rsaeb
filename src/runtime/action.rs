use crate::bytes::ReturnOutputByteCount;
use crate::error::RunError;
use crate::inspect::RuleView;
use crate::limits::StepCount;
use crate::program::{ReturnOutput, ReturnOutputView};
use crate::rule::RuleAction;

use super::budget::RuntimeBudgetState;
use super::matcher::MatchedRuleApplication;
use super::rewrite::RewriteScratch;
use super::state::State;

/// Committed rule application reported back to the session layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppliedRule<'program> {
    /// One rewrite rule committed and execution may continue.
    Rewrite(CommittedRewriteRule<'program>),
    /// One return rule committed and execution is terminal.
    Return(CommittedReturnRule<'program>),
}

/// Committed non-terminal rewrite rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommittedRewriteRule<'program> {
    /// Step number assigned by the runtime budget.
    step: StepCount,
    /// Rule view proven to describe the committed rewrite rule.
    rule: RuleView<'program>,
}

/// Committed terminal return rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommittedReturnRule<'program> {
    /// Step number assigned by the runtime budget.
    step: StepCount,
    /// Rule view proven to describe the committed return rule.
    rule: RuleView<'program>,
    /// Return output borrowed directly from the committed return rule.
    output: ReturnOutputView<'program>,
}

impl<'program> CommittedRewriteRule<'program> {
    /// Step number assigned by the runtime budget.
    pub(crate) const fn step(self) -> StepCount {
        self.step
    }

    /// Rule view proven to describe the committed rewrite rule.
    pub(crate) const fn rule(self) -> RuleView<'program> {
        self.rule
    }
}

impl<'program> CommittedReturnRule<'program> {
    /// Step number assigned by the runtime budget.
    pub(crate) const fn step(self) -> StepCount {
        self.step
    }

    /// Rule view proven to describe the committed return rule.
    pub(crate) const fn rule(self) -> RuleView<'program> {
        self.rule
    }

    /// Return output borrowed directly from the committed return rule.
    pub(crate) const fn output(self) -> ReturnOutputView<'program> {
        self.output
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
    matched: MatchedRuleApplication<'program, '_>,
) -> Result<AppliedRule<'program>, RunError> {
    let permit = budget.reserve_next_step(state.byte_count())?;
    match matched.rule().action() {
        RuleAction::Rewrite(action) => {
            let rule = matched.rule();
            let rewrite = state.rewrite_into(matched.state_match(), action, scratch, *budget)?;
            let committed = matched.commit();
            let step = budget.commit(permit);
            state.commit_rewrite(rewrite, scratch);
            Ok(AppliedRule::Rewrite(CommittedRewriteRule {
                step,
                rule: RuleView::new(committed.position(), rule),
            }))
        }
        RuleAction::Return(output) => {
            let rule = matched.rule();
            let output_len = ReturnOutputByteCount::from_payload_count(output.byte_count());
            (*budget).ensure_return_len(output_len)?;

            let committed = matched.commit();
            let step = budget.commit(permit);
            Ok(AppliedRule::Return(CommittedReturnRule {
                step,
                rule: RuleView::new(committed.position(), rule),
                output: ReturnOutputView::new(output),
            }))
        }
    }
}
