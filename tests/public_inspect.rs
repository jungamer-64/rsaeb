//! Public inspection view contract tests.

mod support;

use rsaeb::inspect::{OnceRuleCount, RuleActionView, RuleAnchor, RuleRepeat};
use rsaeb::policy::DefaultParsePolicy;
use rsaeb::program::ExecutableProgram;
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
/// Returns `TestFailure` if all parser-to-runtime rule variants do not keep
/// repeat, action, and canonical-source shape distinct.
#[test]
fn inspect_all_repeat_and_action_rule_shapes() -> TestResult {
    let inspected = parse_program("a=b\n(once)c=d\ne=(return)ok\n(once)f=(return)done")?;
    let rules = inspected.rules().collect::<Vec<_>>();

    ensure_eq!(inspected.rule_count().get(), 4)?;
    ensure_eq!(inspected.once_rule_count().get(), 2)?;
    ensure_eq!(rules.len(), 4)?;

    let always_rewrite = rules
        .first()
        .copied()
        .ok_or(TestFailure::message("expected always rewrite"))?;
    let once_rewrite = rules
        .get(1)
        .copied()
        .ok_or(TestFailure::message("expected once rewrite"))?;
    let always_return = rules
        .get(2)
        .copied()
        .ok_or(TestFailure::message("expected always return"))?;
    let once_return = rules
        .get(3)
        .copied()
        .ok_or(TestFailure::message("expected once return"))?;

    ensure_eq!(always_rewrite.repeat(), RuleRepeat::Always)?;
    ensure_eq!(
        always_rewrite.canonical_source()?.as_slice(),
        b"a=b".as_slice()
    )?;
    match always_rewrite.action() {
        RuleActionView::Replace(payload) => {
            ensure_eq!(payload.materialize()?.as_slice(), b"b".as_slice())?;
        }
        RuleActionView::MoveStart(_) | RuleActionView::MoveEnd(_) | RuleActionView::Return(_) => {
            return Err(TestFailure::message("expected always rewrite"));
        }
    }

    ensure_eq!(once_rewrite.repeat(), RuleRepeat::Once)?;
    ensure_eq!(
        once_rewrite.canonical_source()?.as_slice(),
        b"(once)c=d".as_slice(),
    )?;
    match once_rewrite.action() {
        RuleActionView::Replace(payload) => {
            ensure_eq!(payload.materialize()?.as_slice(), b"d".as_slice())?;
        }
        RuleActionView::MoveStart(_) | RuleActionView::MoveEnd(_) | RuleActionView::Return(_) => {
            return Err(TestFailure::message("expected once rewrite"));
        }
    }

    ensure_eq!(always_return.repeat(), RuleRepeat::Always)?;
    ensure_eq!(
        always_return.canonical_source()?.as_slice(),
        b"e=(return)ok".as_slice(),
    )?;
    match always_return.action() {
        RuleActionView::Return(payload) => {
            ensure_eq!(payload.materialize()?.as_slice(), b"ok".as_slice())?;
        }
        RuleActionView::Replace(_) | RuleActionView::MoveStart(_) | RuleActionView::MoveEnd(_) => {
            return Err(TestFailure::message("expected always return"));
        }
    }

    ensure_eq!(once_return.repeat(), RuleRepeat::Once)?;
    ensure_eq!(
        once_return.canonical_source()?.as_slice(),
        b"(once)f=(return)done".as_slice(),
    )?;
    match once_return.action() {
        RuleActionView::Return(payload) => {
            ensure_eq!(payload.materialize()?.as_slice(), b"done".as_slice())?;
        }
        RuleActionView::Replace(_) | RuleActionView::MoveStart(_) | RuleActionView::MoveEnd(_) => {
            return Err(TestFailure::message("expected once return"));
        }
    }

    Ok(())
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

    let reparsed = ExecutableProgram::<DefaultParsePolicy>::parse(ProgramSource::from_bytes(
        canonical.as_slice(),
    ))?;
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
