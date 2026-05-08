use std::env;
use std::fmt;
use std::fs;
use std::process;

const TOK_ONCE: &[u8] = b"(once)";
const TOK_START: &[u8] = b"(start)";
const TOK_END: &[u8] = b"(end)";
const TOK_RETURN: &[u8] = b"(return)";

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

    fn to_lossy_string(&self) -> String {
        let bytes = self.iter().map(CodeByte::as_u8).collect::<Vec<_>>();
        String::from_utf8_lossy(&bytes).into_owned()
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

    fn from_payload(payload: &Payload) -> Self {
        Self {
            bytes: payload.to_runtime_bytes(),
        }
    }

    #[cfg(test)]
    fn into_vec_u8(self) -> Vec<u8> {
        self.bytes.into_iter().map(RuntimeByte::as_u8).collect()
    }

    fn to_lossy_string(&self) -> String {
        let bytes = self
            .bytes
            .iter()
            .copied()
            .map(RuntimeByte::as_u8)
            .collect::<Vec<_>>();
        String::from_utf8_lossy(&bytes).into_owned()
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct Program {
    rules: Vec<Rule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Runtime<'program> {
    program: &'program Program,
    state: State,
    steps: usize,
    trace: bool,
    rule_states: Box<[RuntimeRuleState]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RunResult {
    output: State,
    steps: usize,
    returned: bool,
}

#[derive(Debug)]
enum AebError {
    Parse { line: usize, message: String },
    Input { message: String },
    StepLimit { max_steps: usize, state: State },
    Io(std::io::Error),
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
                state.to_lossy_string(),
            ),
            AebError::Io(error) => write!(f, "io error: {error}"),
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

fn parse_program(source: &str) -> Result<Program, AebError> {
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

impl<'program> Runtime<'program> {
    fn new(program: &'program Program, input: &[u8], trace: bool) -> Result<Self, AebError> {
        Ok(Self {
            program,
            state: State::parse_input(input)?,
            steps: 0,
            trace,
            rule_states: vec![RuntimeRuleState::Fresh; program.rules.len()].into_boxed_slice(),
        })
    }

    fn run(mut self, max_steps: usize) -> Result<RunResult, AebError> {
        if self.trace {
            eprintln!("0: {}", self.state.to_lossy_string());
        }

        loop {
            if self.steps >= max_steps {
                return Err(AebError::StepLimit {
                    max_steps,
                    state: self.state,
                });
            }

            let Some((rule_index, position)) = self.find_next_match() else {
                return Ok(RunResult {
                    output: self.state,
                    steps: self.steps,
                    returned: false,
                });
            };

            if let Some(result) = self.apply_rule(rule_index, position) {
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

    fn apply_rule(&mut self, rule_index: usize, position: usize) -> Option<RunResult> {
        self.consume_rule_if_needed(rule_index);

        let rule = &self.program.rules[rule_index];
        let lhs_len = rule.lhs.len();

        match &rule.action {
            Action::Replace(rhs) => {
                self.state = self.state.replace_at(position, lhs_len, rhs);
                self.steps += 1;
            }
            Action::MoveStart(rhs) => {
                self.state = self.state.move_start_at(position, lhs_len, rhs);
                self.steps += 1;
            }
            Action::MoveEnd(rhs) => {
                self.state = self.state.move_end_at(position, lhs_len, rhs);
                self.steps += 1;
            }
            Action::Return(output) => {
                self.steps += 1;

                if self.trace {
                    eprintln!(
                        "{}: line {}: {} => return {}",
                        self.steps,
                        rule.line_number,
                        rule.source,
                        output.to_lossy_string(),
                    );
                }

                return Some(RunResult {
                    output: State::from_payload(output),
                    steps: self.steps,
                    returned: true,
                });
            }
        }

        if self.trace {
            eprintln!(
                "{}: line {}: {} => {}",
                self.steps,
                rule.line_number,
                rule.source,
                self.state.to_lossy_string(),
            );
        }

        None
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct Cli {
    program_path: String,
    input: Vec<u8>,
    max_steps: usize,
    trace: bool,
}

fn parse_cli() -> Result<Cli, String> {
    let mut args = env::args().skip(1);
    let mut max_steps = 1_000_000usize;
    let mut trace = false;
    let mut positional = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--trace" => {
                trace = true;
            }
            "--max-steps" => {
                let Some(value) = args.next() else {
                    return Err("--max-steps requires a number".to_string());
                };

                max_steps = value
                    .parse::<usize>()
                    .map_err(|_| format!("invalid --max-steps value: {value}"))?;
            }
            "-h" | "--help" => {
                return Err(usage());
            }
            _ => {
                positional.push(arg);
            }
        }
    }

    if positional.is_empty() || positional.len() > 2 {
        return Err(usage());
    }

    Ok(Cli {
        program_path: positional[0].clone(),
        input: positional
            .get(1)
            .map_or_else(Vec::new, |value| value.as_bytes().to_vec()),
        max_steps,
        trace,
    })
}

fn usage() -> String {
    "usage: aeb <program-file> [input] [--max-steps N] [--trace]".to_string()
}

fn main() {
    let cli = match parse_cli() {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("{message}");
            process::exit(2);
        }
    };

    let source = match fs::read_to_string(&cli.program_path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("{}", AebError::Io(error));
            process::exit(1);
        }
    };

    let program = match parse_program(&source) {
        Ok(program) => program,
        Err(error) => {
            eprintln!("{error}");
            process::exit(1);
        }
    };

    let runtime = match Runtime::new(&program, &cli.input, cli.trace) {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("{error}");
            process::exit(2);
        }
    };

    match runtime.run(cli.max_steps) {
        Ok(result) => {
            println!("{}", result.output.to_lossy_string());

            if cli.trace {
                eprintln!("steps: {}, returned: {}", result.steps, result.returned);
            }
        }
        Err(error) => {
            eprintln!("{error}");
            process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_source(source: &str, input: &str) -> String {
        let program = parse_program(source).unwrap();
        let result = Runtime::new(&program, input.as_bytes(), false)
            .unwrap()
            .run(10_000)
            .unwrap();

        String::from_utf8(result.output.into_vec_u8()).unwrap()
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
    fn once_state_is_runtime_local() {
        let source = "(once)a=b\na=c";
        let program = parse_program(source).unwrap();

        let first = Runtime::new(&program, b"aa", false)
            .unwrap()
            .run(10_000)
            .unwrap();
        let second = Runtime::new(&program, b"aa", false)
            .unwrap()
            .run(10_000)
            .unwrap();

        assert_eq!(String::from_utf8(first.output.into_vec_u8()).unwrap(), "bc");
        assert_eq!(
            String::from_utf8(second.output.into_vec_u8()).unwrap(),
            "bc"
        );
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
        assert!(parse_program("a=あ").is_err());
        assert!(parse_program("あ=b# comment").is_err());
        assert!(parse_program("a=b#あ").is_ok());
    }

    #[test]
    fn second_equals_is_a_parse_error_unless_it_is_in_a_comment() {
        assert!(parse_program("a=b=c").is_err());
        assert!(parse_program("a=b#=c").is_ok());
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
                parse_program(source).is_err(),
                "source should fail: {source}"
            );
        }

        assert!(parse_program("(once)(start)a=(end)b").is_ok());
        assert!(parse_program("a=(return)").is_ok());
    }

    #[test]
    fn reserved_input_bytes_are_preserved_but_not_editable_from_code() {
        assert_eq!(run_source("a=b", "a=()#c"), "b=()#c");
        assert!(Runtime::new(&parse_program("a=b").unwrap(), "aあ".as_bytes(), false).is_err());
    }

    #[test]
    fn runtime_state_can_hold_reserved_bytes_that_program_payloads_cannot_construct() {
        let program = parse_program("a=b").unwrap();
        assert!(Payload::parse(b"=", 1, "test payload").is_err());

        let result = Runtime::new(&program, b"a=#()", false)
            .unwrap()
            .run(10_000)
            .unwrap();
        assert_eq!(
            String::from_utf8(result.output.into_vec_u8()).unwrap(),
            "b=#()"
        );
    }

    #[test]
    fn palindrome_example_returns_true_or_false() {
        let source = "\
b=a|a|
c=a|aa|
a|-=
--=(return)false
(start)a|=(end)-
(start)a=(end)|-
=(return)true";

        assert_eq!(run_source(source, "aba"), "true");
        assert_eq!(run_source(source, "ab"), "false");
    }
}
