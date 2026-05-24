/// Canonical source generation from typed rule data.
mod canonical;
/// Parsed rule domain model.
mod model;

pub(crate) use canonical::canonical_source;
pub(crate) use model::{
    OnceRuleCount, ParsedRule, RewriteAction, Rule, RuleAction, RuleAnchorSyntax, RuleBody,
    RuleHead, RuleRepeatState, RuleRepeatSyntax,
};
