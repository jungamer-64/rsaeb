use super::budget::RuntimeBudgetState;
use super::matcher::{RuleSearch, find_next_match};
use super::once::OnceStateSet;
use super::rewrite::RewriteScratch;
use super::state::State;
use crate::bytes::{CompactByte, Payload, PayloadSyntax};
use crate::error::{LimitError, PayloadKind, RunError, RunInvariantError, RuntimeInputError};
use crate::execution::{BorrowedFailedRun, BorrowedStepTransition};
use crate::input::{RuntimeInput, RuntimeInputSource};
use crate::limits::{
    DEFAULT_MAX_INPUT_LEN, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, ReturnByteLimit,
    ReturnOutputByteCount, RuntimeInputByteCount, RuntimeInputByteLimit, RuntimeStateByteCount,
    RuntimeStateByteLimit, StepCount, StepLimit,
};
use crate::program::RunOutcome;
use crate::runtime::action::apply_matched_rule;
use crate::test_support::{
    TestFailure, TestResult, TestRunPolicy, ensure_eq, ensure_matches, parse_program, run_seed,
    source_column, source_line_number,
};
use crate::trace::RuntimeStateView;
use alloc::vec::Vec;

fn runtime_view_bytes(view: RuntimeStateView<'_>) -> Vec<u8> {
    view.materialized_bytes().collect()
}

/// Returns the materialized runtime byte at `index`.
///
/// # Errors
///
/// Returns `TestFailure` if the state has no byte at `index`.
fn expect_runtime_byte(state: &State, index: usize) -> Result<u8, TestFailure> {
    state
        .view()
        .materialized_bytes()
        .nth(index)
        .ok_or(TestFailure::message("expected runtime byte"))
}

/// Returns the program payload byte at `index`.
///
/// # Errors
///
/// Returns `TestFailure` if the payload has no byte at `index`.
fn expect_payload_byte(payload: &Payload, index: usize) -> Result<u8, TestFailure> {
    payload
        .bytes()
        .nth(index)
        .ok_or(TestFailure::message("expected payload byte"))
}

/// Returns the expected step limit error.
///
/// # Errors
///
/// Returns `TestFailure` if `error` is not a step limit error.
fn expect_step_limit(error: RunError) -> Result<LimitError, TestFailure> {
    match error {
        RunError::Limit(error @ LimitError::Step { .. }) => Ok(error),
        RunError::Allocation(_)
        | RunError::InternalInvariant(_)
        | RunError::StateSize(_)
        | RunError::Limit(_) => Err(TestFailure::message("expected step limit error")),
    }
}

/// Returns the expected step error.
///
/// # Errors
///
/// Returns `TestFailure` if stepping succeeds.
fn expect_step_error<'program>(
    result: BorrowedStepTransition<'program>,
) -> Result<BorrowedFailedRun<'program>, TestFailure> {
    match result {
        BorrowedStepTransition::Failed(failed) => Ok(failed),
        BorrowedStepTransition::Applied(_)
        | BorrowedStepTransition::Stable(_)
        | BorrowedStepTransition::Returned(_) => Err(TestFailure::message("expected step error")),
    }
}

/// Returns the expected successful step transition.
///
/// # Errors
///
/// Returns `TestFailure` if stepping fails.
fn expect_step_transition<'program>(
    result: BorrowedStepTransition<'program>,
) -> Result<BorrowedStepTransition<'program>, TestFailure> {
    match result {
        BorrowedStepTransition::Failed(failed) => Err(TestFailure::from(failed.into_error())),
        transition => Ok(transition),
    }
}

/// Creates runtime state through the same checked input path as public runs.
///
/// # Errors
///
/// Returns `TestFailure` if input validation fails or the input exceeds runtime
/// state limits.
fn state_from_input_bytes(input: &[u8], limits: TestRunPolicy) -> Result<State, TestFailure> {
    let (input, _) = run_seed(input, limits)?.into_runtime_parts();
    Ok(State::from_input(input))
}

