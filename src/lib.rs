//! Library API for the A=B rewrite interpreter.
//!
//! The crate exposes a byte-oriented parser and runtime. Program syntax and
//! runtime input are separate domains:
//!
//! - program code is compact ASCII syntax;
//! - comments are ignored bytes after `#`;
//! - runtime input is ASCII data and may contain whitespace/reserved bytes;
//! - program payloads cannot contain whitespace, reserved syntax characters, or
//!   non-ASCII bytes.
//!
//! Files, stdout, stderr, argument parsing, and lossy display formatting are
//! intentionally outside this library. The command-line binary can do command-
//! line things without smuggling those habits into the interpreter core.

#![no_std]

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::error::Error;
use core::fmt;

#[cfg(test)]
extern crate std;

/// Default maximum number of rewrite steps for one execution.
pub const DEFAULT_MAX_STEPS: usize = 1_000_000;

const TOK_ONCE: &[u8] = b"(once)";
const TOK_START: &[u8] = b"(start)";
const TOK_END: &[u8] = b"(end)";
const TOK_RETURN: &[u8] = b"(return)";

/// Parses and runs source bytes in one call.
///
/// Use [`Program::parse`] when the same program will be executed multiple
/// times.
pub fn run(
    source: impl AsRef<[u8]>,
    input: impl AsRef<[u8]>,
    options: RunOptions,
) -> Result<RunResult, AebError> {
    let program = Program::parse(source).map_err(AebError::Parse)?;
    program.run(input, options).map_err(AebError::Run)
}

/// Execution options for one runtime invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunOptions {
    /// Maximum number of rewrite steps that may be applied.
    ///
    /// A run that becomes stable exactly at this count succeeds. The limit is
    /// an error only when another matching rule would need to be applied after
    /// this many steps.
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

/// Program-local zero-based index for a parsed rule.
///
/// The value is only meaningful together with the [`Program`] that produced it.
/// It is intentionally named as an index, not a globally valid identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuleIndex(usize);

impl RuleIndex {
    /// Zero-based rule position in parse order.
    #[must_use]
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

/// Read-only metadata for a parsed rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleInfo<'program> {
    index: RuleIndex,
    line_number: usize,
    compact_source: &'program [u8],
}

impl<'program> RuleInfo<'program> {
    /// Program-local zero-based rule index.
    #[must_use]
    pub const fn index(self) -> RuleIndex {
        self.index
    }

    /// One-based source line number.
    #[must_use]
    pub const fn line_number(self) -> usize {
        self.line_number
    }

    /// Whitespace-stripped executable code for this rule.
    #[must_use]
    pub const fn compact_source(self) -> &'program [u8] {
        self.compact_source
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
    fn parse_compact(
        byte: CompactByte,
        line_number: usize,
        payload_kind: PayloadKind,
    ) -> Result<Self, ParseError> {
        debug_assert!(byte.as_u8().is_ascii());
        debug_assert!(!byte.as_u8().is_ascii_whitespace());

        if is_reserved_code_byte(byte.as_u8()) {
            return Err(ParseError::new(
                line_number,
                Some(byte.source_column()),
                ParseErrorKind::ReservedByteInPayload {
                    byte: byte.as_u8(),
                    payload_kind,
                },
            ));
        }

        Ok(Self(byte.as_u8()))
    }

