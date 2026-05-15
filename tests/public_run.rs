mod support;

use rsaeb::error::{LimitError, RunError};
use rsaeb::inspect::{RuleActionView, RuleAnchor, RuleRepeat};
use rsaeb::limits::{ReturnByteLimit, StateByteLimit, StepLimit};
use rsaeb::{
    DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_STEPS, ExecutionStep, Program,
    ProgramSource, RunLimits, RunOutcome, RunResult, RuntimeInput, RuntimeInputBytes,
};
use support::{TestFailure, TestResult, ensure, ensure_eq, ensure_matches};

fn expect_stable_bytes<'result>(
    result: &'result RunResult,
    expected: &[u8],
) -> Result<&'result [u8], TestFailure> {
    match result.outcome() {
        RunOutcome::Stable(output) if output.as_bytes() == expected => Ok(output.as_bytes()),
        RunOutcome::Stable(_) => Err(TestFailure::message("stable output bytes differed")),
        RunOutcome::Return(_) => Err(TestFailure::message("expected stable outcome")),
    }
}

fn expect_return_bytes<'result>(
    result: &'result RunResult,
    expected: &[u8],
) -> Result<&'result [u8], TestFailure> {
    match result.outcome() {
        RunOutcome::Return(output) if output.as_bytes() == expected => Ok(output.as_bytes()),
        RunOutcome::Return(_) => Err(TestFailure::message("return output bytes differed")),
        RunOutcome::Stable(_) => Err(TestFailure::message("expected return outcome")),
    }
}

fn runtime_view_bytes(state: rsaeb::trace::RuntimeStateView<'_>) -> Vec<u8> {
    state.bytes().collect()
}

#[derive(Debug, PartialEq, Eq)]
enum StepSignature {
    Applied {
        step: usize,
        rule: Vec<u8>,
        state: Vec<u8>,
    },
    Stable {
        steps: usize,
        state: Vec<u8>,
    },
    Return {
        step: usize,
        rule: Vec<u8>,
        output: Vec<u8>,
    },
}

fn step_signature(step: ExecutionStep<'_, '_>) -> Result<StepSignature, TestFailure> {
    match step {
        ExecutionStep::Applied { step, rule, state } => Ok(StepSignature::Applied {
            step: step.get(),
            rule: rule.canonical_source()?,
            state: runtime_view_bytes(state),
        }),
        ExecutionStep::Stable { steps, state } => Ok(StepSignature::Stable {
            steps: steps.get(),
            state: runtime_view_bytes(state),
        }),
        ExecutionStep::Return { step, rule, output } => Ok(StepSignature::Return {
            step: step.get(),
            rule: rule.canonical_source()?,
            output: output.to_vec()?,
        }),
    }
}

fn expect_run_error<T>(result: Result<T, RunError>) -> Result<RunError, TestFailure> {
    match result {
        Ok(_) => Err(TestFailure::message("expected runtime error")),
        Err(error) => Ok(error),
    }
}

fn expect_step_limit(error: RunError) -> Result<LimitError, TestFailure> {
    match error {
        RunError::Limit(error @ LimitError::Step { .. }) => Ok(error),
        RunError::Allocation(_)
        | RunError::StateSize(_)
        | RunError::Limit(_)
        | RunError::Invariant(_) => Err(TestFailure::message("expected step limit error")),
    }
}

fn expect_state_limit(error: RunError) -> Result<LimitError, TestFailure> {
    match error {
        RunError::Limit(error @ LimitError::State { .. }) => Ok(error),
        RunError::Allocation(_)
        | RunError::StateSize(_)
        | RunError::Limit(_)
        | RunError::Invariant(_) => Err(TestFailure::message("expected state limit error")),
    }
}

#[test]
fn public_typed_boundaries_parse_and_run_programs() -> TestResult {
    let limits = RunLimits::new(
        DEFAULT_MAX_STEPS,
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );

    let program = Program::parse(ProgramSource::from_str("a=b"))?;
    let input = RuntimeInput::validate(b"a")?;
    let result = program.run(input, limits)?;
    expect_stable_bytes(&result, b"b")?;
    ensure_eq!(result.steps().get(), 1)?;

    let program = Program::parse(ProgramSource::from_bytes(b"a=b#\xff"))?;
    let input = RuntimeInput::validate(b"a")?;
    let result = program.run(input, limits)?;
    expect_stable_bytes(&result, b"b")?;
    Ok(())
}