/// # Errors
///
/// Returns `TestFailure` if a failed once-rule commit attempt mutates runtime
/// state before the commit boundary.
#[test]
fn once_rule_failure_preserves_state_before_step_commit() -> TestResult {
    let program = parse_program("(once)a=(return)ok")?;
    let limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(1),
        DEFAULT_MAX_STATE_LEN,
        ReturnByteLimit::new(1),
    );
    let input = run_seed(b"a", limits)?;
    let runtime = program.start_run(input)?;
    let error = expect_step_error(runtime.step())?;
    ensure_eq!(
        error.error(),
        &RunError::Limit(LimitError::Return {
            limit: ReturnByteLimit::new(1),
            attempted_len: ReturnOutputByteCount::new(2),
        }),
    )?;

    ensure_eq!(error.completed_steps(), StepCount::ZERO)?;
    ensure_eq!(
        runtime_view_bytes(error.state()).as_slice(),
        b"a".as_slice()
    )
}

/// # Errors
///
/// Returns `TestFailure` if a step-limit failure commits state or loses the
/// running execution.
#[test]
fn execution_step_limit_failure_preserves_uncommitted_state() -> TestResult {
    let program = parse_program("a=b")?;
    let limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(0),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let no_match_input = run_seed(b"x", limits)?;
    let no_match = program.start_run(no_match_input)?;
    match expect_step_transition(no_match.step())? {
        BorrowedStepTransition::Stable(stable) => {
            ensure_eq!(stable.steps().get(), 0)?;
            ensure_eq!(
                runtime_view_bytes(stable.state()).as_slice(),
                b"x".as_slice()
            )?;
        }
        BorrowedStepTransition::Applied(_)
        | BorrowedStepTransition::Returned(_)
        | BorrowedStepTransition::Failed(_) => {
            return Err(TestFailure::message("expected stable completion"));
        }
    }

    let program = parse_program("a=b")?;
    let would_match_input = run_seed(b"a", limits)?;
    let would_match = program.start_run(would_match_input)?;
    let error = expect_step_error(would_match.step())?;
    ensure_eq!(
        expect_step_limit(error.into_error())?,
        LimitError::Step {
            max_steps: StepLimit::new(0),
            completed_steps: StepCount::ZERO,
            state_len: RuntimeStateByteCount::new(1),
        },
    )?;
    let program = parse_program("a=b")?;
    let would_match = program.start_run(run_seed(b"a", limits)?)?;
    let error = expect_step_error(would_match.step())?;
    ensure_eq!(error.completed_steps(), StepCount::ZERO)?;
    ensure_eq!(
        runtime_view_bytes(error.state()).as_slice(),
        b"a".as_slice(),
    )?;

    ensure_eq!(
        expect_step_limit(error.into_error())?,
        LimitError::Step {
            max_steps: StepLimit::new(0),
            completed_steps: StepCount::ZERO,
            state_len: RuntimeStateByteCount::new(1),
        },
    )
}

/// # Errors
///
/// Returns `TestFailure` if state or return-size limit failures commit state.
#[test]
fn execution_size_limit_failures_preserve_uncommitted_state() -> TestResult {
    let state_limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(1),
        RuntimeStateByteLimit::new(2),
        ReturnByteLimit::new(10),
    );
    let state_program = parse_program("=a")?;
    let state_input = run_seed(b"aa", state_limits)?;
    let state_limited = state_program.start_run(state_input)?;
    let state_error = expect_step_error(state_limited.step())?;
    ensure_eq!(
        state_error.error(),
        &RunError::Limit(LimitError::State {
            limit: RuntimeStateByteLimit::new(2),
            attempted_len: RuntimeStateByteCount::new(3),
        }),
    )?;
    ensure_eq!(state_error.completed_steps(), StepCount::ZERO)?;
    ensure_eq!(
        runtime_view_bytes(state_error.state()).as_slice(),
        b"aa".as_slice(),
    )?;
    ensure_eq!(
        state_error.into_error(),
        RunError::Limit(LimitError::State {
            limit: RuntimeStateByteLimit::new(2),
            attempted_len: RuntimeStateByteCount::new(3),
        }),
    )?;

    let return_limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(1),
        RuntimeStateByteLimit::new(10),
        ReturnByteLimit::new(1),
    );
    let return_program = parse_program("a=(return)ok")?;
    let return_input = run_seed(b"a", return_limits)?;
    let return_limited = return_program.start_run(return_input)?;
    let return_error = expect_step_error(return_limited.step())?;
    ensure_eq!(
        return_error.error(),
        &RunError::Limit(LimitError::Return {
            limit: ReturnByteLimit::new(1),
            attempted_len: ReturnOutputByteCount::new(2),
        }),
    )?;
    ensure_eq!(return_error.completed_steps(), StepCount::ZERO)?;
    ensure_eq!(
        runtime_view_bytes(return_error.state()).as_slice(),
        b"a".as_slice(),
    )?;
    ensure_eq!(
        return_error.into_error(),
        RunError::Limit(LimitError::Return {
            limit: ReturnByteLimit::new(1),
            attempted_len: ReturnOutputByteCount::new(2),
        }),
    )
}

