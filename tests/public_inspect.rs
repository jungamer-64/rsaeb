//! Public inspection view contract tests.

mod support;

use rsaeb::inspect::{OnceRuleCount, RuleActionView, RuleAnchor, RuleRepeat};
use rsaeb::limits::DEFAULT_PARSE_LIMITS;
use rsaeb::program::Program;
use rsaeb::source::ProgramSource;
use support::{TestFailure, TestResult, ensure_eq, ensure_matches, parse_program};

/// # Errors
///
/// Returns `TestFailure` if rule views lose structured public data.
#[test]
fn inspect_rule_views_expose_structured_public_data() -> TestResult {
    let inspected = parse_program("a = b # comment\n(start)c=(end)d")?;
    let mut rules = inspected.rules();
    let first = rules
        .next()
        .ok_or(TestFailure::message("expected first parsed rule"))?;
    let second = rules
        .next()
        .ok_or(TestFailure::message("expected second parsed rule"))?;
    ensure_matches(rules.next().is_none(), "expected no extra rules")?;

    ensure_eq!(inspected.rule_count().get(), 2)?;
    ensure_eq!(first.line_number().get(), 1)?;
    ensure_eq!(first.repeat(), RuleRepeat::Always)?;
    ensure_eq!(first.anchor(), RuleAnchor::Anywhere)?;
    ensure_eq!(first.lhs().materialize()?.as_slice(), b"a".as_slice())?;
    match first.action() {
        RuleActionView::Replace(payload) => {
            ensure_eq!(payload.materialize()?.as_slice(), b"b".as_slice())?;
        }
        RuleActionView::MoveStart(_) | RuleActionView::MoveEnd(_) | RuleActionView::Return(_) => {
            return Err(TestFailure::message("expected replace action"));
        }
    }
    ensure_eq!(first.canonical_source()?.as_slice(), b"a=b".as_slice())?;

    ensure_eq!(second.line_number().get(), 2)?;
    ensure_eq!(second.anchor(), RuleAnchor::Start)?;
    match second.action() {
        RuleActionView::MoveEnd(payload) => {
            ensure_eq!(payload.materialize()?.as_slice(), b"d".as_slice())?;
        }
        RuleActionView::Replace(_) | RuleActionView::MoveStart(_) | RuleActionView::Return(_) => {
            return Err(TestFailure::message("expected move-end action"));
        }
    }
    ensure_eq!(
        second.canonical_source()?.as_slice(),
        b"(start)c=(end)d".as_slice(),
    )
}

/// # Errors
///
/// Returns `TestFailure` if canonical source does not reparse to the same
/// public rule view.
#[test]
fn inspect_canonical_source_reparses_to_same_public_rule_view() -> TestResult {
    let program = parse_program("( once ) ( start ) a = ( end ) b # comment")?;
    let rule = program
        .rules()
        .next()
        .ok_or(TestFailure::message("expected parsed rule"))?;
    let canonical = rule.canonical_source()?;

    let reparsed = Program::parse(
        ProgramSource::from_bytes(canonical.as_slice()),
        DEFAULT_PARSE_LIMITS,
    )?;
    let reparsed_rule = reparsed
        .rules()
        .next()
        .ok_or(TestFailure::message("expected reparsed rule"))?;

    ensure_eq!(reparsed.rule_count().get(), 1)?;
    let once_rules: OnceRuleCount = reparsed.once_rule_count();
    ensure_eq!(once_rules.get(), 1)?;
    ensure_eq!(reparsed_rule.repeat(), RuleRepeat::Once)?;
    ensure_eq!(reparsed_rule.anchor(), RuleAnchor::Start)?;
    ensure_eq!(
        reparsed_rule.lhs().materialize()?.as_slice(),
        b"a".as_slice(),
    )?;
    ensure_eq!(
        reparsed_rule.canonical_source()?.as_slice(),
        b"(once)(start)a=(end)b".as_slice(),
    )
}
