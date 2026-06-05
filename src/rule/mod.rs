/// Canonical source generation from typed rule data.
mod canonical;
/// Parsed rule domain model.
mod model;

pub(crate) use canonical::{
    canonical_always_return_source, canonical_always_rewrite_source, canonical_once_return_source,
    canonical_once_rewrite_source,
};
pub(crate) use model::{
    ParsedRule, ReturnRule, RewriteAction, RewriteRule, RuleAnchorSyntax, RulePattern,
};
