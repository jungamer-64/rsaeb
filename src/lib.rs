//! Library API for the A=B rewrite interpreter.
//!
//! The public API is intentionally small:
//!
//! - [`Program`] is a parsed, reusable rewrite program.
//! - [`RunOptions`] controls one execution.
//! - [`RunResult`] owns the output bytes and execution metadata.
//! - [`AebError`] reports parse, input, and runtime failures.
//!
//! Program syntax bytes and runtime input bytes are separate domains. Program
//! payload bytes cannot contain whitespace, reserved syntax characters, or
//! non-ASCII data. Runtime input may contain any ASCII byte and those bytes are
//! preserved unless editable adjacent data is rewritten.

use std::error::Error;
use std::fmt;

/// Default maximum number of rewrite steps for one execution.
pub const DEFAULT_MAX_STEPS: usize = 1_000_000;

const TOK_ONCE: &[u8] = b"(once)";
const TOK_START: &[u8] = b"(start)";
const TOK_END: &[u8] = b"(end)";
const TOK_RETURN: &[u8] = b"(return)";

/// Parses and runs a source string in one call.
///
/// Use [`Program::parse`] when the same program will be executed multiple times.
pub fn run(
    source: &str,
    input: impl AsRef<[u8]>,
    options: RunOptions,
) -> Result<RunResult, AebError> {
    Program::parse(source)?.run(input, options)
}

/// Execution options for one runtime invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunOptions {
    /// Maximum number of rewrite steps before execution fails.
    pub max_steps: usize,
}

