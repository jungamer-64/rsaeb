use super::*;
use crate::error::{AebError, InputError, LimitError, StateLimitContext};
use crate::inspect::{RuleActionView, RuleAnchor, RuleCount, RuleRepeat};
use crate::limits::{
    ReturnByteLimit, ReturnOutputByteCount, RuntimeStateByteCount, StateByteLimit,
};
use crate::test_support::{
    TestFailure, TestResult, ensure, ensure_eq, ensure_matches, expect_event, expect_return_output,
    expect_run_error, expect_stable_output, expect_state_limit, result_bytes, run_program,
    trace_event_bytes,
};
use crate::trace::{TraceSnapshotEffect, TraceSnapshotEvent};
use std::vec::Vec;

fn expect_rule(program: &Program, index: usize) -> Result<RuleView<'_>, TestFailure> {
    program
        .rules()
        .nth(index)
        .ok_or(TestFailure::message("expected parsed rule"))
}

#[test]
fn public_typed_run_works() -> TestResult {
    let limits = RunLimits::new(
        crate::DEFAULT_MAX_STEPS,
        crate::DEFAULT_MAX_STATE_LEN,
        crate::DEFAULT_MAX_RETURN_LEN,
    );
    let program = Program::parse(crate::ProgramSource::from_str("a=b"))?;
    let result = program.run(crate::RuntimeInput::validate(b"a")?, limits)?;
    expect_stable_output(&result, b"b")?;
    ensure_eq!(result.steps().get(), 1)?;

    let program = Program::parse(crate::ProgramSource::from_bytes(b"a=b#\xff"))?;
    let result = program.run(crate::RuntimeInput::validate(b"a")?, limits)?;
    expect_stable_output(&result, b"b")?;
    Ok(())
}

#[test]
fn runtime_input_boundary_is_validated_before_run() -> TestResult {
    let Err(error) = crate::RuntimeInput::validate(&[0xff]) else {
        return Err(TestFailure::message("expected input error"));
    };

    ensure_matches(
        matches!(
            error,
            InputError::NonAscii { column, .. }
                if column.get() == 1
        ),
        "expected runtime input error",
    )
}

#[test]
fn aeb_error_covers_runtime_input_validation() -> TestResult {
    let Err(input_error) = crate::RuntimeInput::validate("あ".as_bytes()) else {
        return Err(TestFailure::message("expected input validation error"));
    };
    let error = AebError::from(input_error);

    ensure_matches(
        matches!(error, AebError::Input(_)),
        "expected top-level input error",
    )
}

#[test]
fn parsed_program_is_reusable_and_once_state_is_per_run() -> TestResult {
    let program = Program::parse(crate::ProgramSource::from_str("(once)a=b\na=c"))?;

    let limits = RunLimits::new(
        StepLimit::new(10_000),
        crate::DEFAULT_MAX_STATE_LEN,
        crate::DEFAULT_MAX_RETURN_LEN,
    );
    let first = run_program(&program, b"aa", limits)?;
    let second = run_program(&program, b"aa", limits)?;

    ensure_eq!(result_bytes(&first), b"bc".as_slice())?;
    ensure_eq!(result_bytes(&second), b"bc".as_slice())?;
    ensure_eq!(program.once_rule_count(), RuleCount::new(1))?;
    Ok(())
}

#[test]
fn always_rules_do_not_allocate_once_slots() -> TestResult {
    let program = Program::parse(crate::ProgramSource::from_str("a=b\nb=c\n(start)c=d"))?;

    ensure_eq!(program.rule_count(), RuleCount::new(3))?;
    ensure_eq!(program.once_rule_count(), RuleCount::new(0))?;
    Ok(())
}

#[test]
fn run_outcome_separates_stable_state_from_return_output() -> TestResult {
    let limits = RunLimits::new(
        StepLimit::new(1),
        crate::DEFAULT_MAX_STATE_LEN,
        crate::DEFAULT_MAX_RETURN_LEN,
    );
    let stable = run_program(
        &Program::parse(crate::ProgramSource::from_str("a=b"))?,
        b"a",
        limits,
    )?;
    let returned = run_program(
        &Program::parse(crate::ProgramSource::from_str("a=(return)b"))?,
        b"a",
        limits,
    )?;

    match stable.into_outcome() {
        RunOutcome::Stable(output) => {
            ensure_eq!(output.as_bytes(), b"b".as_slice())?;
            ensure_eq!(output.byte_count(), RuntimeStateByteCount::new(1))?;
        }
        RunOutcome::Return(_) => return Err(TestFailure::message("expected stable outcome")),
    }

    match returned.into_outcome() {
        RunOutcome::Return(output) => {
            ensure_eq!(output.as_bytes(), b"b".as_slice())?;
            ensure_eq!(output.byte_count(), ReturnOutputByteCount::new(1))?;
        }
        RunOutcome::Stable(_) => return Err(TestFailure::message("expected return outcome")),
    }

    Ok(())
}

