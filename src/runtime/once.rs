use alloc::{collections::VecDeque, vec::Vec};

use crate::allocation::{
    AllocationContext, AllocationError, RequestedCapacity, try_push, try_reserve_total_exact,
};
use crate::inspect::{OnceReturnRuleView, OnceRewriteRuleView};
use crate::policy::ParsePolicy;
use crate::program::{ExecutableProgram, RuleScan};
use crate::rule::{ReturnRule, RewriteRule, Rule};
use crate::runtime::matcher::{
    AvailableRuleAttempt, MatchedRuleApplication, RuleAttempt, RuleAttemptMiss,
    attempt_available_rule,
};
use crate::runtime::state::State;

/// Per-run ordinary execution table with parsed rules and runtime availability paired.
#[derive(Debug)]
pub(crate) struct RuntimeRuleTable<'program> {
    /// Runtime rule cells in parser execution order.
    cells: Vec<RuntimeRuleCell<'program>>,
}

/// Outcome of scanning the ordinary runtime rule table.
#[derive(Debug)]
pub(crate) enum RuntimeRuleSearch<'program, 'state, 'once> {
    /// A rule matched and carries the commit permit needed after success.
    Matched(MatchedRuleApplication<'program, 'state, 'once>),
    /// No currently available rule matched the runtime state.
    Stable,
}

/// Non-empty tail of unattempted rules after a continuing current target.
#[derive(Debug)]
struct PendingRuntimeRules<'program> {
    /// Next rule after the current target.
    next: RuntimeRuleCell<'program>,
    /// Remaining rules after `next`, in original rule order.
    remaining: VecDeque<RuntimeRuleCell<'program>>,
}

/// One executable rule classified by its run-local availability shape.
#[derive(Debug)]
enum RuntimeRuleCell<'program> {
    /// Reusable non-terminal rewrite rule.
    AlwaysRewrite(AlwaysRewriteRuntimeRuleCell<'program>),
    /// Once-only non-terminal rewrite rule.
    OnceRewrite(OnceRewriteRuntimeRuleCell<'program>),
    /// Reusable terminal return rule.
    AlwaysReturn(AlwaysReturnRuntimeRuleCell<'program>),
    /// Once-only terminal return rule.
    OnceReturn(OnceReturnRuntimeRuleCell<'program>),
}

/// Runtime cell for a reusable rewrite rule.
#[derive(Debug)]
struct AlwaysRewriteRuntimeRuleCell<'program> {
    /// Parsed executable rule.
    rule: &'program RewriteRule,
}

/// Runtime cell for a once-only rewrite rule.
#[derive(Debug)]
struct OnceRewriteRuntimeRuleCell<'program> {
    /// Parsed executable rule.
    rule: &'program RewriteRule,
    /// Run-local availability for this once rule.
    state: OnceRuleRuntimeState,
}

/// Runtime cell for a reusable return rule.
#[derive(Debug)]
struct AlwaysReturnRuntimeRuleCell<'program> {
    /// Parsed executable rule.
    rule: &'program ReturnRule,
}

/// Runtime cell for a once-only return rule.
#[derive(Debug)]
struct OnceReturnRuntimeRuleCell<'program> {
    /// Parsed executable rule.
    rule: &'program ReturnRule,
    /// Run-local availability for this once rule.
    state: OnceRuleRuntimeState,
}

/// Runtime availability state for one parsed `(once)` executable rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OnceRuleRuntimeState {
    /// Rule has not committed during this run.
    Fresh,
    /// Rule has already committed during this run.
    Committed,
}

/// Private permit that consumes one fresh once-rule state on commit.
#[derive(Debug)]
pub(crate) struct OnceMatchPermit<'state> {
    /// Fresh per-rule state reserved for the matched rule.
    state: &'state mut OnceRuleRuntimeState,
    /// Non-copy token that keeps the permit linear even though its witnesses are copyable.
    linearity: OnceMatchPermitLinearity,
}

/// Non-copy marker carried by once-rule commit permits.
#[derive(Debug)]
struct OnceMatchPermitLinearity;