    fn as_u8(self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RuntimeByte(u8);

impl RuntimeByte {
    fn parse_input(byte: u8, zero_based_column: usize) -> Result<Self, InputError> {
        if !byte.is_ascii() {
            return Err(InputError {
                column: zero_based_column + 1,
                byte,
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
    fn parse(
        input: &[CompactByte],
        line_number: usize,
        payload_kind: PayloadKind,
    ) -> Result<Self, ParseError> {
        let bytes = input
            .iter()
            .copied()
            .map(|byte| CodeByte::parse_compact(byte, line_number, payload_kind))
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

    fn runtime_bytes(&self) -> impl Iterator<Item = RuntimeByte> + '_ {
        self.iter().map(RuntimeByte::from_code)
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
    fn parse_input(input: &[u8]) -> Result<Self, InputError> {
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

    fn starts_with_payload(&self, payload: &Payload) -> Option<StateMatch> {
        self.matches_payload_at(0, payload)
    }

    fn ends_with_payload(&self, payload: &Payload) -> Option<StateMatch> {
        let start = self.len().checked_sub(payload.len())?;
        self.matches_payload_at(start, payload)
    }

    fn find_payload(&self, payload: &Payload) -> Option<StateMatch> {
        if payload.is_empty() {
            return StateMatch::checked(0, 0, self.len());
        }

        let last_start = self.len().checked_sub(payload.len())?;
        (0..=last_start).find_map(|position| self.matches_payload_at(position, payload))
    }

    fn matches_payload_at(&self, position: usize, payload: &Payload) -> Option<StateMatch> {
        let end = position.checked_add(payload.len())?;
        let window = self.bytes.get(position..end)?;

        window
            .iter()
            .copied()
            .zip(payload.iter())
            .all(|(state_byte, payload_byte)| state_byte.as_u8() == payload_byte.as_u8())
            .then(|| StateMatch::new_unchecked(position, payload.len(), end))
    }

    fn replace_at(&self, state_match: StateMatch, rhs: &Payload) -> Self {
        let mut bytes = Vec::with_capacity(self.replaced_len(state_match, rhs));
        self.push_prefix(&mut bytes, state_match);
        bytes.extend(rhs.runtime_bytes());
        self.push_suffix(&mut bytes, state_match);
        Self { bytes }
    }

    fn move_start_at(&self, state_match: StateMatch, rhs: &Payload) -> Self {
        let mut bytes = Vec::with_capacity(self.replaced_len(state_match, rhs));
        bytes.extend(rhs.runtime_bytes());
        self.push_prefix(&mut bytes, state_match);
        self.push_suffix(&mut bytes, state_match);
        Self { bytes }
    }

    fn move_end_at(&self, state_match: StateMatch, rhs: &Payload) -> Self {
        let mut bytes = Vec::with_capacity(self.replaced_len(state_match, rhs));
        self.push_prefix(&mut bytes, state_match);
        self.push_suffix(&mut bytes, state_match);
        bytes.extend(rhs.runtime_bytes());
        Self { bytes }
    }

    fn apply_action(&self, state_match: StateMatch, action: &Action) -> RewriteEffect {
        match action {
            Action::Replace(rhs) => RewriteEffect::Continue(self.replace_at(state_match, rhs)),
            Action::MoveStart(rhs) => RewriteEffect::Continue(self.move_start_at(state_match, rhs)),
            Action::MoveEnd(rhs) => RewriteEffect::Continue(self.move_end_at(state_match, rhs)),
            Action::Return(output) => RewriteEffect::Return(output.to_vec_u8()),
        }
    }

    fn replaced_len(&self, state_match: StateMatch, rhs: &Payload) -> usize {
        self.len() - state_match.lhs_len() + rhs.len()
    }

    fn push_prefix(&self, output: &mut Vec<RuntimeByte>, state_match: StateMatch) {
        output.extend(self.bytes.iter().take(state_match.position()).copied());
    }

    fn push_suffix(&self, output: &mut Vec<RuntimeByte>, state_match: StateMatch) {
        output.extend(self.bytes.iter().skip(state_match.end()).copied());
    }

    fn to_vec_u8(&self) -> Vec<u8> {
        self.bytes.iter().copied().map(RuntimeByte::as_u8).collect()
    }

    fn into_vec_u8(self) -> Vec<u8> {
        self.bytes.into_iter().map(RuntimeByte::as_u8).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StateMatch {
    position: usize,
    lhs_len: usize,
    end: usize,
}

impl StateMatch {
    fn checked(position: usize, lhs_len: usize, state_len: usize) -> Option<Self> {
        let end = position.checked_add(lhs_len)?;
        (position <= state_len && end <= state_len)
            .then(|| Self::new_unchecked(position, lhs_len, end))
    }

    const fn new_unchecked(position: usize, lhs_len: usize, end: usize) -> Self {
        Self {
            position,
            lhs_len,
            end,
        }
    }

    const fn position(self) -> usize {
        self.position
    }

    const fn lhs_len(self) -> usize {
        self.lhs_len
    }

    const fn end(self) -> usize {
        self.end
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
enum RewriteEffect {
    Continue(State),
    Return(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Rule {
    line_number: usize,
    compact_source: Vec<u8>,
    repeat: RuleRepeat,
    anchor: Anchor,
    lhs: Payload,
    action: Action,
}

impl Rule {
    fn info(&self, index: usize) -> RuleInfo<'_> {
        RuleInfo {
            index: RuleIndex(index),
            line_number: self.line_number,
            compact_source: &self.compact_source,
        }
    }
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
    /// Parses program source bytes into a reusable program value.
    pub fn parse(source: impl AsRef<[u8]>) -> Result<Self, ParseError> {
        parse_program_impl(source.as_ref())
    }

    /// Parses program source bytes into a reusable program value.
    pub fn parse_bytes(source: &[u8]) -> Result<Self, ParseError> {
        parse_program_impl(source)
    }

    /// Parses a UTF-8 source string into a reusable program value.
    pub fn parse_str(source: &str) -> Result<Self, ParseError> {
        parse_program_impl(source.as_bytes())
    }

    /// Returns the number of executable rules in the parsed program.
    #[must_use]
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Returns rule metadata by program-local [`RuleIndex`].
    #[must_use]
    pub fn rule(&self, index: RuleIndex) -> Option<RuleInfo<'_>> {
        self.rules
            .get(index.as_usize())
            .map(|rule| rule.info(index.as_usize()))
    }

    /// Iterates over parsed rule metadata in execution order.
    pub fn rules(&self) -> impl Iterator<Item = RuleInfo<'_>> + '_ {
        self.rules
            .iter()
            .enumerate()
            .map(|(index, rule)| rule.info(index))
    }

    /// Runs this program with the given input bytes.
    pub fn run(&self, input: impl AsRef<[u8]>, options: RunOptions) -> Result<RunResult, RunError> {
        Runtime::new(self, input.as_ref())?.run(options.max_steps)
    }

    /// Runs this program and emits trace events.
    pub fn run_with_trace<'program, F>(
        &'program self,
        input: impl AsRef<[u8]>,
        options: RunOptions,
        trace: F,
    ) -> Result<RunResult, RunError>
    where
        F: FnMut(TraceEvent<'program>),
    {
        Runtime::new(self, input.as_ref())?.run_with_trace(options.max_steps, trace)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MatchedRule<'program> {
    rule_index: RuleIndex,
    rule: &'program Rule,
    state_match: StateMatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Runtime<'program> {
    program: &'program Program,
    state: State,
    steps: usize,
    rule_states: Box<[RuntimeRuleState]>,
}

impl<'program> Runtime<'program> {
    fn new(program: &'program Program, input: &[u8]) -> Result<Self, InputError> {
        Ok(Self {
            program,
            state: State::parse_input(input)?,
            steps: 0,
            rule_states: vec![RuntimeRuleState::Fresh; program.rules.len()].into_boxed_slice(),
        })
    }

    fn run(self, max_steps: usize) -> Result<RunResult, RunError> {
        self.run_impl(max_steps, None::<fn(TraceEvent<'program>)>)
    }

    fn run_with_trace<F>(self, max_steps: usize, trace: F) -> Result<RunResult, RunError>
    where
        F: FnMut(TraceEvent<'program>),
    {
        self.run_impl(max_steps, Some(trace))
    }

    fn run_impl<F>(mut self, max_steps: usize, mut trace: Option<F>) -> Result<RunResult, RunError>
    where
        F: FnMut(TraceEvent<'program>),
    {
        emit_trace(&mut trace, || TraceEvent::Initial {
            state: self.state.to_vec_u8(),
        });

        loop {
            let Some(matched) = self.find_next_match() else {
                return Ok(RunResult {
                    output: self.state.into_vec_u8(),
                    steps: self.steps,
                    returned: false,
                });
            };

            if self.steps >= max_steps {
                return Err(StepLimitError {
                    max_steps,
                    state: self.state.into_vec_u8(),
                }
                .into());
            }

            if let Some(result) = self.apply_rule(matched, &mut trace) {
                return Ok(result);
            }
        }
    }

    fn find_next_match(&self) -> Option<MatchedRule<'program>> {
        let rules: &'program [Rule] = &self.program.rules;

        rules
            .iter()
            .zip(self.rule_states.iter())
            .enumerate()
            .find_map(|(rule_index, (rule, state))| {
                if rule.repeat.is_once() && state.is_consumed() {
                    return None;
                }

                find_match(&self.state, rule).map(|state_match| MatchedRule {
                    rule_index: RuleIndex(rule_index),
                    rule,
                    state_match,
                })
            })
    }

    fn consume_rule_if_needed(&mut self, matched: MatchedRule<'_>) {
        if !matched.rule.repeat.is_once() {
            return;
        }

        if let Some(state) = self.rule_states.get_mut(matched.rule_index.as_usize()) {
            *state = RuntimeRuleState::Consumed;
        }
    }

    fn apply_rule<F>(
        &mut self,
        matched: MatchedRule<'program>,
        trace: &mut Option<F>,
    ) -> Option<RunResult>
    where
        F: FnMut(TraceEvent<'program>),
    {
        self.consume_rule_if_needed(matched);

        let rule = matched.rule;
        let rule_index = matched.rule_index;
        let effect = self
            .state
            .apply_action(matched.state_match, &matched.rule.action);

        self.steps += 1;

        match effect {
            RewriteEffect::Continue(next_state) => {
                emit_trace(trace, || TraceEvent::Step {
                    step: self.steps,
                    rule: rule.info(rule_index.as_usize()),
                    output: next_state.to_vec_u8(),
                    returned: false,
                });

                self.state = next_state;
                None
            }
            RewriteEffect::Return(output) => {
                emit_trace(trace, || TraceEvent::Step {
                    step: self.steps,
                    rule: rule.info(rule_index.as_usize()),
                    output: output.clone(),
                    returned: true,
                });

                Some(RunResult {
                    output,
                    steps: self.steps,
                    returned: true,
                })
            }
        }
    }
}

fn emit_trace<'program, F, E>(trace: &mut Option<F>, event: E)
where
    F: FnMut(TraceEvent<'program>),
    E: FnOnce() -> TraceEvent<'program>,
{
    if let Some(trace) = trace.as_mut() {
        trace(event());
    }
}

fn find_match(state: &State, rule: &Rule) -> Option<StateMatch> {
    match rule.anchor {
        Anchor::Anywhere => state.find_payload(&rule.lhs),
        Anchor::Start => state.starts_with_payload(&rule.lhs),
        Anchor::End => state.ends_with_payload(&rule.lhs),
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
///
/// Step events carry borrowed rule metadata tied to the source [`Program`], so a
/// trace cannot accidentally describe a rule from a different program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceEvent<'program> {
    /// Initial runtime state before any rewrite step.
    Initial { state: Vec<u8> },
    /// One applied rule.
    Step {
        step: usize,
        rule: RuleInfo<'program>,
        output: Vec<u8>,
        returned: bool,
    },
}

impl<'program> TraceEvent<'program> {
    /// Output/state bytes carried by this event.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::Initial { state } => state,
            Self::Step { output, .. } => output,
        }
    }
}

/// Combined one-shot error used by [`run`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AebError {
    /// Source program parse error.
    Parse(ParseError),
    /// Runtime execution error.
    Run(RunError),
}

impl fmt::Display for AebError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(error) => error.fmt(f),
            Self::Run(error) => error.fmt(f),
        }
    }
}

impl Error for AebError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Parse(error) => Some(error),
            Self::Run(error) => Some(error),
        }
    }
}

impl From<ParseError> for AebError {
    fn from(value: ParseError) -> Self {
        Self::Parse(value)
    }
}

impl From<RunError> for AebError {
    fn from(value: RunError) -> Self {
        Self::Run(value)
    }
}

/// Source program parse error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    line: usize,
    column: Option<usize>,
    kind: ParseErrorKind,
}

impl ParseError {
    fn new(line: usize, column: Option<usize>, kind: ParseErrorKind) -> Self {
        Self { line, column, kind }
    }

    /// One-based source line number.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }

    /// One-based source column, when the error has a single byte position.
    #[must_use]
    pub const fn column(&self) -> Option<usize> {
        self.column
    }

    /// Structured parse error reason.
    #[must_use]
    pub const fn kind(&self) -> &ParseErrorKind {
        &self.kind
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "parse error at line {}", self.line)?;

        if let Some(column) = self.column {
            write!(f, ", column {column}")?;
        }

        write!(f, ": {}", self.kind)
    }
}

impl Error for ParseError {}

/// Structured parse error reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseErrorKind {
    /// A non-ASCII byte appeared before the line comment marker.
    NonAsciiInCode { byte: u8 },
    /// A non-empty code line did not contain `=`.
    MissingEquals,
    /// A compact code line contained more than one `=`.
    MultipleEquals,
    /// A payload attempted to contain syntax bytes such as `=`, `#`, `(`, or `)`.
    ReservedByteInPayload { byte: u8, payload_kind: PayloadKind },
    /// Left-side modifiers were duplicated or ordered outside the supported grammar.
    UnsupportedLeftModifierOrder,
}

impl fmt::Display for ParseErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonAsciiInCode { byte } => write!(f, "non-ASCII byte 0x{byte:02x} in code"),
            Self::MissingEquals => write!(f, "missing '='"),
            Self::MultipleEquals => write!(f, "multiple '=' characters are not allowed"),
            Self::ReservedByteInPayload { byte, payload_kind } => write!(
                f,
                "reserved character '{}' cannot be represented as {payload_kind}",
                printable_ascii(*byte),
            ),
            Self::UnsupportedLeftModifierOrder => {
                write!(f, "duplicated or unsupported left-side modifier order")
            }
        }
    }
}

/// Program payload context used by structured parse errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadKind {
    LeftSideData,
    RightSideData,
    RightSideMoveStartPayload,
    RightSideMoveEndPayload,
    RightSideReturnPayload,
}

impl fmt::Display for PayloadKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LeftSideData => write!(f, "left-side data"),
            Self::RightSideData => write!(f, "right-side data"),
            Self::RightSideMoveStartPayload => write!(f, "right-side move-to-start payload"),
            Self::RightSideMoveEndPayload => write!(f, "right-side move-to-end payload"),
            Self::RightSideReturnPayload => write!(f, "right-side return payload"),
        }
    }
}

