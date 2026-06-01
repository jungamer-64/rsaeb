/// Runtime state for one parsed `(once)` rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OnceRuleRuntimeState {
    /// Rule has not committed during this run.
    Fresh,
    /// Rule has already committed during this run.
    Committed,
}

/// Linear commit action for a matched rule.
#[derive(Debug)]
pub(super) enum MatchedRuleCommit<'state> {
    /// Rule has no once-state side effect.
    Always,
    /// Rule owns the unique permit to consume its once state.
    Once(OnceMatchPermit<'state>),
}

/// Private permit that consumes one fresh once-rule state on commit.
#[derive(Debug)]
pub(super) struct OnceMatchPermit<'state> {
    /// Fresh per-rule state reserved for the matched rule.
    state: &'state mut OnceRuleRuntimeState,
    /// Non-copy token that keeps the permit linear even though its witnesses are copyable.
    linearity: OnceMatchPermitLinearity,
}

/// Non-copy marker carried by once-rule commit permits.
#[derive(Debug)]
struct OnceMatchPermitLinearity;

impl OnceMatchPermitLinearity {
    /// Creates the linearity marker for one permit.
    const fn new() -> Self {
        Self
    }
}

impl<'state> OnceMatchPermit<'state> {
    /// Creates the commit permit after availability has been checked.
    pub(super) fn new(state: &'state mut OnceRuleRuntimeState) -> Self {
        Self {
            state,
            linearity: OnceMatchPermitLinearity::new(),
        }
    }
}

impl MatchedRuleCommit<'_> {
    /// Applies the rule's once-state side effect after rewrite success.
    pub(super) fn commit(self) {
        match self {
            Self::Always => {}
            Self::Once(commit) => commit.commit(),
        }
    }
}

impl OnceMatchPermit<'_> {
    /// Consumes this permit and marks the owning once-rule state as consumed.
    fn commit(self) {
        let Self {
            state,
            linearity: _linearity,
        } = self;
        *state = OnceRuleRuntimeState::Committed;
    }
}
