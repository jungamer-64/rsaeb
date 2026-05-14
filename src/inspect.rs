//! Borrowed inspection views for parsed rules and payloads.
//!
//! These types describe parsed program structure without exposing the internal
//! rule table or runtime execution state.

pub use crate::rule::{
    PayloadView, RuleActionView, RuleAnchor, RuleCount, RuleNumber, RulePosition, RuleRepeat,
    RuleView,
};
