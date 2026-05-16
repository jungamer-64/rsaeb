mod support;

use rsaeb::error::{LimitError, RunError};
use rsaeb::execution::{
    AppliedExecution, ExecutionStepError, ExecutionTransition, ReturnedExecution, RunningExecution,
    StableExecution,
};
use rsaeb::inspect::{RuleActionView, RuleAnchor, RuleRepeat};
use rsaeb::limits::{
    DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_STEPS, DEFAULT_PARSE_LIMITS,
    ReturnByteLimit, RuntimeStateByteLimit, StepLimit,
};
use rsaeb::{ProgramSource, RunLimits, RunOutcome, RunResult};
use support::{
    TestFailure, TestResult, ensure, ensure_eq, ensure_matches, parse_program, runtime_input,
};

/// Returns stable output bytes when they match `expected`.
///
/// # Errors
///
/// Returns `TestFailure` if the run result is not stable or stable bytes differ.
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

/// Returns return output bytes when they match `expected`.
///
/// # Errors
///
/// Returns `TestFailure` if the run result is not returned or return bytes
/// differ.
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

/// Builds a comparable signature for an applied step.
///
/// # Errors
///
/// Returns `TestFailure` if canonical rule source materialization fails.
fn applied_signature(applied: &AppliedExecution<'_>) -> Result<StepSignature, TestFailure> {
    Ok(StepSignature::Applied {
        step: applied.step().get(),
        rule: applied.rule().canonical_source()?,
        state: runtime_view_bytes(applied.state()),
    })
}

fn stable_signature(stable: &StableExecution<'_>) -> StepSignature {
    StepSignature::Stable {
        steps: stable.steps().get(),
        state: runtime_view_bytes(stable.state()),
    }
}

/// Builds a comparable signature for a returned step.
///
/// # Errors
///
/// Returns `TestFailure` if canonical rule source or return output
/// materialization fails.
fn returned_signature(returned: &ReturnedExecution<'_>) -> Result<StepSignature, TestFailure> {
    Ok(StepSignature::Return {
        step: returned.step().get(),
        rule: returned.rule().canonical_source()?,
        output: returned.output().to_vec()?,
    })
}

/// Runs stepwise execution and collects comparable transition signatures.
///
/// # Errors
///
/// Returns `TestFailure` if a step fails or transition materialization fails.
fn finish_step_signatures(
    mut execution: RunningExecution<'_>,
) -> Result<Vec<StepSignature>, TestFailure> {
    let mut signatures = Vec::new();
    loop {
        match expect_step_transition(execution.step())? {
            ExecutionTransition::Applied(applied) => {
                signatures.push(applied_signature(&applied)?);
                execution = applied.into_running();
            }
            ExecutionTransition::Stable(stable) => {
                signatures.push(stable_signature(&stable));
                return Ok(signatures);
            }
            ExecutionTransition::Returned(returned) => {
                signatures.push(returned_signature(&returned)?);
                return Ok(signatures);
            }
        }
    }
}

/// Returns the expected runtime error.
///
/// # Errors
///
/// Returns `TestFailure` if the result succeeds.
fn expect_run_error<T>(result: Result<T, RunError>) -> Result<RunError, TestFailure> {
    match result {
        Ok(_) => Err(TestFailure::message("expected runtime error")),
        Err(error) => Ok(error),
    }
}

/// Returns the expected step limit error.
///
/// # Errors
///
/// Returns `TestFailure` if `error` is not a step limit error.
fn expect_step_limit(error: RunError) -> Result<LimitError, TestFailure> {
    match error {
        RunError::Limit(error @ LimitError::Step { .. }) => Ok(error),
        RunError::Allocation(_) | RunError::StateSize(_) | RunError::Limit(_) => {
            Err(TestFailure::message("expected step limit error"))
        }
    }
}

/// Returns the expected state limit error.
///
/// # Errors
///
/// Returns `TestFailure` if `error` is not a state limit error.
fn expect_state_limit(error: RunError) -> Result<LimitError, TestFailure> {
    match error {
        RunError::Limit(error @ LimitError::State { .. }) => Ok(error),
        RunError::Allocation(_) | RunError::StateSize(_) | RunError::Limit(_) => {
            Err(TestFailure::message("expected state limit error"))
        }
    }
}