/// # Errors
///
/// Returns `TestFailure` if a return action enters rewrite state-limit
/// accounting instead of the return-output path.
#[test]
fn return_action_bypasses_rewrite_state_mutation_path() -> TestResult {
    let program = parse_program("a=(return)ok")?;
    let limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(1),
        RuntimeStateByteLimit::new(1),
        ReturnByteLimit::new(2),
    );
    let session = program.start_run(run_seed(b"a", limits)?)?;

    match expect_step_transition(session.step())? {
        BorrowedStepTransition::Returned(returned) => {
            let result = returned.into_result();
            ensure_eq!(result.steps().get(), 1)?;
            ensure_matches(
                matches!(
                    result.outcome(),
                    RunOutcome::Return(output) if output.as_slice() == b"ok"
                ),
                "expected return output to bypass rewrite state limit",
            )
        }
        BorrowedStepTransition::Applied(_)
        | BorrowedStepTransition::Stable(_)
        | BorrowedStepTransition::Failed(_) => {
            Err(TestFailure::message("expected return transition"))
        }
    }
}

/// # Errors
///
/// Returns `TestFailure` if a failed `(once)` rewrite commits the once slot.
#[test]
fn once_rewrite_limit_failure_does_not_commit_rule() -> TestResult {
    let program = parse_program("(once)=aa")?;
    let limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(1),
        RuntimeStateByteLimit::new(1),
        DEFAULT_MAX_RETURN_LEN,
    );
    let mut state = state_from_input_bytes(b"a", limits)?;
    let mut budget = RuntimeBudgetState::new(limits.execution());
    let mut scratch = RewriteScratch::new();
    let mut once_states = OnceStateSet::new(program.rule_slice())?;

    let matched = match find_next_match(program.rule_slice(), &once_states, &state)? {
        RuleSearch::Matched(matched) => matched,
        RuleSearch::Stable => {
            return Err(TestFailure::message("expected once rewrite to match"));
        }
    };

    ensure_eq!(
        apply_matched_rule(
            &mut state,
            &mut scratch,
            &mut budget,
            &mut once_states,
            matched
        ),
        Err(RunError::Limit(LimitError::State {
            limit: RuntimeStateByteLimit::new(1),
            attempted_len: RuntimeStateByteCount::new(3),
        })),
    )?;
    ensure_eq!(budget.completed_steps(), StepCount::ZERO)?;
    ensure_eq!(runtime_view_bytes(state.view()).as_slice(), b"a")?;

    ensure_matches(
        matches!(
            find_next_match(program.rule_slice(), &once_states, &state)?,
            RuleSearch::Matched(_)
        ),
        "expected failed once rewrite to remain available",
    )
}

/// # Errors
///
/// Returns `TestFailure` if a failed `(once)` return commits the once slot.
#[test]
fn once_return_limit_failure_does_not_commit_rule() -> TestResult {
    let program = parse_program("(once)a=(return)ok")?;
    let limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(1),
        DEFAULT_MAX_STATE_LEN,
        ReturnByteLimit::new(1),
    );
    let mut state = state_from_input_bytes(b"a", limits)?;
    let mut budget = RuntimeBudgetState::new(limits.execution());
    let mut scratch = RewriteScratch::new();
    let mut once_states = OnceStateSet::new(program.rule_slice())?;

    let matched = match find_next_match(program.rule_slice(), &once_states, &state)? {
        RuleSearch::Matched(matched) => matched,
        RuleSearch::Stable => {
            return Err(TestFailure::message("expected once return to match"));
        }
    };

    ensure_eq!(
        apply_matched_rule(
            &mut state,
            &mut scratch,
            &mut budget,
            &mut once_states,
            matched
        ),
        Err(RunError::Limit(LimitError::Return {
            limit: ReturnByteLimit::new(1),
            attempted_len: ReturnOutputByteCount::new(2),
        })),
    )?;
    ensure_eq!(budget.completed_steps(), StepCount::ZERO)?;
    ensure_eq!(runtime_view_bytes(state.view()).as_slice(), b"a")?;

    ensure_matches(
        matches!(
            find_next_match(program.rule_slice(), &once_states, &state)?,
            RuleSearch::Matched(_)
        ),
        "expected failed once return to remain available",
    )
}