#[test]
fn language_whitespace_comments_and_actions_are_public_contract() -> TestResult {
    let limits = RunLimits::new(
        StepLimit::new(10_000),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );

    let program = Program::parse(ProgramSource::from_str("a b=bb"))?;
    let result = program.run(RuntimeInput::validate(b"abc")?, limits)?;
    expect_stable_bytes(&result, b"bbc")?;

    let program = Program::parse(ProgramSource::from_str("a=b\r\nb=c\r\n"))?;
    let result = program.run(RuntimeInput::validate(b"a")?, limits)?;
    expect_stable_bytes(&result, b"c")?;

    let program = Program::parse(ProgramSource::from_str("a\tb = c\tc"))?;
    let result = program.run(RuntimeInput::validate(b"ab")?, limits)?;
    expect_stable_bytes(&result, b"cc")?;

    let program = Program::parse(ProgramSource::from_str("a=b#ignored"))?;
    let result = program.run(RuntimeInput::validate(b"a")?, limits)?;
    expect_stable_bytes(&result, b"b")?;

    let program = Program::parse(ProgramSource::from_str("#a=b"))?;
    let result = program.run(RuntimeInput::validate(b"a")?, limits)?;
    expect_stable_bytes(&result, b"a")?;

    let program = Program::parse(ProgramSource::from_str("a=(start)x"))?;
    let result = program.run(RuntimeInput::validate(b"ba")?, limits)?;
    expect_stable_bytes(&result, b"xb")?;

    let program = Program::parse(ProgramSource::from_str("a=(end)x"))?;
    let result = program.run(RuntimeInput::validate(b"ba")?, limits)?;
    expect_stable_bytes(&result, b"bx")?;

    let program = Program::parse(ProgramSource::from_str("a=(return)ok"))?;
    let result = program.run(RuntimeInput::validate(b"a")?, limits)?;
    expect_return_bytes(&result, b"ok")?;
    Ok(())
}

#[test]
fn rewrite_order_anchors_once_and_runtime_only_bytes_are_public_contract() -> TestResult {
    let limits = RunLimits::new(
        StepLimit::new(10_000),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );

    let program = Program::parse(ProgramSource::from_str("aa=x\na=y"))?;
    let result = program.run(RuntimeInput::validate(b"aaaa")?, limits)?;
    expect_stable_bytes(&result, b"xx")?;

    let program = Program::parse(ProgramSource::from_str("(start)a=x"))?;
    let result = program.run(RuntimeInput::validate(b"aba")?, limits)?;
    expect_stable_bytes(&result, b"xba")?;

    let program = Program::parse(ProgramSource::from_str("(end)a=x"))?;
    let result = program.run(RuntimeInput::validate(b"aba")?, limits)?;
    expect_stable_bytes(&result, b"abx")?;

    let program = Program::parse(ProgramSource::from_str("(once)a=b\na=c"))?;
    let result = program.run(RuntimeInput::validate(b"aa")?, limits)?;
    expect_stable_bytes(&result, b"bc")?;

    let program = Program::parse(ProgramSource::from_str("ab=x"))?;
    let result = program.run(RuntimeInput::validate(b"a=b")?, limits)?;
    expect_stable_bytes(&result, b"a=b")?;

    let program = Program::parse(ProgramSource::from_str("a= b"))?;
    let result = program.run(RuntimeInput::validate(b"a bc")?, limits)?;
    expect_stable_bytes(&result, b"b bc")?;
    Ok(())
}