/// Runtime cell classification before state matching.
#[derive(Debug)]
enum RuntimeRuleTarget<'program, 'once> {
    /// The rule can be evaluated against the current runtime state.
    Available(AvailableRuntimeRule<'program, 'once>),
    /// The rule has already committed during this runtime invocation.
    Consumed(ConsumedRuntimeRule<'program>),
}

/// Parsed once-only rule that is already consumed in the current run.
#[derive(Debug)]
enum ConsumedRuntimeRule<'program> {
    /// Consumed once-only rewrite rule.
    OnceRewrite(OnceRewriteRuleView<'program>),
    /// Consumed once-only return rule.
    OnceReturn(OnceReturnRuleView<'program>),
}

/// Parsed rule proven available for runtime-state matching.
#[derive(Debug)]
pub(crate) enum AvailableRuntimeRule<'program, 'once> {
    /// Reusable rewrite rule.
    AlwaysRewrite(AvailableAlwaysRewriteRuntimeRule<'program>),
    /// Fresh once-only rewrite rule paired with its linear commit permit.
    OnceRewrite(AvailableOnceRewriteRuntimeRule<'program, 'once>),
    /// Reusable return rule.
    AlwaysReturn(AvailableAlwaysReturnRuntimeRule<'program>),
    /// Fresh once-only return rule paired with its linear commit permit.
    OnceReturn(AvailableOnceReturnRuntimeRule<'program, 'once>),
}

/// Reusable rewrite rule proven available for runtime-state matching.
#[derive(Debug)]
pub(crate) struct AvailableAlwaysRewriteRuntimeRule<'program> {
    /// Parsed executable rule.
    rule: &'program RewriteRule,
}

/// Fresh once-only rewrite rule paired with the permit that can consume it after a match commits.
#[derive(Debug)]
pub(crate) struct AvailableOnceRewriteRuntimeRule<'program, 'once> {
    /// Parsed executable rule.
    rule: &'program RewriteRule,
    /// Linear once-state commit permit.
    commit: OnceMatchPermit<'once>,
}

/// Reusable return rule proven available for runtime-state matching.
#[derive(Debug)]
pub(crate) struct AvailableAlwaysReturnRuntimeRule<'program> {
    /// Parsed executable rule.
    rule: &'program ReturnRule,
}

/// Fresh once-only return rule paired with the permit that can consume it after a match commits.
#[derive(Debug)]
pub(crate) struct AvailableOnceReturnRuntimeRule<'program, 'once> {
    /// Parsed executable rule.
    rule: &'program ReturnRule,
    /// Linear once-state commit permit.
    commit: OnceMatchPermit<'once>,
}

impl OnceMatchPermitLinearity {
    /// Creates the linearity marker for one permit.
    const fn new() -> Self {
        Self
    }
}

impl<'program> RuntimeRuleTable<'program> {
    /// Builds a run-local ordinary execution table from an executable program.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the per-execution rule table cannot be
    /// allocated.
    pub(crate) fn from_program<P: ParsePolicy>(
        program: &'program ExecutableProgram<P>,
    ) -> Result<Self, AllocationError> {
        Self::from_rule_scan(program.rule_scan())
    }

    /// Builds a run-local ordinary execution table from the executable rule table.
    ///
    /// # Errors
    ///
    /// Returns `AllocationError` if the per-execution rule table cannot be
    /// allocated.
    fn from_rule_scan(rules: RuleScan<'program>) -> Result<Self, AllocationError> {
        let rule_count = rules.iter().count();
        let mut cells = Vec::new();
        try_reserve_total_exact(
            &mut cells,
            RequestedCapacity::new(rule_count),
            AllocationContext::RuntimeRuleCell,
        )?;
        for rule in rules.iter() {
            try_push(
                &mut cells,
                RuntimeRuleCell::new(rule),
                AllocationContext::RuntimeRuleCell,
            )?;
        }

        Ok(Self { cells })
    }

    /// Finds the first currently available rule that matches `state`.
    pub(crate) fn find_next_match<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuntimeRuleSearch<'program, 'state, 'once> {
        for cell in &mut self.cells {
            match cell.attempt(state) {
                RuleAttempt::Matched(matched) => return RuntimeRuleSearch::Matched(matched),
                RuleAttempt::Missed(_missed) => {}
            }
        }

        RuntimeRuleSearch::Stable
    }
}

impl<'program> RuntimeRuleCell<'program> {
    /// Builds a runtime rule cell from typed parsed rule data.
    fn new(rule: &'program Rule) -> Self {
        match rule {
            Rule::AlwaysRewrite(rule) => Self::AlwaysRewrite(AlwaysRewriteRuntimeRuleCell { rule }),
            Rule::OnceRewrite(rule) => Self::OnceRewrite(OnceRewriteRuntimeRuleCell {
                rule,
                state: OnceRuleRuntimeState::Fresh,
            }),
            Rule::AlwaysReturn(rule) => Self::AlwaysReturn(AlwaysReturnRuntimeRuleCell { rule }),
            Rule::OnceReturn(rule) => Self::OnceReturn(OnceReturnRuntimeRuleCell {
                rule,
                state: OnceRuleRuntimeState::Fresh,
            }),
        }
    }

    /// Attempts this rule cell against the current runtime state.
    fn attempt<'state, 'once>(
        &'once mut self,
        state: &'state State,
    ) -> RuleAttempt<'program, 'state, 'once> {
        match self.target() {
            RuntimeRuleTarget::Available(target) => match attempt_available_rule(target, state) {
                AvailableRuleAttempt::Matched(matched) => RuleAttempt::Matched(matched),
                AvailableRuleAttempt::StateMismatch(miss) => RuleAttempt::Missed(miss),
            },
            RuntimeRuleTarget::Consumed(rule) => RuleAttempt::Missed(rule.into_miss()),
        }
    }

