/// Canonical source generation from typed rule data.
mod canonical;
/// Parsed rule domain model.
mod model;

pub(crate) use canonical::canonical_source;
pub(crate) use model::{
    ParsedRule, ParsedRuleAction, RewriteAction, Rule, RuleAnchorSyntax, RuleBody, RuleHead,
    RuleRepeatBehavior, RuleRepeatSyntax,
};
