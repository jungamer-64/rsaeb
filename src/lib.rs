//! Library API for the A=B rewrite interpreter.
//!
//! The crate exposes a byte-oriented parser and runtime. Program syntax and
//! runtime input are separate domains:
//!
//! - program code is compact printable ASCII syntax;
//! - comments are ignored bytes after `#`;
//! - runtime input is ASCII data and may contain whitespace/reserved bytes;
//! - program payloads cannot contain whitespace, reserved syntax characters, or
//!   non-ASCII/control bytes.
//!
//! Files, stdout, stderr, argument parsing, and lossy display formatting are
//! intentionally outside this library. The command-line binary can do command-
//! line concerns without coupling them to the interpreter core.

#![no_std]
#![forbid(unsafe_code)]

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

#[cfg(test)]
extern crate std;

mod allocation;
mod bytes;
mod error;
mod parser;
mod program;
mod rule;
mod runtime;
mod trace;

pub use allocation::{AllocationContext, AllocationError};
pub use error::{
    AebError, InputError, ParseError, ParseErrorKind, PayloadKind, RunError, StateSizeError,
    StepLimitError, TracedRunError,
};
pub use program::{run, Program, RunOptions, RunResult, RunTermination, DEFAULT_MAX_STEPS};
pub use rule::{RuleInfo, RulePosition};
pub use trace::{TraceEffect, TraceEvent};

#[cfg(test)]
mod tests {
    use super::*;
    use std::string::{FromUtf8Error, String};

    enum TestFailure {
        Message(&'static str),
        Parse(ParseError),
        Run(RunError),
        Aeb(AebError),
        Utf8(FromUtf8Error),
    }

    impl core::fmt::Debug for TestFailure {
        fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            match self {
                Self::Message(message) => formatter.debug_tuple("Message").field(message).finish(),
                Self::Parse(error) => formatter.debug_tuple("Parse").field(error).finish(),
                Self::Run(error) => formatter.debug_tuple("Run").field(error).finish(),
                Self::Aeb(error) => formatter.debug_tuple("Aeb").field(error).finish(),
                Self::Utf8(error) => formatter.debug_tuple("Utf8").field(error).finish(),
            }
        }
    }

    impl From<ParseError> for TestFailure {
        fn from(value: ParseError) -> Self {
            Self::Parse(value)
        }
    }

    impl From<RunError> for TestFailure {
        fn from(value: RunError) -> Self {
            Self::Run(value)
        }
    }

    impl From<AebError> for TestFailure {
        fn from(value: AebError) -> Self {
            Self::Aeb(value)
        }
    }

    impl From<FromUtf8Error> for TestFailure {
        fn from(value: FromUtf8Error) -> Self {
            Self::Utf8(value)
        }
    }

    type TestResult = Result<(), TestFailure>;

    fn run_source(source: &str, input: &str) -> Result<String, TestFailure> {
        let program = Program::parse(source)?;
        let result = program.run(input.as_bytes(), RunOptions::new(10_000))?;
        Ok(String::from_utf8(result.into_output())?)
    }

    fn expect_parse_error(source: &str) -> Result<ParseError, TestFailure> {
        match Program::parse(source) {
            Ok(_) => Err(TestFailure::Message("expected parse error")),
            Err(error) => Ok(error),
        }
    }

    fn expect_run_error(result: Result<RunResult, RunError>) -> Result<RunError, TestFailure> {
        match result {
            Ok(_) => Err(TestFailure::Message("expected runtime error")),
            Err(error) => Ok(error),
        }
    }