    /// Classifies this cell before runtime-state matching.
    fn target(&mut self) -> RuntimeRuleTarget<'program, '_> {
        match self {
            Self::AlwaysRewrite(cell) => {
                RuntimeRuleTarget::Available(AvailableRuntimeRule::AlwaysRewrite(
                    AvailableAlwaysRewriteRuntimeRule { rule: cell.rule },
                ))
            }
            Self::OnceRewrite(cell) => {
                if matches!(cell.state, OnceRuleRuntimeState::Fresh) {
                    RuntimeRuleTarget::Available(AvailableRuntimeRule::OnceRewrite(
                        AvailableOnceRewriteRuntimeRule {
                            rule: cell.rule,
                            commit: OnceMatchPermit::new(&mut cell.state),
                        },
                    ))
                } else {
                    RuntimeRuleTarget::Consumed(ConsumedRuntimeRule::OnceRewrite(
                        OnceRewriteRuleView::new(cell.rule),
                    ))
                }
            }
            Self::AlwaysReturn(cell) => {
                RuntimeRuleTarget::Available(AvailableRuntimeRule::AlwaysReturn(
                    AvailableAlwaysReturnRuntimeRule { rule: cell.rule },
                ))
            }
            Self::OnceReturn(cell) => {
                if matches!(cell.state, OnceRuleRuntimeState::Fresh) {
                    RuntimeRuleTarget::Available(AvailableRuntimeRule::OnceReturn(
                        AvailableOnceReturnRuntimeRule {
                            rule: cell.rule,
                            commit: OnceMatchPermit::new(&mut cell.state),
                        },
                    ))
                } else {
                    RuntimeRuleTarget::Consumed(ConsumedRuntimeRule::OnceReturn(
                        OnceReturnRuleView::new(cell.rule),
                    ))
                }
            }
        }
    }
}

impl<'program> ConsumedRuntimeRule<'program> {
    /// Projects this consumed once-only rule into a non-applying attempt.
    const fn into_miss(self) -> RuleAttemptMiss<'program> {
        match self {
            Self::OnceRewrite(rule) => RuleAttemptMiss::once_rewrite_consumed(rule),
            Self::OnceReturn(rule) => RuleAttemptMiss::once_return_consumed(rule),
        }
    }
}

/// Reserves a rule queue through the runtime-rule allocation boundary.
///
/// # Errors
///
/// Returns `AllocationError` if the requested capacity cannot be represented
/// or if the allocator rejects the reservation.
fn try_reserve_rule_queue<T>(
    queue: &mut VecDeque<T>,
    total_capacity: RequestedCapacity,
    context: AllocationContext,
) -> Result<(), AllocationError> {
    if queue.capacity() >= total_capacity.get() {
        return Ok(());
    }

    let additional = total_capacity
        .get()
        .checked_sub(queue.len())
        .ok_or_else(|| AllocationError::capacity_overflow(context))?;

    queue
        .try_reserve_exact(additional)
        .map_err(|_| AllocationError::reservation_failed(context, total_capacity))
}

impl<'program> AvailableAlwaysRewriteRuntimeRule<'program> {
    /// Parsed reusable rewrite rule selected with no once state.
    pub(crate) const fn rule(&self) -> &'program RewriteRule {
        self.rule
    }
}

impl<'program, 'once> AvailableOnceRewriteRuntimeRule<'program, 'once> {
    /// Splits this available once target into its rule and linear commit permit.
    pub(crate) fn into_parts(self) -> (&'program RewriteRule, OnceMatchPermit<'once>) {
        (self.rule, self.commit)
    }
}

impl<'program> AvailableAlwaysReturnRuntimeRule<'program> {
    /// Parsed reusable return rule selected with no once state.
    pub(crate) const fn rule(&self) -> &'program ReturnRule {
        self.rule
    }
}

impl<'program, 'once> AvailableOnceReturnRuntimeRule<'program, 'once> {
    /// Splits this available once target into its rule and linear commit permit.
    pub(crate) fn into_parts(self) -> (&'program ReturnRule, OnceMatchPermit<'once>) {
        (self.rule, self.commit)
    }
}

impl<'state> OnceMatchPermit<'state> {
    /// Creates the commit permit after availability has been checked.
    fn new(state: &'state mut OnceRuleRuntimeState) -> Self {
        Self {
            state,
            linearity: OnceMatchPermitLinearity::new(),
        }
    }
}

impl OnceMatchPermit<'_> {
    /// Consumes this permit and marks the owning once-rule state as consumed.
    pub(crate) fn commit(self) {
        let Self {
            state,
            linearity: _linearity,
        } = self;
        *state = OnceRuleRuntimeState::Committed;
    }
}