/// # Errors
///
/// Returns `TestFailure` if once-state construction can be detached from the
/// parsed rule table.
#[test]
fn once_state_set_is_constructed_from_the_rule_table() -> TestResult {
    let program = parse_program("(once)a=b")?;
    let once_states = OnceStateSet::new(program.rule_slice())?;
    let state = state_from_input_bytes(
        b"a",
        TestRunPolicy::new(
            DEFAULT_MAX_INPUT_LEN,
            StepLimit::new(1),
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        ),
    )?;

    ensure_matches(
        matches!(
            find_next_match(program.rule_slice(), &once_states, &state)?,
            RuleSearch::Matched(_)
        ),
        "expected rule-aligned once state to keep the rule available",
    )
}

/// # Errors
///
/// Returns `TestFailure` if a parsed once slot missing from runtime state is
/// treated as an ordinary stable run.
#[test]
fn missing_once_rule_state_is_runtime_invariant_error() -> TestResult {
    let program = parse_program("(once)a=b")?;
    let empty_rules: &[crate::rule::Rule] = &[];
    let once_states = OnceStateSet::new(empty_rules)?;
    let state = state_from_input_bytes(
        b"a",
        TestRunPolicy::new(
            DEFAULT_MAX_INPUT_LEN,
            StepLimit::new(1),
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        ),
    )?;

    let error = find_next_match(program.rule_slice(), &once_states, &state);
    ensure_matches(
        matches!(
            error,
            Err(RunError::InternalInvariant(
                RunInvariantError::MissingOnceRuleState {
                    rule,
                    available_slots
                }
            )) if rule.number().get() == 1 && available_slots.get() == 0
        ),
        "expected missing once-state slot invariant error",
    )
}

/// # Errors
///
/// Returns `TestFailure` if runtime input errors lose structured boundary
/// information.
#[test]
fn runtime_input_error_is_structured_at_the_runtime_boundary() -> TestResult {
    let Err(error) = RuntimeInput::validate(
        RuntimeInputSource::from_bytes(b"abc"),
        TestRunPolicy::new(
            RuntimeInputByteLimit::new(2),
            StepLimit::new(10),
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        )
        .input(),
    ) else {
        return Err(TestFailure::message("expected input limit error"));
    };

    ensure_eq!(
        error,
        RuntimeInputError::InputLimit {
            limit: RuntimeInputByteLimit::new(2),
            attempted_len: RuntimeInputByteCount::new(3),
        },
    )?;

    let Err(error) = RuntimeInput::validate(
        RuntimeInputSource::from_bytes("a\u{80}".as_bytes()),
        TestRunPolicy::new(
            RuntimeInputByteLimit::new(1),
            StepLimit::new(10),
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        )
        .input(),
    ) else {
        return Err(TestFailure::message(
            "expected input limit before byte error",
        ));
    };

    ensure_eq!(
        error,
        RuntimeInputError::InputLimit {
            limit: RuntimeInputByteLimit::new(1),
            attempted_len: RuntimeInputByteCount::new(3),
        },
    )?;

    let Err(error) = RuntimeInput::validate(
        RuntimeInputSource::from_bytes("a\u{80}".as_bytes()),
        TestRunPolicy::default().input(),
    ) else {
        return Err(TestFailure::message("expected input error"));
    };

    ensure_matches(
        matches!(
            error,
            RuntimeInputError::NonAscii { column, .. } if column.get() == 2
        ),
        "expected non-ASCII input error at the original column",
    )
}