    fn expect_event<'events, 'program>(
        events: &'events [TraceEvent<'program>],
        index: usize,
    ) -> Result<&'events TraceEvent<'program>, TestFailure> {
        events
            .get(index)
            .ok_or(TestFailure::Message("expected trace event"))
    }

    fn expect_step_limit(error: RunError) -> Result<StepLimitError, TestFailure> {
        match error {
            RunError::StepLimit(error) => Ok(error),
            RunError::Input(_) | RunError::Allocation(_) | RunError::StateSize(_) => {
                Err(TestFailure::Message("expected step limit error"))
            }
        }
    }

    fn expect_input_error(error: RunError) -> Result<InputError, TestFailure> {
        match error {
            RunError::Input(error) => Ok(error),
            RunError::Allocation(_) | RunError::StateSize(_) | RunError::StepLimit(_) => {
                Err(TestFailure::Message("expected input error"))
            }
        }
    }

    #[test]
    fn public_free_run_works() -> TestResult {
        let result = run("a=b", b"a", RunOptions::default())?;
        assert_eq!(result.output(), b"b");
        assert_eq!(result.steps(), 1);
        assert!(!result.returned());
        Ok(())
    }

    #[test]
    fn parsed_program_is_reusable_and_once_state_is_per_run() -> TestResult {
        let program = Program::parse("(once)a=b\na=c")?;

        let first = program.run(b"aa", RunOptions::new(10_000))?;
        let second = program.run(b"aa", RunOptions::new(10_000))?;

        assert_eq!(first.output(), b"bc");
        assert_eq!(second.output(), b"bc");
        Ok(())
    }

    #[test]
    fn trace_events_are_emitted_without_core_stderr() -> TestResult {
        let program = Program::parse("a=b\nb=(return)ok")?;
        let mut events = Vec::new();
        let result = program.run_with_trace(b"a", RunOptions::new(10_000), |event| {
            events.push(event);
        })?;

        assert_eq!(result.output(), b"ok");
        assert!(result.returned());
        assert_eq!(events.len(), 3);

        let initial = expect_event(&events, 0)?;
        let first_step = expect_event(&events, 1)?;
        let second_step = expect_event(&events, 2)?;

        assert!(matches!(initial, TraceEvent::Initial { .. }));
        assert_eq!(initial.bytes(), b"a");
        assert_eq!(first_step.bytes(), b"b");
        assert_eq!(second_step.bytes(), b"ok");
        assert!(!first_step.is_return_step());
        assert!(second_step.is_return_step());

        match first_step {
            TraceEvent::Step {
                rule,
                effect: TraceEffect::Continue { state },
                ..
            } => {
                assert_eq!(state.as_slice(), b"b");
                assert_eq!(rule.position().zero_based(), 0);
                assert_eq!(rule.line_number(), 1);
                assert_eq!(rule.compact_source(), b"a=b");
            }
            TraceEvent::Initial { .. } | TraceEvent::Step { .. } => {
                return Err(TestFailure::Message("expected continuing step event"));
            }
        }

        Ok(())
    }

    #[test]
    fn fallible_trace_callback_can_abort_execution() -> TestResult {
        let program = Program::parse("a=b\nb=c")?;
        let result = program.try_run_with_trace(b"a", RunOptions::new(10_000), |_event| {
            Err::<(), _>("trace sink full")
        });

        assert_eq!(result, Err(TracedRunError::Trace("trace sink full")));
        Ok(())
    }

    #[test]
    fn rule_metadata_is_exposed_without_embedding_display_strings_in_trace_events() -> TestResult {
        let program = Program::parse("a = b # comment\n(start)c=(end)d")?;
        let rules = program.rules().collect::<Vec<_>>();

        assert_eq!(rules.len(), 2);

        let first = rules
            .first()
            .ok_or(TestFailure::Message("expected first rule"))?;
        let second = rules
            .get(1)
            .ok_or(TestFailure::Message("expected second rule"))?;

        assert_eq!(first.position().zero_based(), 0);
        assert_eq!(first.line_number(), 1);
        assert_eq!(first.compact_source(), b"a=b");
        assert_eq!(second.position().zero_based(), 1);
        assert_eq!(second.line_number(), 2);
        assert_eq!(second.compact_source(), b"(start)c=(end)d");
        Ok(())
    }

    #[test]
    fn normal_replacement_is_ordered_and_leftmost() -> TestResult {
        let source = "aa=x\na=y";
        assert_eq!(run_source(source, "aaaa")?, "xx");
        Ok(())
    }

    #[test]
    fn start_anchor_matches_only_at_start() -> TestResult {
        let source = "(start)a=x";
        assert_eq!(run_source(source, "aba")?, "xba");
        assert_eq!(run_source(source, "ba")?, "ba");
        Ok(())
    }

    #[test]
    fn end_anchor_matches_only_at_end() -> TestResult {
        let source = "(end)a=x";
        assert_eq!(run_source(source, "aba")?, "abx");
        assert_eq!(run_source(source, "ab")?, "ab");
        Ok(())
    }

    #[test]
    fn runtime_continues_after_anchored_replacement() -> TestResult {
        let source = "(start)a=x\na=y";
        assert_eq!(run_source(source, "aba")?, "xby");

        let source = "(end)a=x\na=y";
        assert_eq!(run_source(source, "aba")?, "ybx");
        Ok(())
    }

    #[test]
    fn move_start_works() -> TestResult {
        let source = "a=(start)x";
        assert_eq!(run_source(source, "ba")?, "xb");
        Ok(())
    }

    #[test]
    fn move_end_works() -> TestResult {
        let source = "a=(end)x";
        assert_eq!(run_source(source, "ba")?, "bx");
        Ok(())
    }

    #[test]
    fn empty_lhs_anywhere_matches_at_start() -> TestResult {
        let source = "(once)=x\n(start)x=(return)ok";
        let result = Program::parse(source)?.run(b"ab", RunOptions::new(2))?;

        assert_eq!(result.output(), b"ok");
        assert_eq!(result.steps(), 2);
        assert!(result.returned());
        Ok(())
    }

    #[test]
    fn empty_lhs_start_and_end_anchors_pick_different_edges() -> TestResult {
        let start_result =
            Program::parse("(once)(start)=x\nxab=(return)start")?.run(b"ab", RunOptions::new(2))?;
        let end_result =
            Program::parse("(once)(end)=x\nabx=(return)end")?.run(b"ab", RunOptions::new(2))?;

        assert_eq!(start_result.output(), b"start");
        assert_eq!(end_result.output(), b"end");
        Ok(())
    }

    #[test]
    fn once_rule_is_used_at_most_once() -> TestResult {
        let source = "(once)a=b\na=c";
        assert_eq!(run_source(source, "aa")?, "bc");
        Ok(())
    }

    #[test]
    fn return_discards_current_state() -> TestResult {
        let source = "aa=(return)ok\na=x";
        assert_eq!(run_source(source, "aabb")?, "ok");
        Ok(())
    }

    #[test]
    fn return_discards_runtime_only_bytes_explicitly() -> TestResult {
        let result = Program::parse("a=(return)x")?.run(b"a=()#c", RunOptions::new(1))?;

        assert_eq!(result.output(), b"x");
        assert!(result.returned());
        Ok(())
    }

    #[test]
    fn empty_lhs_inserts_at_start() -> TestResult {
        let source = "aaa=(return)a\n=a";
        assert_eq!(run_source(source, "")?, "a");
        Ok(())
    }

    #[test]
    fn code_spaces_are_ignored_in_rules() -> TestResult {
        assert_eq!(run_source("a b=bb", "abc")?, "bbc");
        assert_eq!(run_source("a = b", "a")?, "b");
        assert_eq!(run_source("( once ) a = ( end ) b", "ca")?, "cb");
        Ok(())
    }

    #[test]
    fn crlf_source_is_accepted_as_code_whitespace() -> TestResult {
        assert_eq!(run_source("a=b\r\nb=c\r\n", "a")?, "c");
        Ok(())
    }

    #[test]
    fn tab_whitespace_is_ignored_in_code() -> TestResult {
        assert_eq!(run_source("a\tb = c\tc", "ab")?, "cc");
        Ok(())
    }

    #[test]
    fn input_spaces_are_preserved_and_do_not_bridge_matches() -> TestResult {
        assert_eq!(run_source("a= b", "a bc")?, "b bc");
        assert_eq!(run_source("a b=bb", "a bc")?, "a bc");
        assert_eq!(run_source("ab=bb", "a bc")?, "a bc");
        Ok(())
    }

    #[test]
    fn code_cannot_create_or_match_space_even_when_space_is_written_near_rules() -> TestResult {
        assert_eq!(run_source("a= ", "a ")?, " ");
        assert_eq!(run_source(" a = b ", "a")?, "b");
        Ok(())
    }

    #[test]
    fn hash_starts_a_comment() -> TestResult {
        assert_eq!(run_source("a=b#c", "a")?, "b");
        assert_eq!(run_source("#a=b", "a")?, "a");
        assert_eq!(run_source("a=b#コメント内の非ASCIIは許可", "a")?, "b");
        Ok(())
    }

    #[test]
    fn comments_may_contain_non_utf8_bytes_because_the_core_parser_is_byte_oriented() -> TestResult
    {
        let source = b"a=b#\xff\xfe\n";
        let program = Program::parse(source)?;
        let result = program.run(b"a", RunOptions::new(10_000))?;
        assert_eq!(result.output(), b"b");
        Ok(())
    }

    #[test]
    fn code_body_rejects_non_ascii_outside_comments() -> TestResult {
        assert!(Program::parse("a=あ").is_err());
        assert!(Program::parse("あ=b# comment").is_err());
        assert!(Program::parse("a=b#あ").is_ok());

        let error = expect_parse_error("a=あ")?;
        assert_eq!(error.line(), 1);
        assert_eq!(error.column(), Some(3));
        assert!(matches!(
            error.kind(),
            ParseErrorKind::NonAsciiInCode { .. }
        ));
        Ok(())
    }

    #[test]
    fn code_body_rejects_non_printable_ascii_outside_comments() -> TestResult {
        let error = expect_parse_error("a=\0")?;
        assert_eq!(error.line(), 1);
        assert_eq!(error.column(), Some(3));
        assert!(matches!(
            error.kind(),
            ParseErrorKind::NonPrintableAsciiInCode { .. }
        ));

        assert!(Program::parse("a=b#\0").is_ok());
        Ok(())
    }

    #[test]
    fn second_equals_is_a_parse_error_unless_it_is_in_a_comment() -> TestResult {
        let error = expect_parse_error("a=b=c")?;
        assert_eq!(error.column(), Some(4));
        assert!(matches!(error.kind(), ParseErrorKind::MultipleEquals));

        let error = expect_parse_error("a=b =c")?;
        assert_eq!(error.column(), Some(5));
        assert!(matches!(error.kind(), ParseErrorKind::MultipleEquals));

        assert!(Program::parse("a=b#=c").is_ok());
        Ok(())
    }

    #[test]
    fn unsupported_parentheses_are_parse_errors() {
        for source in [
            "a=b(",
            "a=b)",
            "a=b()",
            "a=()",
            "a=b(start)",
            "a=(once)b",
            "a(once)=b",
        ] {
            assert!(
                Program::parse(source).is_err(),
                "source should fail: {source}"
            );
        }

        assert!(Program::parse("(once)(start)a=(end)b").is_ok());
        assert!(Program::parse("a=(return)").is_ok());
    }


    #[test]
    fn empty_program_returns_input_unchanged() -> TestResult {
        let result = Program::parse("")?.run(b"a=()#c", RunOptions::new(0))?;

        assert_eq!(result.output(), b"a=()#c");
        assert_eq!(result.steps(), 0);
        assert_eq!(result.termination(), RunTermination::Stable);
        Ok(())
    }

    #[test]
    fn comment_before_non_ascii_code_hides_it() {
        assert!(Program::parse(b"#\xff\xfe\n").is_ok());
        assert!(Program::parse(b"a=b#\xff\xfe\n").is_ok());
    }

    #[test]
    fn rhs_action_with_empty_payload_is_allowed() -> TestResult {
        assert_eq!(run_source("a=(start)", "ba")?, "b");
        assert_eq!(run_source("a=(end)", "ba")?, "b");
        assert_eq!(run_source("a=(return)", "a")?, "");
        Ok(())
    }

    #[test]
    fn multiline_errors_report_line_and_original_column() -> TestResult {
        let error = expect_parse_error("a=b\nx = y = z")?;

        assert_eq!(error.line(), 2);
        assert_eq!(error.column(), Some(7));
        assert!(matches!(error.kind(), ParseErrorKind::MultipleEquals));
        Ok(())
    }

    #[test]
    fn right_side_action_payload_cannot_start_with_another_action() {
        for source in [
            "a=(start)(end)b",
            "a=(start)(return)b",
            "a=(end)(start)b",
            "a=(return)(start)b",
        ] {
            assert!(
                Program::parse(source).is_err(),
                "source should fail: {source}"
            );
        }
    }

    #[test]
    fn reserved_payload_syntax_errors_keep_original_source_column() -> TestResult {
        let error = expect_parse_error("a = b (")?;
        assert_eq!(error.column(), Some(7));
        assert!(matches!(
            error.kind(),
            ParseErrorKind::ReservedSyntaxInPayload {
                payload_kind: PayloadKind::RightSideData,
                ..
            }
        ));
        Ok(())
    }

    #[test]
    fn code_byte_rejects_every_reserved_syntax_byte_even_if_payload_parser_is_called_directly() {
        for reserved in [b'=', b'#', b'(', b')'] {
            let compact = [CompactByte::new(reserved, 1)];
            let error = Payload::parse(&compact, 1, PayloadKind::RightSideData)
                .expect_err("reserved syntax byte should not become CodeByte");

            assert_eq!(error.column(), Some(1));
            assert!(matches!(
                error.kind(),
                ParseErrorKind::ReservedSyntaxInPayload {
                    payload_kind: PayloadKind::RightSideData,
                    ..
                }
            ));
        }
    }

    #[test]
    fn code_byte_revalidates_compact_bytes_instead_of_trusting_the_previous_phase() {
        let non_ascii = [CompactByte::new(0xff, 1)];
        let non_graphic = [CompactByte::new(b' ', 2)];

        let error = Payload::parse(&non_ascii, 1, PayloadKind::RightSideData)
            .expect_err("non-ASCII byte should not become CodeByte");
        assert!(matches!(error.kind(), ParseErrorKind::NonAsciiInCode { .. }));

        let error = Payload::parse(&non_graphic, 1, PayloadKind::RightSideData)
            .expect_err("non-graphic byte should not become CodeByte");
        assert_eq!(error.column(), Some(2));
        assert!(matches!(
            error.kind(),
            ParseErrorKind::NonPrintableAsciiInCode { .. }
        ));
    }

    #[test]
    fn invalid_left_modifier_order_is_structured() -> TestResult {
        let error = expect_parse_error("(start)(once)a=b")?;
        assert!(matches!(
            error.kind(),
            ParseErrorKind::UnsupportedLeftModifierOrder
        ));
        Ok(())
    }

    #[test]
    fn reserved_input_bytes_are_preserved_but_not_editable_from_code() -> TestResult {
        assert_eq!(run_source("a=b", "a=()#c")?, "b=()#c");
        assert!(
            Program::parse("a=b")?
                .run("aあ".as_bytes(), RunOptions::default())
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn runtime_input_error_is_structured() -> TestResult {
        let error =
            expect_run_error(Program::parse("a=b")?.run("aあ".as_bytes(), RunOptions::default()))?;
        let error = expect_input_error(error)?;

        assert_eq!(error.column(), 2);
        Ok(())
    }

    #[test]
    fn runtime_state_can_hold_reserved_bytes_that_program_payloads_cannot_construct() -> TestResult
    {
        let program = Program::parse("a=b")?;
        assert!(Program::parse("a=(return)(").is_err());
        assert!(Program::parse("a=b)").is_err());

        let result = program.run(b"a=#()", RunOptions::new(10_000))?;
        assert_eq!(String::from_utf8(result.into_output())?, "b=#()");
        Ok(())
    }

    #[test]
    fn one_step_program_succeeds_at_exact_step_limit() -> TestResult {
        let result = Program::parse("a=b")?.run(b"a", RunOptions::new(1))?;

        assert_eq!(result.output(), b"b");
        assert_eq!(result.steps(), 1);
        assert!(!result.returned());
        Ok(())
    }

    #[test]
    fn return_program_succeeds_at_exact_step_limit() -> TestResult {
        let result = Program::parse("a=(return)b")?.run(b"a", RunOptions::new(1))?;

        assert_eq!(result.output(), b"b");
        assert_eq!(result.steps(), 1);
        assert!(result.returned());
        Ok(())
    }

    #[test]
    fn zero_step_limit_succeeds_when_no_rule_matches() -> TestResult {
        let result = Program::parse("a=b")?.run(b"x", RunOptions::new(0))?;

        assert_eq!(result.output(), b"x");
        assert_eq!(result.steps(), 0);
        assert!(!result.returned());
        Ok(())
    }

    #[test]
    fn zero_step_limit_fails_only_when_a_rule_would_apply() -> TestResult {
        let error = expect_run_error(Program::parse("a=b")?.run(b"a", RunOptions::new(0)))?;
        let error = expect_step_limit(error)?;

        assert_eq!(error.max_steps(), 0);
        assert_eq!(error.state(), b"a");
        Ok(())
    }

    #[test]
    fn zero_step_limit_blocks_return_rule_too() -> TestResult {
        let error = expect_run_error(Program::parse("a=(return)b")?.run(b"a", RunOptions::new(0)))?;
        let error = expect_step_limit(error)?;

        assert_eq!(error.max_steps(), 0);
        assert_eq!(error.state(), b"a");
        Ok(())
    }

    #[test]
    fn step_limit_error_keeps_state_as_bytes() -> TestResult {
        let error = expect_run_error(Program::parse("=a")?.run(b"", RunOptions::new(3)))?;
        let error = expect_step_limit(error)?;

        assert_eq!(error.max_steps(), 3);
        assert_eq!(error.state(), b"aaa");
        Ok(())
    }

    #[test]
    fn palindrome_example_returns_true_or_false() -> TestResult {
        let source = "\
b=a|a|
c=a|aa|
a|-=
--=(return)false
(start)a|=(end)-
(start)a=(end)|-
=(return)true";

        assert_eq!(run_source(source, "aba")?, "true");
        assert_eq!(run_source(source, "ab")?, "false");
        Ok(())
    }

    #[test]
    fn runtime_output_preserves_ascii_control_bytes_from_input() -> TestResult {
        let result = Program::parse("a=b")?.run(b"a\0", RunOptions::new(1))?;
        assert_eq!(result.output(), b"b\0");
        Ok(())
    }


    #[test]
    fn traced_final_event_matches_run_result() -> TestResult {
        let program = Program::parse("a=b\nb=(return)c")?;
        let mut events = Vec::new();

        let result = program.run_with_trace(b"a", RunOptions::new(10), |event| {
            events.push(event);
        })?;

        let last = events
            .last()
            .ok_or(TestFailure::Message("expected final trace event"))?;
        assert_eq!(last.bytes(), result.output());
        assert_eq!(events.len(), result.steps() + 1);
        assert!(last.is_return_step());
        Ok(())
    }

    #[test]
    fn compacted_source_and_spaced_source_are_equivalent() -> TestResult {
        let compact = Program::parse("(once)(start)a=(end)b")?;
        let spaced = Program::parse("( once ) ( start ) a = ( end ) b # comment")?;

        let compact_result = compact.run(b"ac", RunOptions::new(10))?;
        let spaced_result = spaced.run(b"ac", RunOptions::new(10))?;

        assert_eq!(compact_result.output(), spaced_result.output());
        Ok(())
    }

    #[test]
    fn internal_code_and_runtime_bytes_are_distinct_domains() -> TestResult {
        let compact = [CompactByte::new(b'a', 1)];
        let payload = Payload::parse(&compact, 1, PayloadKind::LeftSideData)?;
        let state = State::parse_input(b"a=()# ")?;

        assert_eq!(payload.bytes()[0].as_u8(), b'a');
        assert_eq!(state.bytes[0].as_u8(), b'a');
        assert_eq!(state.bytes[1].as_u8(), b'=');
        assert_eq!(state.bytes[2].as_u8(), b'(');
        assert_eq!(state.bytes[5].as_u8(), b' ');
        Ok(())
    }

    #[test]
    fn allocation_contexts_are_publicly_inspectable() {
        let error = AllocationError::new(AllocationContext::TraceSnapshot, 123);
        assert_eq!(error.context(), AllocationContext::TraceSnapshot);
        assert_eq!(error.requested_capacity(), 123);
    }
}