#[test]
fn rule_view_generates_canonical_source_without_stored_source_blob() -> TestResult {
    let program = Program::parse(crate::ProgramSource::from_str(
        "a = b # comment\n(start)c=(end)d",
    ))?;
    let rules = program.rules().collect::<Vec<_>>();

    ensure_eq!(rules.len(), 2)?;
    let first = rules
        .first()
        .copied()
        .ok_or(TestFailure::message("expected first rule"))?;
    let second = rules
        .get(1)
        .copied()
        .ok_or(TestFailure::message("expected second rule"))?;

    ensure_eq!(first.position().number().get(), 1)?;
    ensure_eq!(first.line_number().get(), 1)?;
    ensure_eq!(first.repeat(), RuleRepeat::Always)?;
    ensure_eq!(first.anchor(), RuleAnchor::Anywhere)?;
    ensure(first.lhs().eq_bytes(b"a"), "expected first lhs")?;
    ensure_matches(
        matches!(
            first.action(),
            RuleActionView::Replace(payload) if payload.eq_bytes(b"b")
        ),
        "expected replace action",
    )?;
    ensure_eq!(first.canonical_source()?, b"a=b".as_slice())?;

    ensure_eq!(second.position().number().get(), 2)?;
    ensure_eq!(second.line_number().get(), 2)?;
    ensure_eq!(second.repeat(), RuleRepeat::Always)?;
    ensure_eq!(second.anchor(), RuleAnchor::Start)?;
    ensure(second.lhs().eq_bytes(b"c"), "expected second lhs")?;
    ensure_matches(
        matches!(
            second.action(),
            RuleActionView::MoveEnd(payload) if payload.eq_bytes(b"d")
        ),
        "expected move-end action",
    )?;
    ensure_eq!(second.canonical_source()?, b"(start)c=(end)d".as_slice())?;
    Ok(())
}

#[test]
fn canonical_source_reparses_to_the_same_executable_rule() -> TestResult {
    let program = Program::parse(crate::ProgramSource::from_str(
        "( once ) ( start ) a = ( end ) b # comment",
    ))?;
    let canonical = expect_rule(&program, 0)?.canonical_source()?;

    let reparsed = Program::parse(crate::ProgramSource::from_bytes(canonical.as_slice()))?;
    let reparsed_rule = expect_rule(&reparsed, 0)?;

    ensure_eq!(reparsed.rule_count(), RuleCount::new(1))?;
    ensure_eq!(reparsed.once_rule_count(), RuleCount::new(1))?;
    ensure_eq!(reparsed_rule.repeat(), RuleRepeat::Once)?;
    ensure_eq!(reparsed_rule.anchor(), RuleAnchor::Start)?;
    ensure(reparsed_rule.lhs().eq_bytes(b"a"), "expected lhs")?;
    ensure_eq!(
        reparsed_rule.canonical_source()?,
        b"(once)(start)a=(end)b".as_slice(),
    )?;
    Ok(())
}

const EMPTY: &[u8] = b"";
const ONCE: &[u8] = b"(once)";
const START: &[u8] = b"(start)";
const END: &[u8] = b"(end)";
const RETURN: &[u8] = b"(return)";
const A: &[u8] = b"a";
const B: &[u8] = b"b";
const EQUALS: &[u8] = b"=";

fn collect_canonical_rule_shapes(
    dimensions: &[&[&[u8]]],
    current: &mut Vec<u8>,
    shapes: &mut Vec<Vec<u8>>,
) {
    let Some((dimension, rest)) = dimensions.split_first() else {
        shapes.push(current.clone());
        return;
    };

    for &part in *dimension {
        let len = current.len();
        current.extend_from_slice(part);
        collect_canonical_rule_shapes(rest, current, shapes);
        current.truncate(len);
    }
}

fn canonical_rule_shapes() -> Vec<Vec<u8>> {
    let repeats: &[&[u8]] = &[EMPTY, ONCE];
    let anchors: &[&[u8]] = &[EMPTY, START, END];
    let left_payloads: &[&[u8]] = &[EMPTY, A];
    let separator: &[&[u8]] = &[EQUALS];
    let actions: &[&[u8]] = &[EMPTY, START, END, RETURN];
    let right_payloads: &[&[u8]] = &[EMPTY, B];

    let mut shapes = Vec::new();
    let mut current = Vec::new();
    collect_canonical_rule_shapes(
        &[
            repeats,
            anchors,
            left_payloads,
            separator,
            actions,
            right_payloads,
        ],
        &mut current,
        &mut shapes,
    );
    shapes
}