#[test]
fn parsed_program_is_reusable_and_rule_views_are_structured() -> TestResult {
    let limits = RunLimits::new(
        StepLimit::new(10_000),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let program = Program::parse(ProgramSource::from_str("(once)a=b\na=c"))?;
    let first = program.run(RuntimeInput::validate(b"aa")?, limits)?;
    let second = program.run(RuntimeInput::validate(b"aa")?, limits)?;

    expect_stable_bytes(&first, b"bc")?;
    expect_stable_bytes(&second, b"bc")?;
    ensure_eq!(program.rule_count().get(), 2)?;
    ensure_eq!(program.once_rule_count().get(), 1)?;

    let inspected = Program::parse(ProgramSource::from_str("a = b # comment\n(start)c=(end)d"))?;
    let mut rules = inspected.rules();
    let first = rules
        .next()
        .ok_or(TestFailure::message("expected first parsed rule"))?;
    let second = rules
        .next()
        .ok_or(TestFailure::message("expected second parsed rule"))?;

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

    ensure_eq!(second.line_number().get(), 2)?;
    ensure_eq!(second.anchor(), RuleAnchor::Start)?;
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
fn canonical_source_reparses_to_the_same_public_rule_view() -> TestResult {
    let program = Program::parse(ProgramSource::from_str(
        "( once ) ( start ) a = ( end ) b # comment",
    ))?;
    let rule = program
        .rules()
        .next()
        .ok_or(TestFailure::message("expected parsed rule"))?;
    let canonical = rule.canonical_source()?;

    let reparsed = Program::parse(ProgramSource::from_bytes(canonical.as_slice()))?;
    let reparsed_rule = reparsed
        .rules()
        .next()
        .ok_or(TestFailure::message("expected reparsed rule"))?;

    ensure_eq!(reparsed.rule_count().get(), 1)?;
    ensure_eq!(reparsed.once_rule_count().get(), 1)?;
    ensure_eq!(reparsed_rule.repeat(), RuleRepeat::Once)?;
    ensure_eq!(reparsed_rule.anchor(), RuleAnchor::Start)?;
    ensure(reparsed_rule.lhs().eq_bytes(b"a"), "expected lhs")?;
    ensure_eq!(
        reparsed_rule.canonical_source()?,
        b"(once)(start)a=(end)b".as_slice(),
    )?;
    Ok(())
}

#[test]
fn stepwise_execution_matches_full_run_and_waits_after_each_rule() -> TestResult {
    let limits = RunLimits::new(
        StepLimit::new(10),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let program = Program::parse(ProgramSource::from_str("a=b\nb=c"))?;
    let mut execution = program.start_execution(RuntimeInput::validate(b"a")?, limits)?;
    ensure_eq!(execution.completed_steps().get(), 0)?;

    match execution.step()? {
        ExecutionStep::Applied { step, rule, state } => {
            ensure_eq!(step.get(), 1)?;
            ensure_eq!(rule.canonical_source()?.as_slice(), b"a=b".as_slice())?;
            ensure_eq!(runtime_view_bytes(state).as_slice(), b"b".as_slice())?;
            ensure_eq!(state.byte_count().get(), 1)?;
        }
        ExecutionStep::Stable { .. } | ExecutionStep::Return { .. } => {
            return Err(TestFailure::message("expected first applied step"));
        }
    }

    match execution.step()? {
        ExecutionStep::Applied { step, rule, state } => {
            ensure_eq!(step.get(), 2)?;
            ensure_eq!(rule.canonical_source()?.as_slice(), b"b=c".as_slice())?;
            ensure_eq!(runtime_view_bytes(state).as_slice(), b"c".as_slice())?;
        }
        ExecutionStep::Stable { .. } | ExecutionStep::Return { .. } => {
            return Err(TestFailure::message("expected second applied step"));
        }
    }

    match execution.step()? {
        ExecutionStep::Stable { steps, state } => {
            ensure_eq!(steps.get(), 2)?;
            ensure_eq!(runtime_view_bytes(state).as_slice(), b"c".as_slice())?;
        }
        ExecutionStep::Applied { .. } | ExecutionStep::Return { .. } => {
            return Err(TestFailure::message("expected stable completion"));
        }
    }
    Ok(())
}

#[test]
fn execution_state_view_exposes_initial_and_current_state() -> TestResult {
    let limits = RunLimits::new(
        StepLimit::new(10),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let program = Program::parse(ProgramSource::from_str("a=b"))?;
    let mut execution = program.start_execution(RuntimeInput::validate(b"a")?, limits)?;

    ensure_eq!(
        runtime_view_bytes(execution.state()).as_slice(),
        b"a".as_slice(),
    )?;

    match execution.step()? {
        ExecutionStep::Applied { state, .. } => {
            ensure_eq!(runtime_view_bytes(state).as_slice(), b"b".as_slice())?;
        }
        ExecutionStep::Stable { .. } | ExecutionStep::Return { .. } => {
            return Err(TestFailure::message("expected applied step"));
        }
    }

    ensure_eq!(
        runtime_view_bytes(execution.state()).as_slice(),
        b"b".as_slice(),
    )
}

#[test]
fn owned_runtime_input_bytes_reborrow_without_revalidation() -> TestResult {
    let input = RuntimeInputBytes::from_slice(b"a=()# ")?;

    ensure_eq!(input.as_bytes(), b"a=()# ".as_slice())?;
    ensure_eq!(input.byte_count().get(), 6)?;
    ensure(!input.is_empty(), "expected non-empty owned input")?;

    let program = Program::parse(ProgramSource::from_str("a=b"))?;
    let result = program.run(
        input.as_input(),
        RunLimits::new(
            DEFAULT_MAX_STEPS,
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        ),
    )?;
    expect_stable_bytes(&result, b"b=()# ")?;
    Ok(())
}

#[test]
fn owned_execution_matches_borrowed_stepwise_execution_and_owns_input() -> TestResult {
    let limits = RunLimits::new(
        StepLimit::new(10),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let source = ProgramSource::from_str("(once)a=b\na=c");
    let borrowed_program = Program::parse(source)?;
    let owned_program = Program::parse(source)?;
    let mut borrowed = borrowed_program.start_execution(RuntimeInput::validate(b"aa")?, limits)?;

    let input = RuntimeInputBytes::from_slice(b"aa")?;
    let mut owned = owned_program.into_execution(input.as_input(), limits)?;
    drop(input);

    ensure_eq!(owned.completed_steps().get(), 0)?;
    ensure_eq!(
        runtime_view_bytes(owned.state()).as_slice(),
        b"aa".as_slice()
    )?;

    ensure_eq!(
        step_signature(owned.step()?)?,
        step_signature(borrowed.step()?)?,
    )?;
    ensure_eq!(
        step_signature(owned.step()?)?,
        step_signature(borrowed.step()?)?,
    )?;
    ensure_eq!(
        step_signature(owned.step()?)?,
        step_signature(borrowed.step()?)?,
    )?;
    ensure_eq!(
        step_signature(owned.step()?)?,
        StepSignature::Stable {
            steps: 2,
            state: b"bc".to_vec(),
        },
    )?;
    Ok(())
}

#[test]
fn owned_execution_preserves_return_terminal_state() -> TestResult {
    let limits = RunLimits::new(
        StepLimit::new(10),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let program = Program::parse(ProgramSource::from_str("a=(return)ok"))?;
    let mut execution = program.into_execution(RuntimeInput::validate(b"a")?, limits)?;

    let first = step_signature(execution.step()?)?;
    ensure_eq!(
        first,
        StepSignature::Return {
            step: 1,
            rule: b"a=(return)ok".to_vec(),
            output: b"ok".to_vec(),
        },
    )?;

    ensure_eq!(step_signature(execution.step()?)?, first)
}

#[test]
fn public_limits_preserve_distinct_step_state_and_return_errors() -> TestResult {
    let step_limited = Program::parse(ProgramSource::from_str("a=b"))?.run(
        RuntimeInput::validate(b"a")?,
        RunLimits::new(
            StepLimit::new(0),
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        ),
    );
    let step_limited = expect_step_limit(expect_run_error(step_limited)?)?;
    ensure_matches(
        matches!(
            step_limited,
            rsaeb::error::LimitError::Step {
                max_steps,
                completed_steps,
                state_len,
            } if max_steps == StepLimit::new(0)
                && completed_steps.get() == 0
                && state_len.get() == 1
        ),
        "expected step limit details",
    )?;

    let state_limited = Program::parse(ProgramSource::from_str("# no executable rules"))?.run(
        RuntimeInput::validate(b"aa")?,
        RunLimits::new(
            StepLimit::new(10),
            StateByteLimit::new(1),
            ReturnByteLimit::new(10),
        ),
    );
    let state_limited = expect_state_limit(expect_run_error(state_limited)?)?;
    ensure_matches(
        matches!(
            state_limited,
            rsaeb::error::LimitError::State {
                context: rsaeb::error::StateLimitContext::Input,
                limit,
                attempted_len,
            } if limit == StateByteLimit::new(1)
                && attempted_len.get() == 2
        ),
        "expected runtime input state limit",
    )?;

    let return_limited = Program::parse(ProgramSource::from_str("a=(return)ok"))?.run(
        RuntimeInput::validate(b"a")?,
        RunLimits::new(
            StepLimit::new(1),
            StateByteLimit::new(10),
            ReturnByteLimit::new(1),
        ),
    );
    let return_limited = expect_run_error(return_limited)?;
    ensure_matches(
        matches!(
            return_limited,
            rsaeb::error::RunError::Limit(rsaeb::error::LimitError::Return {
                limit,
                attempted_len,
            }) if limit == ReturnByteLimit::new(1) && attempted_len.get() == 2
        ),
        "expected return limit details",
    )?;
    Ok(())
}

#[test]
fn runtime_input_public_boundary_accepts_ascii_and_rejects_non_ascii() -> TestResult {
    let input: Vec<u8> = (0x00..=0x7f).collect();
    let program = Program::parse(ProgramSource::from_str("# no executable rules"))?;
    let result = program.run(
        RuntimeInput::validate(&input)?,
        RunLimits::new(
            DEFAULT_MAX_STEPS,
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        ),
    )?;
    expect_stable_bytes(&result, input.as_slice())?;
    ensure_eq!(result.steps().get(), 0)?;

    for byte in 0x80..=0xff {
        ensure(
            RuntimeInput::validate(&[byte]).is_err(),
            "byte should be rejected",
        )?;
    }
    Ok(())
}