/// Returns the expected successful step transition.
///
/// # Errors
///
/// Returns `TestFailure` if stepping fails.
fn expect_step_transition<'program>(
    result: Result<ExecutionTransition<'program>, ExecutionStepError<'program>>,
) -> Result<ExecutionTransition<'program>, TestFailure> {
    match result {
        Ok(transition) => Ok(transition),
        Err(error) => Err(TestFailure::from(error.into_error())),
    }
}

/// # Errors
///
/// Returns `TestFailure` if public typed boundaries cannot parse or run simple
/// programs.
#[test]
fn public_typed_boundaries_parse_and_run_programs() -> TestResult {
    let limits = RunLimits::new(
        DEFAULT_MAX_STEPS,
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );

    let program = parse_program("a=b")?;
    let input = runtime_input(b"a")?;
    let result = program.run(&input, limits)?;
    expect_stable_bytes(&result, b"b")?;
    ensure_eq!(result.steps().get(), 1)?;

    let program =
        rsaeb::Program::parse(ProgramSource::from_bytes(b"a=b#\xff"), DEFAULT_PARSE_LIMITS)?;
    let input = runtime_input(b"a")?;
    let result = program.run(&input, limits)?;
    expect_stable_bytes(&result, b"b")?;
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if public language whitespace, comments, or actions
/// drift from the expected contract.
#[test]
fn language_whitespace_comments_and_actions_are_public_contract() -> TestResult {
    let limits = RunLimits::new(
        StepLimit::new(10_000),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );

    let program = parse_program("a b=bb")?;
    let result = program.run(&runtime_input(b"abc")?, limits)?;
    expect_stable_bytes(&result, b"bbc")?;

    let program = parse_program("a=b\r\nb=c\r\n")?;
    let result = program.run(&runtime_input(b"a")?, limits)?;
    expect_stable_bytes(&result, b"c")?;

    let program = parse_program("a\tb = c\tc")?;
    let result = program.run(&runtime_input(b"ab")?, limits)?;
    expect_stable_bytes(&result, b"cc")?;

    let program = parse_program("a=b#ignored")?;
    let result = program.run(&runtime_input(b"a")?, limits)?;
    expect_stable_bytes(&result, b"b")?;

    let program = parse_program("#a=b")?;
    let result = program.run(&runtime_input(b"a")?, limits)?;
    expect_stable_bytes(&result, b"a")?;

    let program = parse_program("a=(start)x")?;
    let result = program.run(&runtime_input(b"ba")?, limits)?;
    expect_stable_bytes(&result, b"xb")?;

    let program = parse_program("a=(end)x")?;
    let result = program.run(&runtime_input(b"ba")?, limits)?;
    expect_stable_bytes(&result, b"bx")?;

    let program = parse_program("a=(return)ok")?;
    let result = program.run(&runtime_input(b"a")?, limits)?;
    expect_return_bytes(&result, b"ok")?;
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if rewrite order, anchors, once rules, or runtime-only
/// byte preservation drift from the public contract.
#[test]
fn rewrite_order_anchors_once_and_runtime_only_bytes_are_public_contract() -> TestResult {
    let limits = RunLimits::new(
        StepLimit::new(10_000),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );

    let program = parse_program("aa=x\na=y")?;
    let result = program.run(&runtime_input(b"aaaa")?, limits)?;
    expect_stable_bytes(&result, b"xx")?;

    let program = parse_program("(start)a=x")?;
    let result = program.run(&runtime_input(b"aba")?, limits)?;
    expect_stable_bytes(&result, b"xba")?;

    let program = parse_program("(end)a=x")?;
    let result = program.run(&runtime_input(b"aba")?, limits)?;
    expect_stable_bytes(&result, b"abx")?;

    let program = parse_program("(once)a=b\na=c")?;
    let result = program.run(&runtime_input(b"aa")?, limits)?;
    expect_stable_bytes(&result, b"bc")?;

    let program = parse_program("ab=x")?;
    let result = program.run(&runtime_input(b"a=b")?, limits)?;
    expect_stable_bytes(&result, b"a=b")?;

    let program = parse_program("a= b")?;
    let result = program.run(&runtime_input(b"a bc")?, limits)?;
    expect_stable_bytes(&result, b"b bc")?;
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if parsed programs are not reusable or rule views lose
/// structured public data.
#[test]
fn parsed_program_is_reusable_and_rule_views_are_structured() -> TestResult {
    let limits = RunLimits::new(
        StepLimit::new(10_000),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let program = parse_program("(once)a=b\na=c")?;
    let first = program.run(&runtime_input(b"aa")?, limits)?;
    let second = program.run(&runtime_input(b"aa")?, limits)?;

    expect_stable_bytes(&first, b"bc")?;
    expect_stable_bytes(&second, b"bc")?;
    ensure_eq!(program.rule_count().get(), 2)?;
    ensure_eq!(program.once_rule_count().get(), 1)?;

    let inspected = parse_program("a = b # comment\n(start)c=(end)d")?;
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

/// # Errors
///
/// Returns `TestFailure` if canonical source does not reparse to the same
/// public rule view.
#[test]
fn canonical_source_reparses_to_the_same_public_rule_view() -> TestResult {
    let program = parse_program("( once ) ( start ) a = ( end ) b # comment")?;
    let rule = program
        .rules()
        .next()
        .ok_or(TestFailure::message("expected parsed rule"))?;
    let canonical = rule.canonical_source()?;

    let reparsed = rsaeb::Program::parse(
        ProgramSource::from_bytes(canonical.as_slice()),
        DEFAULT_PARSE_LIMITS,
    )?;
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

/// # Errors
///
/// Returns `TestFailure` if stepwise execution diverges from full-run behavior
/// or fails to pause after each applied rule.
#[test]
fn stepwise_execution_matches_full_run_and_waits_after_each_rule() -> TestResult {
    let limits = RunLimits::new(
        StepLimit::new(10),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let program = parse_program("a=b\nb=c")?;
    let input = runtime_input(b"a")?;
    let execution = program.start_execution(&input, limits)?;
    ensure_eq!(execution.completed_steps().get(), 0)?;

    let execution = match expect_step_transition(execution.step())? {
        ExecutionTransition::Applied(applied) => {
            ensure_eq!(applied.step().get(), 1)?;
            ensure_eq!(
                applied.rule().canonical_source()?.as_slice(),
                b"a=b".as_slice()
            )?;
            ensure_eq!(
                runtime_view_bytes(applied.state()).as_slice(),
                b"b".as_slice()
            )?;
            ensure_eq!(applied.state().byte_count().get(), 1)?;
            applied.into_running()
        }
        ExecutionTransition::Stable(_) | ExecutionTransition::Returned(_) => {
            return Err(TestFailure::message("expected first applied step"));
        }
    };

    let execution = match expect_step_transition(execution.step())? {
        ExecutionTransition::Applied(applied) => {
            ensure_eq!(applied.step().get(), 2)?;
            ensure_eq!(
                applied.rule().canonical_source()?.as_slice(),
                b"b=c".as_slice()
            )?;
            ensure_eq!(
                runtime_view_bytes(applied.state()).as_slice(),
                b"c".as_slice()
            )?;
            applied.into_running()
        }
        ExecutionTransition::Stable(_) | ExecutionTransition::Returned(_) => {
            return Err(TestFailure::message("expected second applied step"));
        }
    };

    match expect_step_transition(execution.step())? {
        ExecutionTransition::Stable(stable) => {
            ensure_eq!(stable.steps().get(), 2)?;
            ensure_eq!(
                runtime_view_bytes(stable.state()).as_slice(),
                b"c".as_slice()
            )?;
        }
        ExecutionTransition::Applied(_) | ExecutionTransition::Returned(_) => {
            return Err(TestFailure::message("expected stable completion"));
        }
    }
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if execution state views do not expose initial and
/// current state bytes correctly.
#[test]
fn execution_state_view_exposes_initial_and_current_state() -> TestResult {
    let limits = RunLimits::new(
        StepLimit::new(10),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let program = parse_program("a=b")?;
    let input = runtime_input(b"a")?;
    let execution = program.start_execution(&input, limits)?;

    ensure_eq!(
        runtime_view_bytes(execution.state()).as_slice(),
        b"a".as_slice(),
    )?;

    let execution = match expect_step_transition(execution.step())? {
        ExecutionTransition::Applied(applied) => {
            ensure_eq!(
                runtime_view_bytes(applied.state()).as_slice(),
                b"b".as_slice()
            )?;
            applied.into_running()
        }
        ExecutionTransition::Stable(_) | ExecutionTransition::Returned(_) => {
            return Err(TestFailure::message("expected applied step"));
        }
    };

    ensure_eq!(
        runtime_view_bytes(execution.state()).as_slice(),
        b"b".as_slice(),
    )
}

/// # Errors
///
/// Returns `TestFailure` if reusable runtime input revalidates or loses owned
/// typed bytes.
#[test]
fn runtime_input_owns_typed_bytes_without_revalidation() -> TestResult {
    let input = runtime_input(b"a=()# ")?;

    ensure_eq!(input.to_vec()?.as_slice(), b"a=()# ".as_slice())?;
    ensure_eq!(input.byte_count().get(), 6)?;
    ensure(!input.is_empty(), "expected non-empty owned input")?;

    let program = parse_program("a=b")?;
    let result = program.run(
        &input,
        RunLimits::new(
            DEFAULT_MAX_STEPS,
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        ),
    )?;
    expect_stable_bytes(&result, b"b=()# ")?;
    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if repeated stepwise executions with the same runtime
/// input diverge.
#[test]
fn reusable_runtime_input_matches_repeated_stepwise_execution() -> TestResult {
    let limits = RunLimits::new(
        StepLimit::new(10),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let program = parse_program("(once)a=b\na=c")?;
    let input = runtime_input(b"aa")?;

    let first = program.start_execution(&input, limits)?;
    let second = program.start_execution(&input, limits)?;

    ensure_eq!(
        finish_step_signatures(first)?,
        [
            StepSignature::Applied {
                step: 1,
                rule: b"(once)a=b".to_vec(),
                state: b"ba".to_vec(),
            },
            StepSignature::Applied {
                step: 2,
                rule: b"a=c".to_vec(),
                state: b"bc".to_vec(),
            },
            StepSignature::Stable {
                steps: 2,
                state: b"bc".to_vec(),
            },
        ],
    )?;
    ensure_eq!(
        finish_step_signatures(second)?,
        finish_step_signatures(program.start_execution(&input, limits)?)?,
    )
}

/// # Errors
///
/// Returns `TestFailure` if public limit errors no longer preserve distinct
/// step, state, and return domains.
#[test]
fn public_limits_preserve_distinct_step_state_and_return_errors() -> TestResult {
    let step_limited = parse_program("a=b")?.run(
        &runtime_input(b"a")?,
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

    let state_limited = parse_program("# no executable rules")?.run(
        &runtime_input(b"aa")?,
        RunLimits::new(
            StepLimit::new(10),
            RuntimeStateByteLimit::new(1),
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
            } if limit == RuntimeStateByteLimit::new(1)
                && attempted_len.get() == 2
        ),
        "expected runtime input state limit",
    )?;

    let return_limited = parse_program("a=(return)ok")?.run(
        &runtime_input(b"a")?,
        RunLimits::new(
            StepLimit::new(1),
            RuntimeStateByteLimit::new(10),
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

/// # Errors
///
/// Returns `TestFailure` if the runtime input public boundary accepts
/// non-ASCII bytes or rejects ASCII bytes.
#[test]
fn runtime_input_public_boundary_accepts_ascii_and_rejects_non_ascii() -> TestResult {
    let input: Vec<u8> = (0x00..=0x7f).collect();
    let program = parse_program("# no executable rules")?;
    let result = program.run(
        &runtime_input(&input)?,
        RunLimits::new(
            DEFAULT_MAX_STEPS,
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        ),
    )?;
    expect_stable_bytes(&result, input.as_slice())?;
    ensure_eq!(result.steps().get(), 0)?;

    for byte in 0x80..=0xff {
        ensure(runtime_input(&[byte]).is_err(), "byte should be rejected")?;
    }
    Ok(())
}