impl RunOptions {
    /// Creates options with an explicit step limit.
    #[must_use]
    pub const fn new(max_steps: usize) -> Self {
        Self { max_steps }
    }
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            max_steps: DEFAULT_MAX_STEPS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Anchor {
    Anywhere,
    Start,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuleRepeat {
    Always,
    Once,
}

impl RuleRepeat {
    fn is_once(self) -> bool {
        matches!(self, Self::Once)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeRuleState {
    Fresh,
    Consumed,
}

impl RuntimeRuleState {
    fn is_consumed(self) -> bool {
        matches!(self, Self::Consumed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CodeByte(u8);

impl CodeByte {
    fn parse(
        byte: u8,
        line_number: usize,
        context: &str,
        compact_column: usize,
    ) -> Result<Self, AebError> {
        if !byte.is_ascii() {
            return Err(AebError::Parse {
                line: line_number,
                message: format!(
                    "non-ASCII byte 0x{byte:02x} in {context} at compact column {compact_column}",
                ),
            });
        }

        if byte.is_ascii_whitespace() {
            return Err(AebError::Parse {
                line: line_number,
                message: format!(
                    "whitespace byte 0x{byte:02x} cannot be represented as \
                     {context} at compact column {compact_column}",
                ),
            });
        }

        if is_reserved_code_byte(byte) {
            return Err(AebError::Parse {
                line: line_number,
                message: format!(
                    "reserved character '{}' cannot be represented as \
                     {context} at compact column {compact_column}",
                    byte as char,
                ),
            });
        }

        Ok(Self(byte))
    }

    fn as_u8(self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RuntimeByte(u8);

impl RuntimeByte {
    fn parse_input(byte: u8, zero_based_column: usize) -> Result<Self, AebError> {
        if !byte.is_ascii() {
            return Err(AebError::Input {
                message: format!(
                    "non-ASCII byte 0x{byte:02x} at column {}",
                    zero_based_column + 1,
                ),
            });
        }

        Ok(Self(byte))
    }

    fn from_code(byte: CodeByte) -> Self {
        Self(byte.as_u8())
    }

    fn as_u8(self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Payload {
    bytes: Vec<CodeByte>,
}

impl Payload {
    fn parse(input: &[u8], line_number: usize, context: &str) -> Result<Self, AebError> {
        let bytes = input
            .iter()
            .copied()
            .enumerate()
            .map(|(zero_based_column, byte)| {
                CodeByte::parse(byte, line_number, context, zero_based_column + 1)
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self { bytes })
    }

    fn len(&self) -> usize {
        self.bytes.len()
    }

    fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    fn iter(&self) -> impl Iterator<Item = CodeByte> + '_ {
        self.bytes.iter().copied()
    }

    fn to_runtime_bytes(&self) -> Vec<RuntimeByte> {
        self.iter().map(RuntimeByte::from_code).collect()
    }

    fn to_vec_u8(&self) -> Vec<u8> {
        self.iter().map(CodeByte::as_u8).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct State {
    bytes: Vec<RuntimeByte>,
}

impl State {
    fn parse_input(input: &[u8]) -> Result<Self, AebError> {
        let bytes = input
            .iter()
            .copied()
            .enumerate()
            .map(|(zero_based_column, byte)| RuntimeByte::parse_input(byte, zero_based_column))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self { bytes })
    }

    fn len(&self) -> usize {
        self.bytes.len()
    }

    fn starts_with_payload(&self, payload: &Payload) -> bool {
        if payload.len() > self.len() {
            return false;
        }

        self.bytes
            .iter()
            .copied()
            .zip(payload.iter())
            .take(payload.len())
            .all(|(state_byte, payload_byte)| state_byte.as_u8() == payload_byte.as_u8())
    }

    fn ends_with_payload(&self, payload: &Payload) -> bool {
        if payload.len() > self.len() {
            return false;
        }

        let start = self.len() - payload.len();
        self.matches_payload_at(start, payload)
    }

    fn find_payload(&self, payload: &Payload) -> Option<usize> {
        if payload.is_empty() {
            return Some(0);
        }

        if payload.len() > self.len() {
            return None;
        }

        (0..=self.len() - payload.len())
            .find(|&position| self.matches_payload_at(position, payload))
    }

    fn matches_payload_at(&self, position: usize, payload: &Payload) -> bool {
        if position + payload.len() > self.len() {
            return false;
        }

        self.bytes[position..position + payload.len()]
            .iter()
            .copied()
            .zip(payload.iter())
            .all(|(state_byte, payload_byte)| state_byte.as_u8() == payload_byte.as_u8())
    }

    fn replace_at(&self, position: usize, lhs_len: usize, rhs: &Payload) -> Self {
        let mut bytes = Vec::with_capacity(self.len() - lhs_len + rhs.len());
        bytes.extend_from_slice(&self.bytes[..position]);
        bytes.extend(rhs.to_runtime_bytes());
        bytes.extend_from_slice(&self.bytes[position + lhs_len..]);
        Self { bytes }
    }

    fn move_start_at(&self, position: usize, lhs_len: usize, rhs: &Payload) -> Self {
        let mut bytes = Vec::with_capacity(self.len() - lhs_len + rhs.len());
        bytes.extend(rhs.to_runtime_bytes());
        bytes.extend_from_slice(&self.bytes[..position]);
        bytes.extend_from_slice(&self.bytes[position + lhs_len..]);
        Self { bytes }
    }

    fn move_end_at(&self, position: usize, lhs_len: usize, rhs: &Payload) -> Self {
        let mut bytes = Vec::with_capacity(self.len() - lhs_len + rhs.len());
        bytes.extend_from_slice(&self.bytes[..position]);
        bytes.extend_from_slice(&self.bytes[position + lhs_len..]);
        bytes.extend(rhs.to_runtime_bytes());
        Self { bytes }
    }

    fn to_vec_u8(&self) -> Vec<u8> {
        self.bytes.iter().copied().map(RuntimeByte::as_u8).collect()
    }

    fn into_vec_u8(self) -> Vec<u8> {
        self.bytes.into_iter().map(RuntimeByte::as_u8).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
    Replace(Payload),
    MoveStart(Payload),
    MoveEnd(Payload),
    Return(Payload),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Rule {
    line_number: usize,
    source: String,
    repeat: RuleRepeat,
    anchor: Anchor,
    lhs: Payload,
    action: Action,
}

/// Parsed A=B rewrite program.
///
/// A parsed program is immutable and reusable. Per-run `(once)` state lives in
/// the runtime invocation, not in this value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    rules: Vec<Rule>,
}

impl Program {
    /// Parses program source into a reusable program value.
    pub fn parse(source: &str) -> Result<Self, AebError> {
        parse_program_impl(source)
    }

    /// Returns the number of executable rules in the parsed program.
    #[must_use]
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Runs this program with the given input bytes.
    pub fn run(&self, input: impl AsRef<[u8]>, options: RunOptions) -> Result<RunResult, AebError> {
        Runtime::new(self, input.as_ref())?.run(options.max_steps)
    }

    /// Runs this program and emits trace events.
    ///
    /// The core interpreter still owns no stderr/stdout behavior. The CLI uses
    /// this hook to print traces; library users can collect or format events in
    /// their own domain instead of inheriting yet another hard-coded human
    /// nozzle.
    pub fn run_with_trace<F>(
        &self,
        input: impl AsRef<[u8]>,
        options: RunOptions,
        trace: F,
    ) -> Result<RunResult, AebError>
    where
        F: FnMut(TraceEvent),
    {
        Runtime::new(self, input.as_ref())?.run_with_trace(options.max_steps, trace)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Runtime<'program> {
    program: &'program Program,
    state: State,
    steps: usize,
    rule_states: Box<[RuntimeRuleState]>,
}

impl<'program> Runtime<'program> {
    fn new(program: &'program Program, input: &[u8]) -> Result<Self, AebError> {
        Ok(Self {
            program,
            state: State::parse_input(input)?,
            steps: 0,
            rule_states: vec![RuntimeRuleState::Fresh; program.rules.len()].into_boxed_slice(),
        })
    }

    fn run(self, max_steps: usize) -> Result<RunResult, AebError> {
        self.run_impl(max_steps, None::<fn(TraceEvent)>)
    }

    fn run_with_trace<F>(self, max_steps: usize, trace: F) -> Result<RunResult, AebError>
    where
        F: FnMut(TraceEvent),
    {
        self.run_impl(max_steps, Some(trace))
    }

    fn run_impl<F>(mut self, max_steps: usize, mut trace: Option<F>) -> Result<RunResult, AebError>
    where
        F: FnMut(TraceEvent),
    {
        emit_trace(
            &mut trace,
            TraceEvent::Initial {
                state: self.state.to_vec_u8(),
            },
        );

        loop {
            if self.steps >= max_steps {
                return Err(AebError::StepLimit {
                    max_steps,
                    state: self.state.into_vec_u8(),
                });
            }

            let Some((rule_index, position)) = self.find_next_match() else {
                return Ok(RunResult {
                    output: self.state.into_vec_u8(),
                    steps: self.steps,
                    returned: false,
                });
            };

            if let Some(result) = self.apply_rule(rule_index, position, &mut trace) {
                return Ok(result);
            }
        }
    }

    fn find_next_match(&self) -> Option<(usize, usize)> {
        for (rule_index, rule) in self.program.rules.iter().enumerate() {
            if self.is_rule_consumed(rule_index, rule) {
                continue;
            }

            if let Some(position) = find_match(&self.state, rule) {
                return Some((rule_index, position));
            }
        }

        None
    }

    fn is_rule_consumed(&self, rule_index: usize, rule: &Rule) -> bool {
        rule.repeat.is_once() && self.rule_states[rule_index].is_consumed()
    }

    fn consume_rule_if_needed(&mut self, rule_index: usize) {
        let rule = &self.program.rules[rule_index];

        if rule.repeat.is_once() {
            self.rule_states[rule_index] = RuntimeRuleState::Consumed;
        }
    }

    fn apply_rule<F>(
        &mut self,
        rule_index: usize,
        position: usize,
        trace: &mut Option<F>,
    ) -> Option<RunResult>
    where
        F: FnMut(TraceEvent),
    {
        self.consume_rule_if_needed(rule_index);

        let rule = &self.program.rules[rule_index];
        let lhs_len = rule.lhs.len();

        match &rule.action {
            Action::Replace(rhs) => {
                self.state = self.state.replace_at(position, lhs_len, rhs);
                self.steps += 1;

                emit_trace(
                    trace,
                    TraceEvent::Step {
                        step: self.steps,
                        line_number: rule.line_number,
                        source: rule.source.clone(),
                        output: self.state.to_vec_u8(),
                        returned: false,
                    },
                );
            }
            Action::MoveStart(rhs) => {
                self.state = self.state.move_start_at(position, lhs_len, rhs);
                self.steps += 1;

                emit_trace(
                    trace,
                    TraceEvent::Step {
                        step: self.steps,
                        line_number: rule.line_number,
                        source: rule.source.clone(),
                        output: self.state.to_vec_u8(),
                        returned: false,
                    },
                );
            }
            Action::MoveEnd(rhs) => {
                self.state = self.state.move_end_at(position, lhs_len, rhs);
                self.steps += 1;

                emit_trace(
                    trace,
                    TraceEvent::Step {
                        step: self.steps,
                        line_number: rule.line_number,
                        source: rule.source.clone(),
                        output: self.state.to_vec_u8(),
                        returned: false,
                    },
                );
            }
            Action::Return(output) => {
                self.steps += 1;
                let output = output.to_vec_u8();

                emit_trace(
                    trace,
                    TraceEvent::Step {
                        step: self.steps,
                        line_number: rule.line_number,
                        source: rule.source.clone(),
                        output: output.clone(),
                        returned: true,
                    },
                );

                return Some(RunResult {
                    output,
                    steps: self.steps,
                    returned: true,
                });
            }
        }

        None
    }
}

fn emit_trace<F>(trace: &mut Option<F>, event: TraceEvent)
where
    F: FnMut(TraceEvent),
{
    if let Some(trace) = trace.as_mut() {
        trace(event);
    }
}

fn find_match(state: &State, rule: &Rule) -> Option<usize> {
    match rule.anchor {
        Anchor::Anywhere => state.find_payload(&rule.lhs),
        Anchor::Start => state.starts_with_payload(&rule.lhs).then_some(0),
        Anchor::End => {
            if state.ends_with_payload(&rule.lhs) {
                Some(state.len().saturating_sub(rule.lhs.len()))
            } else {
                None
            }
        }
    }
}

/// Result of one program execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunResult {
    output: Vec<u8>,
    steps: usize,
    returned: bool,
}

impl RunResult {
    /// Final output bytes.
    #[must_use]
    pub fn output(&self) -> &[u8] {
        &self.output
    }

    /// Consumes the result and returns final output bytes.
    #[must_use]
    pub fn into_output(self) -> Vec<u8> {
        self.output
    }

    /// Final output rendered lossily as UTF-8.
    ///
    /// Valid input is restricted to ASCII, so this is mainly a convenience for
    /// callers that want `String` output.
    #[must_use]
    pub fn output_lossy_string(&self) -> String {
        String::from_utf8_lossy(&self.output).into_owned()
    }

    /// Number of rewrite steps applied.
    #[must_use]
    pub fn steps(&self) -> usize {
        self.steps
    }

    /// Whether execution stopped by `(return)`.
    #[must_use]
    pub fn returned(&self) -> bool {
        self.returned
    }
}

/// Trace event emitted by [`Program::run_with_trace`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceEvent {
    /// Initial runtime state before any rewrite step.
    Initial { state: Vec<u8> },
    /// One applied rule.
    Step {
        step: usize,
        line_number: usize,
        source: String,
        output: Vec<u8>,
        returned: bool,
    },
}

impl TraceEvent {
    /// Output/state bytes carried by this event.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::Initial { state } => state,
            Self::Step { output, .. } => output,
        }
    }

    /// Output/state rendered lossily as UTF-8.
    #[must_use]
    pub fn lossy_string(&self) -> String {
        String::from_utf8_lossy(self.bytes()).into_owned()
    }
}

/// Interpreter error.
#[derive(Debug)]
pub enum AebError {
    /// Source program parse error.
    Parse { line: usize, message: String },
    /// Runtime input is invalid.
    Input { message: String },
    /// Execution exceeded the configured step limit.
    StepLimit { max_steps: usize, state: Vec<u8> },
    /// I/O error kept for CLI/library convenience when callers choose to map
    /// filesystem loading into this error type.
    Io(std::io::Error),
}

impl AebError {
    /// Returns the line number for parse errors.
    #[must_use]
    pub fn parse_line(&self) -> Option<usize> {
        match self {
            Self::Parse { line, .. } => Some(*line),
            Self::Input { .. } | Self::StepLimit { .. } | Self::Io(_) => None,
        }
    }
}

impl fmt::Display for AebError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AebError::Parse { line, message } => {
                write!(f, "parse error at line {line}: {message}")
            }
            AebError::Input { message } => write!(f, "input error: {message}"),
            AebError::StepLimit { max_steps, state } => write!(
                f,
                "step limit exceeded after {max_steps} steps; state: {}",
                String::from_utf8_lossy(state),
            ),
            AebError::Io(error) => write!(f, "io error: {error}"),
        }
    }
}

impl Error for AebError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Parse { .. } | Self::Input { .. } | Self::StepLimit { .. } => None,
        }
    }
}

impl From<std::io::Error> for AebError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

fn is_reserved_code_byte(byte: u8) -> bool {
    matches!(byte, b'=' | b'#' | b'(' | b')')
}

fn parse_program_impl(source: &str) -> Result<Program, AebError> {
    let mut rules = Vec::new();

    for (zero_based_line, raw_line) in source.lines().enumerate() {
        let line_number = zero_based_line + 1;
        let code_line = parse_code_line(raw_line, line_number)?;
        let code = strip_code_whitespace(&code_line);

        if code.is_empty() {
            continue;
        }

        let equals_count = code.iter().filter(|&&byte| byte == b'=').count();

        if equals_count == 0 {
            return Err(AebError::Parse {
                line: line_number,
                message: "missing '='".to_string(),
            });
        }

        if equals_count > 1 {
            return Err(AebError::Parse {
                line: line_number,
                message: "multiple '=' characters are not allowed".to_string(),
            });
        }

        let equals_position = code
            .iter()
            .position(|&byte| byte == b'=')
            .expect("equals_count checked above");
        let lhs_code = &code[..equals_position];
        let rhs_code = &code[equals_position + 1..];
        let (repeat, anchor, lhs) = parse_lhs(lhs_code, line_number)?;
        let action = parse_rhs(rhs_code, line_number)?;

        rules.push(Rule {
            line_number,
            source: String::from_utf8_lossy(&code_line).trim().to_string(),
            repeat,
            anchor,
            lhs,
            action,
        });
    }

    Ok(Program { rules })
}

fn parse_code_line(raw_line: &str, line_number: usize) -> Result<Vec<u8>, AebError> {
    let raw_bytes = raw_line.as_bytes();
    let code_bytes = match raw_bytes.iter().position(|&byte| byte == b'#') {
        Some(comment_start) => &raw_bytes[..comment_start],
        None => raw_bytes,
    };

    if let Some((zero_based_column, byte)) = code_bytes
        .iter()
        .copied()
        .enumerate()
        .find(|(_, byte)| !byte.is_ascii())
    {
        return Err(AebError::Parse {
            line: line_number,
            message: format!(
                "non-ASCII byte 0x{byte:02x} in code at column {}",
                zero_based_column + 1,
            ),
        });
    }

    Ok(code_bytes.to_vec())
}

fn strip_code_whitespace(input: &[u8]) -> Vec<u8> {
    input
        .iter()
        .copied()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect()
}

fn parse_lhs(
    mut input: &[u8],
    line_number: usize,
) -> Result<(RuleRepeat, Anchor, Payload), AebError> {
    let mut repeat = RuleRepeat::Always;

    if input.starts_with(TOK_ONCE) {
        repeat = RuleRepeat::Once;
        input = &input[TOK_ONCE.len()..];
    }

    let anchor = if input.starts_with(TOK_START) {
        input = &input[TOK_START.len()..];
        Anchor::Start
    } else if input.starts_with(TOK_END) {
        input = &input[TOK_END.len()..];
        Anchor::End
    } else {
        Anchor::Anywhere
    };

    if input.starts_with(TOK_ONCE) || input.starts_with(TOK_START) || input.starts_with(TOK_END) {
        return Err(AebError::Parse {
            line: line_number,
            message: "duplicated or unsupported left-side modifier order".to_string(),
        });
    }

    let lhs = Payload::parse(input, line_number, "left-side data")?;
    Ok((repeat, anchor, lhs))
}

fn parse_rhs(input: &[u8], line_number: usize) -> Result<Action, AebError> {
    if input.starts_with(TOK_START) {
        let payload = Payload::parse(
            &input[TOK_START.len()..],
            line_number,
            "right-side move-to-start payload",
        )?;
        Ok(Action::MoveStart(payload))
    } else if input.starts_with(TOK_END) {
        let payload = Payload::parse(
            &input[TOK_END.len()..],
            line_number,
            "right-side move-to-end payload",
        )?;
        Ok(Action::MoveEnd(payload))
    } else if input.starts_with(TOK_RETURN) {
        let payload = Payload::parse(
            &input[TOK_RETURN.len()..],
            line_number,
            "right-side return payload",
        )?;
        Ok(Action::Return(payload))
    } else {
        let payload = Payload::parse(input, line_number, "right-side data")?;
        Ok(Action::Replace(payload))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_source(source: &str, input: &str) -> String {
        let program = Program::parse(source).unwrap();
        let result = program
            .run(input.as_bytes(), RunOptions::new(10_000))
            .unwrap();

        String::from_utf8(result.into_output()).unwrap()
    }

    #[test]
    fn public_free_run_works() {
        let result = run("a=b", b"a", RunOptions::default()).unwrap();
        assert_eq!(result.output(), b"b");
        assert_eq!(result.steps(), 1);
        assert!(!result.returned());
    }

    #[test]
    fn parsed_program_is_reusable_and_once_state_is_per_run() {
        let program = Program::parse("(once)a=b\na=c").unwrap();

        let first = program.run(b"aa", RunOptions::new(10_000)).unwrap();
        let second = program.run(b"aa", RunOptions::new(10_000)).unwrap();

        assert_eq!(first.output(), b"bc");
        assert_eq!(second.output(), b"bc");
    }

    #[test]
    fn trace_events_are_emitted_without_core_stderr() {
        let program = Program::parse("a=b\nb=(return)ok").unwrap();
        let mut events = Vec::new();
        let result = program
            .run_with_trace(b"a", RunOptions::new(10_000), |event| events.push(event))
            .unwrap();

        assert_eq!(result.output(), b"ok");
        assert!(result.returned());
        assert_eq!(events.len(), 3);
        assert!(matches!(events[0], TraceEvent::Initial { .. }));
        assert_eq!(events[0].bytes(), b"a");
        assert_eq!(events[1].bytes(), b"b");
        assert_eq!(events[2].bytes(), b"ok");
    }

    #[test]
    fn normal_replacement_is_ordered_and_leftmost() {
        let source = "aa=x\na=y";
        assert_eq!(run_source(source, "aaaa"), "xx");
    }

    #[test]
    fn start_anchor_matches_only_at_start() {
        let source = "(start)a=x";
        assert_eq!(run_source(source, "aba"), "xba");
        assert_eq!(run_source(source, "ba"), "ba");
    }

    #[test]
    fn end_anchor_matches_only_at_end() {
        let source = "(end)a=x";
        assert_eq!(run_source(source, "aba"), "abx");
        assert_eq!(run_source(source, "ab"), "ab");
    }

    #[test]
    fn runtime_continues_after_anchored_replacement() {
        let source = "(start)a=x\na=y";
        assert_eq!(run_source(source, "aba"), "xby");

        let source = "(end)a=x\na=y";
        assert_eq!(run_source(source, "aba"), "ybx");
    }

    #[test]
    fn move_start_works() {
        let source = "a=(start)x";
        assert_eq!(run_source(source, "ba"), "xb");
    }

    #[test]
    fn move_end_works() {
        let source = "a=(end)x";
        assert_eq!(run_source(source, "ba"), "bx");
    }

    #[test]
    fn once_rule_is_used_at_most_once() {
        let source = "(once)a=b\na=c";
        assert_eq!(run_source(source, "aa"), "bc");
    }

    #[test]
    fn return_discards_current_state() {
        let source = "aa=(return)ok\na=x";
        assert_eq!(run_source(source, "aabb"), "ok");
    }

    #[test]
    fn empty_lhs_inserts_at_start() {
        let source = "aaa=(return)a\n=a";
        assert_eq!(run_source(source, ""), "a");
    }

    #[test]
    fn code_spaces_are_ignored_in_rules() {
        assert_eq!(run_source("a b=bb", "abc"), "bbc");
        assert_eq!(run_source("a = b", "a"), "b");
        assert_eq!(run_source("( once ) a = ( end ) b", "ca"), "cb");
    }

    #[test]
    fn input_spaces_are_preserved_and_do_not_bridge_matches() {
        assert_eq!(run_source("a= b", "a bc"), "b bc");
        assert_eq!(run_source("a b=bb", "a bc"), "a bc");
        assert_eq!(run_source("ab=bb", "a bc"), "a bc");
    }

    #[test]
    fn code_cannot_create_or_match_space_even_when_space_is_written_near_rules() {
        assert_eq!(run_source("a= ", "a "), " ");
        assert_eq!(run_source(" a = b ", "a"), "b");
    }

    #[test]
    fn hash_starts_a_comment() {
        assert_eq!(run_source("a=b#c", "a"), "b");
        assert_eq!(run_source("#a=b", "a"), "a");
        assert_eq!(run_source("a=b#コメント内の非ASCIIは許可", "a"), "b");
    }

    #[test]
    fn code_body_rejects_non_ascii_outside_comments() {
        assert!(Program::parse("a=あ").is_err());
        assert!(Program::parse("あ=b# comment").is_err());
        assert!(Program::parse("a=b#あ").is_ok());
    }

    #[test]
    fn second_equals_is_a_parse_error_unless_it_is_in_a_comment() {
        assert!(Program::parse("a=b=c").is_err());
        assert!(Program::parse("a=b#=c").is_ok());
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
    fn reserved_input_bytes_are_preserved_but_not_editable_from_code() {
        assert_eq!(run_source("a=b", "a=()#c"), "b=()#c");
        assert!(
            Program::parse("a=b")
                .unwrap()
                .run("aあ".as_bytes(), RunOptions::default())
                .is_err()
        );
    }

    #[test]
    fn runtime_state_can_hold_reserved_bytes_that_program_payloads_cannot_construct() {
        let program = Program::parse("a=b").unwrap();
        assert!(Payload::parse(b"=", 1, "test payload").is_err());

        let result = program.run(b"a=#()", RunOptions::new(10_000)).unwrap();
        assert_eq!(String::from_utf8(result.into_output()).unwrap(), "b=#()");
    }

    #[test]
    fn palindrome_example_returns_true_or_false() {
        let source = "\
b=a|a|
c=a|aa|
a|-=\n--=(return)false\n(start)a|=(end)-\n(start)a=(end)|-\n=(return)true";

        assert_eq!(run_source(source, "aba"), "true");
        assert_eq!(run_source(source, "ab"), "false");
    }
}