/// Runtime execution error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    /// Runtime input is invalid.
    Input(InputError),
    /// Execution exceeded the configured step limit.
    StepLimit(StepLimitError),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(error) => error.fmt(f),
            Self::StepLimit(error) => error.fmt(f),
        }
    }
}

impl Error for RunError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Input(error) => Some(error),
            Self::StepLimit(error) => Some(error),
        }
    }
}

impl From<InputError> for RunError {
    fn from(value: InputError) -> Self {
        Self::Input(value)
    }
}

impl From<StepLimitError> for RunError {
    fn from(value: StepLimitError) -> Self {
        Self::StepLimit(value)
    }
}

/// Runtime input validation error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputError {
    column: usize,
    byte: u8,
}

impl InputError {
    /// One-based input column.
    #[must_use]
    pub const fn column(&self) -> usize {
        self.column
    }

    /// Rejected byte.
    #[must_use]
    pub const fn byte(&self) -> u8 {
        self.byte
    }
}

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "input error: non-ASCII byte 0x{:02x} at column {}",
            self.byte, self.column,
        )
    }
}

impl Error for InputError {}

/// Step-limit failure with the last runtime state preserved as bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepLimitError {
    max_steps: usize,
    state: Vec<u8>,
}

impl StepLimitError {
    /// Configured maximum step count.
    #[must_use]
    pub const fn max_steps(&self) -> usize {
        self.max_steps
    }

