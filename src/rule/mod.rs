/// Canonical source generation from typed rule data.
mod canonical;
/// Parsed rule domain model.
mod model;

pub(crate) use model::{
    ParsedRule, ParsedRulePattern, RewriteAction, Rule, RuleAnchorSyntax, RulePattern,
};