fn expect_canonical_roundtrip(source: &[u8]) -> TestResult {
    let program = Program::parse(crate::ProgramSource::from_bytes(source))?;
    let rule = expect_rule(&program, 0)?;
    let canonical = rule.canonical_source()?;

    ensure_eq!(program.rule_count(), RuleCount::new(1))?;
    ensure_eq!(canonical.as_slice(), source)?;

    let reparsed = Program::parse(crate::ProgramSource::from_bytes(&canonical))?;
    let reparsed_rule = expect_rule(&reparsed, 0)?;
    ensure_eq!(reparsed_rule.canonical_source()?, source)?;
    Ok(())
}

#[test]
fn canonical_source_roundtrips_all_supported_rule_shapes() -> TestResult {
    for source in canonical_rule_shapes() {
        expect_canonical_roundtrip(&source)?;
    }
    Ok(())
}

fn expect_state_limit_from_run(
    source: &str,
    input: &[u8],
    limits: RunLimits,
) -> Result<LimitError, TestFailure> {
    let error = expect_run_error(
        Program::parse(crate::ProgramSource::from_str(source))?
            .run(crate::RuntimeInput::validate(input)?, limits),
    )?;
    expect_state_limit(error)
}

#[test]
fn state_limit_rejects_oversized_input_before_runtime_allocation() -> TestResult {
    let error = expect_state_limit_from_run(
        "# no executable rules",
        b"aa",
        RunLimits::new(
            StepLimit::new(10),
            StateByteLimit::new(1),
            ReturnByteLimit::new(10),
        ),
    )?;
    ensure_eq!(
        error,
        LimitError::State {
            context: StateLimitContext::Input,
            limit: StateByteLimit::new(1),
            attempted_len: RuntimeStateByteCount::new(2),
        },
    )?;
    Ok(())
}

#[test]
fn state_limit_rejects_oversized_rewrite_before_allocating_next_state() -> TestResult {
    let error = expect_state_limit_from_run(
        "=a",
        b"aa",
        RunLimits::new(
            StepLimit::new(10),
            StateByteLimit::new(2),
            ReturnByteLimit::new(10),
        ),
    )?;
    ensure_eq!(
        error,
        LimitError::State {
            context: StateLimitContext::Rewrite,
            limit: StateByteLimit::new(2),
            attempted_len: RuntimeStateByteCount::new(3),
        },
    )?;
    Ok(())
}

#[test]
fn trace_snapshots_are_derived_from_borrowed_trace() -> TestResult {
    let program = Program::parse(crate::ProgramSource::from_str("a=b\nb=(return)ok"))?;
    let mut events = Vec::new();
    let limits = TraceSnapshotLimits::new(
        RunLimits::new(
            StepLimit::new(10_000),
            crate::DEFAULT_MAX_STATE_LEN,
            crate::DEFAULT_MAX_RETURN_LEN,
        ),
        DEFAULT_MAX_TRACE_SNAPSHOT_LEN,
    );
    let result = program.run_with_trace_snapshots(
        crate::RuntimeInput::validate(b"a")?,
        limits,
        |event| {
            events.push(event);
        },
    )?;

    expect_return_output(&result, b"ok")?;
    ensure_eq!(events.len(), 3)?;
    ensure_matches(
        matches!(events.first(), Some(TraceSnapshotEvent::Initial { .. })),
        "expected initial trace event",
    )?;
    let initial = expect_event(&events, 0)?;
    let first_step = expect_event(&events, 1)?;
    let second_step = expect_event(&events, 2)?;

    ensure_eq!(trace_event_bytes(initial), b"a".as_slice())?;
    ensure_eq!(trace_event_bytes(first_step), b"b".as_slice())?;
    ensure_eq!(trace_event_bytes(second_step), b"ok".as_slice())?;
    ensure_matches(
        matches!(
            first_step,
            TraceSnapshotEvent::Step {
                effect: TraceSnapshotEffect::Continue { .. },
                ..
            }
        ),
        "expected continue step",
    )?;
    ensure_matches(
        matches!(
            second_step,
            TraceSnapshotEvent::Step {
                effect: TraceSnapshotEffect::Return { .. },
                ..
            }
        ),
        "expected return step",
    )?;

    match first_step {
        TraceSnapshotEvent::Step {
            rule,
            effect: TraceSnapshotEffect::Continue { state },
            ..
        } => {
            ensure_eq!(state.as_bytes(), b"b".as_slice())?;
            ensure_eq!(rule.canonical_source()?, b"a=b".as_slice())?;
        }
        TraceSnapshotEvent::Initial { .. } | TraceSnapshotEvent::Step { .. } => {
            return Err(TestFailure::message("expected continue step"));
        }
    }
    Ok(())
}
