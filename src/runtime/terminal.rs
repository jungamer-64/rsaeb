use crate::program::StepCount;
use crate::rule::RulePosition;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExecutionTerminal {
    Running,
    Stable,
    Return {
        step: StepCount,
        rule_position: RulePosition,
    },
}
