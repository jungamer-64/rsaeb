/// Internal canonical module.
mod canonical;
/// Internal model module.
mod model;

pub(crate) use canonical::canonical_source;
pub(crate) use model::{
    OnceRuleCount, OnceRuleSlot, ParsedRule, RewriteAction, Rule, RuleAction, RuleAnchorSyntax,
    RuleBody, RuleHead, RuleRepeatState, RuleRepeatSyntax,
};
