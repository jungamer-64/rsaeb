mod canonical;
mod model;

pub(crate) use canonical::canonical_source;
pub(crate) use model::{
    Action, OnceRuleSlot, ParsedRule, Rule, RuleBody, RuleHead, RuleRepeatState,
};
