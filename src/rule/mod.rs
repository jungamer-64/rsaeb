/// Canonical source generation from typed rule data.
mod canonical;
/// Parsed rule domain model.
mod model;

pub(crate) use canonical::{canonical_always_source, canonical_once_source};
pub(crate) use model::{
    ParsedRule, ParsedRulePattern, RepeatRule, ReturnRule, RewriteAction, RewriteRule, Rule,
    RuleAnchorSyntax, RulePattern,
};
