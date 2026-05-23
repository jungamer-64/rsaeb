/// Matched-rule application and step effects.
pub(crate) mod action;
/// Runtime budget tracking and step permits.
pub(crate) mod budget;
/// Rule-table scanning and match witnesses.
pub(crate) mod matcher;
/// Per-run `(once)` rule state.
pub(crate) mod once;
/// Rewrite scratch storage.
pub(crate) mod rewrite;
/// Mutable runtime state and matching logic.
pub(crate) mod state;

#[cfg(test)]
mod tests;