/// # Errors
///
/// Returns `TestFailure` if executable payload bytes and runtime-only bytes are
/// not kept in distinct domains.
#[test]
fn internal_code_and_runtime_bytes_are_distinct_domains() -> TestResult {
    let compact = [CompactByte::new(b'a', source_column(1)?)];
    let payload = PayloadSyntax::new(&compact, source_line_number(1)?, PayloadKind::LeftSideData)
        .validate()?;
    let limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(10_000),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let (input, _) = run_seed(b"a=()# ", limits)?.into_runtime_parts();
    let state = State::from_input(input);

    ensure_eq!(expect_payload_byte(&payload, 0)?, b'a')?;
    ensure_eq!(expect_runtime_byte(&state, 0)?, b'a')?;
    ensure_eq!(expect_runtime_byte(&state, 1)?, b'=')?;
    ensure_eq!(expect_runtime_byte(&state, 2)?, b'(')?;
    ensure_eq!(expect_runtime_byte(&state, 5)?, b' ')?;

    let program = parse_program("a=b")?;
    let result = program.run(run_seed(b"a=()# ", limits)?)?;
    ensure_matches(
        matches!(
            result.outcome(),
            RunOutcome::Stable(output) if output.as_slice() == b"b=()# "
        ),
        "expected rewrite to leave runtime-only input bytes materialized but unmatched",
    )
}

/// # Errors
///
/// Returns `TestFailure` if a consumed `(once)` rule can be matched again
/// before later rules are considered.
#[test]
fn once_rule_commit_proof_allows_only_one_successful_application() -> TestResult {
    let program = parse_program("(once)a=a\na=b")?;
    let limits = TestRunPolicy::new(
        DEFAULT_MAX_INPUT_LEN,
        StepLimit::new(10),
        DEFAULT_MAX_STATE_LEN,
        DEFAULT_MAX_RETURN_LEN,
    );
    let result = program.run(run_seed(b"a", limits)?)?;

    ensure_eq!(result.steps().get(), 2)?;
    ensure_matches(
        matches!(
            result.outcome(),
            RunOutcome::Stable(output) if output.as_slice() == b"b"
        ),
        "expected consumed once rule to give the later rule a chance",
    )
}

/// # Errors
///
/// Returns `TestFailure` if rewrite action variants lose their placement
/// semantics after being prepared from matched state spans.
#[test]
fn rewrite_action_variants_preserve_runtime_placement() -> TestResult {
    for (source, input, expected) in [
        ("a=x", b"ab".as_slice(), b"xb".as_slice()),
        ("b=(start)x", b"ab".as_slice(), b"xa".as_slice()),
        ("a=(end)x", b"ab".as_slice(), b"bx".as_slice()),
    ] {
        let limits = TestRunPolicy::new(
            DEFAULT_MAX_INPUT_LEN,
            StepLimit::new(1),
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        );
        let result = parse_program(source)?.run(run_seed(input, limits)?)?;

        ensure_matches(
            matches!(
                result.outcome(),
                RunOutcome::Stable(output) if output.as_slice() == expected
            ),
            "expected rewrite action variant to preserve placement",
        )?;
    }

    Ok(())
}

/// # Errors
///
/// Returns `TestFailure` if empty payload matches lose their start/end span
/// placement while deriving matched length from the validated range.
#[test]
fn empty_payload_matches_keep_anchor_specific_span_placement() -> TestResult {
    for (source, expected) in [
        ("=x", b"xab".as_slice()),
        ("(start)=x", b"xab".as_slice()),
        ("(end)=x", b"abx".as_slice()),
    ] {
        let program = parse_program(source)?;
        let limits = TestRunPolicy::new(
            DEFAULT_MAX_INPUT_LEN,
            StepLimit::new(1),
            DEFAULT_MAX_STATE_LEN,
            DEFAULT_MAX_RETURN_LEN,
        );
        let session = program.start_run(run_seed(b"ab", limits)?)?;

        match expect_step_transition(session.step())? {
            BorrowedStepTransition::Applied(applied) => {
                ensure_eq!(runtime_view_bytes(applied.state()).as_slice(), expected)?;
            }
            BorrowedStepTransition::Stable(_)
            | BorrowedStepTransition::Returned(_)
            | BorrowedStepTransition::Failed(_) => {
                return Err(TestFailure::message("expected one empty-payload rewrite"));
            }
        }
    }

    Ok(())
}