    /// Runtime state when the limit was hit.
    #[must_use]
    pub fn state(&self) -> &[u8] {
        &self.state
    }

    /// Consumes the error and returns the runtime state.
    #[must_use]
    pub fn into_state(self) -> Vec<u8> {
        self.state
    }
}

impl fmt::Display for StepLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "step limit exceeded after {} steps; state length: {} bytes",
            self.max_steps,
            self.state.len(),
        )
    }
}

impl Error for StepLimitError {}

fn is_reserved_code_byte(byte: u8) -> bool {
    matches!(byte, b'=' | b'#' | b'(' | b')')
}

fn printable_ascii(byte: u8) -> char {
    if byte.is_ascii() {
        byte as char
    } else {
        '\u{fffd}'
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CompactByte {
    byte: u8,
    source_column: usize,
}

impl CompactByte {
    const fn new(byte: u8, source_column: usize) -> Self {
        Self {
            byte,
            source_column,
        }
    }

    const fn as_u8(self) -> u8 {
        self.byte
    }

    const fn source_column(self) -> usize {
        self.source_column
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CodeLine<'source> {
    line_number: usize,
    bytes: &'source [u8],
}

impl<'source> CodeLine<'source> {
    fn parse(raw_line: &'source [u8], line_number: usize) -> Result<Self, ParseError> {
        let code_bytes = raw_line
            .iter()
            .position(|&byte| byte == b'#')
            .and_then(|comment_start| raw_line.get(..comment_start))
            .unwrap_or(raw_line);

        if let Some((zero_based_column, byte)) = code_bytes
            .iter()
            .copied()
            .enumerate()
            .find(|(_, byte)| !byte.is_ascii())
        {
            return Err(ParseError::new(
                line_number,
                Some(zero_based_column + 1),
                ParseErrorKind::NonAsciiInCode { byte },
            ));
        }

        Ok(Self {
            line_number,
            bytes: code_bytes,
        })
    }

    fn compact(self) -> CompactCodeLine {
        let bytes = self
            .bytes
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, byte)| !byte.is_ascii_whitespace())
            .map(|(zero_based_column, byte)| CompactByte::new(byte, zero_based_column + 1))
            .collect();

        CompactCodeLine {
            line_number: self.line_number,
            bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompactCodeLine {
    line_number: usize,
    bytes: Vec<CompactByte>,
}

impl CompactCodeLine {
    fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    fn compact_source(&self) -> Vec<u8> {
        self.bytes.iter().copied().map(CompactByte::as_u8).collect()
    }

    fn equals_position(&self) -> Result<usize, ParseError> {
        let Some(first_equals) = self.bytes.iter().position(|byte| byte.as_u8() == b'=') else {
            return Err(ParseError::new(
                self.line_number,
                None,
                ParseErrorKind::MissingEquals,
            ));
        };

        if let Some(second_equals) = self
            .bytes
            .iter()
            .skip(first_equals + 1)
            .find(|byte| byte.as_u8() == b'=')
            .copied()
        {
            return Err(ParseError::new(
                self.line_number,
                Some(second_equals.source_column()),
                ParseErrorKind::MultipleEquals,
            ));
        }

        Ok(first_equals)
    }

    fn split_at_equals(
        &self,
        equals_position: usize,
    ) -> Result<(&[CompactByte], &[CompactByte]), ParseError> {
        let (lhs, rhs_with_equals) = self.bytes.split_at(equals_position);

        let Some(rhs) = rhs_with_equals.get(1..) else {
            return Err(ParseError::new(
                self.line_number,
                None,
                ParseErrorKind::MissingEquals,
            ));
        };

        Ok((lhs, rhs))
    }
}

fn parse_program_impl(source: &[u8]) -> Result<Program, ParseError> {
    let mut rules = Vec::new();

    for (zero_based_line, raw_line) in source.split(|&byte| byte == b'\n').enumerate() {
        let line_number = zero_based_line + 1;
        let compact_code = CodeLine::parse(raw_line, line_number)?.compact();

        if compact_code.is_empty() {
            continue;
        }

        let equals_position = compact_code.equals_position()?;
        let compact_source = compact_code.compact_source();
        let (lhs_code, rhs_code) = compact_code.split_at_equals(equals_position)?;
        let (repeat, anchor, lhs) = parse_lhs(lhs_code, line_number)?;
        let action = parse_rhs(rhs_code, line_number)?;

        rules.push(Rule {
            line_number,
            compact_source,
            repeat,
            anchor,
            lhs,
            action,
        });
    }

    Ok(Program { rules })
}

fn strip_token<'code>(input: &'code [CompactByte], token: &[u8]) -> Option<&'code [CompactByte]> {
    if input.len() < token.len() {
        return None;
    }

    let starts_with_token = input
        .iter()
        .take(token.len())
        .copied()
        .map(CompactByte::as_u8)
        .eq(token.iter().copied());

    if starts_with_token {
        input.get(token.len()..)
    } else {
        None
    }
}

fn starts_with_token(input: &[CompactByte], token: &[u8]) -> bool {
    strip_token(input, token).is_some()
}

fn parse_lhs(
    mut input: &[CompactByte],
    line_number: usize,
) -> Result<(RuleRepeat, Anchor, Payload), ParseError> {
    let mut repeat = RuleRepeat::Always;

    if let Some(rest) = strip_token(input, TOK_ONCE) {
        repeat = RuleRepeat::Once;
        input = rest;
    }

    let anchor = if let Some(rest) = strip_token(input, TOK_START) {
        input = rest;
        Anchor::Start
    } else if let Some(rest) = strip_token(input, TOK_END) {
        input = rest;
        Anchor::End
    } else {
        Anchor::Anywhere
    };

    if starts_with_token(input, TOK_ONCE)
        || starts_with_token(input, TOK_START)
        || starts_with_token(input, TOK_END)
    {
        return Err(ParseError::new(
            line_number,
            input.first().copied().map(CompactByte::source_column),
            ParseErrorKind::UnsupportedLeftModifierOrder,
        ));
    }

    let lhs = Payload::parse(input, line_number, PayloadKind::LeftSideData)?;
    Ok((repeat, anchor, lhs))
}

fn parse_rhs(input: &[CompactByte], line_number: usize) -> Result<Action, ParseError> {
    if let Some(rest) = strip_token(input, TOK_START) {
        let payload = Payload::parse(rest, line_number, PayloadKind::RightSideMoveStartPayload)?;
        Ok(Action::MoveStart(payload))
    } else if let Some(rest) = strip_token(input, TOK_END) {
        let payload = Payload::parse(rest, line_number, PayloadKind::RightSideMoveEndPayload)?;
        Ok(Action::MoveEnd(payload))
    } else if let Some(rest) = strip_token(input, TOK_RETURN) {
        let payload = Payload::parse(rest, line_number, PayloadKind::RightSideReturnPayload)?;
        Ok(Action::Return(payload))
    } else {
        let payload = Payload::parse(input, line_number, PayloadKind::RightSideData)?;
        Ok(Action::Replace(payload))
    }
}

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

    fn expect_rule(program: &Program, rule: RuleIndex) -> Result<RuleInfo<'_>, TestFailure> {
        program
            .rule(rule)
            .ok_or(TestFailure::Message("expected rule metadata"))
    }

    fn expect_step_limit(error: RunError) -> Result<StepLimitError, TestFailure> {
        match error {
            RunError::StepLimit(error) => Ok(error),
            RunError::Input(_) => Err(TestFailure::Message("expected step limit error")),
        }
    }

    fn expect_input_error(error: RunError) -> Result<InputError, TestFailure> {
        match error {
            RunError::Input(error) => Ok(error),
            RunError::StepLimit(_) => Err(TestFailure::Message("expected input error")),
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

        match first_step {
            TraceEvent::Step { rule, .. } => {
                assert_eq!(rule.index().as_usize(), 0);
                assert_eq!(rule.line_number(), 1);
                assert_eq!(rule.compact_source(), b"a=b");
                assert_eq!(expect_rule(&program, rule.index())?.compact_source(), b"a=b");
            }
            TraceEvent::Initial { .. } => {
                return Err(TestFailure::Message("expected step event"));
            }
        }

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

        assert_eq!(first.index().as_usize(), 0);
        assert_eq!(first.line_number(), 1);
        assert_eq!(first.compact_source(), b"a=b");
        assert_eq!(second.index().as_usize(), 1);
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
    fn reserved_payload_errors_report_original_source_column_after_compaction() -> TestResult {
        let error = expect_parse_error("a = b (")?;
        assert_eq!(error.column(), Some(7));
        assert!(matches!(
            error.kind(),
            ParseErrorKind::ReservedByteInPayload {
                payload_kind: PayloadKind::RightSideData,
                ..
            }
        ));
        Ok(())
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
        let error =
            expect_run_error(Program::parse("a=(return)b")?.run(b"a", RunOptions::new(0)))?;
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
a|-=\n--=(return)false\n(start)a|=(end)-\n(start)a=(end)|-\n=(return)true";

        assert_eq!(run_source(source, "aba")?, "true");
        assert_eq!(run_source(source, "ab")?, "false");
        Ok(())
    }
}
