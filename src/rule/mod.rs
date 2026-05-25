/// Canonical source generation from typed rule data.
mod canonical;
/// Parsed rule domain model.
mod model;

pub(crate) use canonical::canonical_source;
pub(crate) use model::{
    OnceRuleCount, OnceRuleSlot, ParsedRule, ParsedRuleAction, RewriteAction, Rule,
    RuleAnchorSyntax, RuleAvailability, RuleBody, RuleHead, RuleRepeatSyntax,
};
