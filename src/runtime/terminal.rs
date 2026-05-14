use crate::program::StepCount;
use crate::rule::{PayloadView, Rule};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExecutionTerminal<'program> {
    Running,
    Stable,
    Return {
        step: StepCount,
        rule: &'program Rule,
        output: PayloadView<'program>,
    },
}
