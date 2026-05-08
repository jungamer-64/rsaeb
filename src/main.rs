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

#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
    Replace(Vec<u8>),
    MoveStart(Vec<u8>),
    MoveEnd(Vec<u8>),
    Return(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Rule {
    line_number: usize,
    source: String,
    repeat: RuleRepeat,
    anchor: Anchor,
    lhs: Vec<u8>,
    action: Action,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Program {
    rules: Vec<Rule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Runtime<'program> {
    program: &'program Program,
    state: Vec<u8>,
    steps: usize,
    trace: bool,
    rule_states: Box<[RuntimeRuleState]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RunResult {
    output: Vec<u8>,
    steps: usize,
    returned: bool,
}

#[derive(Debug)]
enum OsrError {
    Parse { line: usize, message: String },
    StepLimit { max_steps: usize, state: Vec<u8> },
    Io(std::io::Error),
}

impl fmt::Display for OsrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OsrError::Parse { line, message } => {
                write!(f, "parse error at line {line}: {message}")
            }
            OsrError::StepLimit { max_steps, state } => write!(
                f,
                "step limit exceeded after {max_steps} steps; state: {}",
                String::from_utf8_lossy(state),
            ),
            OsrError::Io(error) => write!(f, "io error: {error}"),
        }
    }
}

impl From<std::io::Error> for OsrError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

fn parse_program(source: &str) -> Result<Program, OsrError> {
    let mut rules = Vec::new();

    for (zero_based_line, raw_line) in source.lines().enumerate() {
        let line_number = zero_based_line + 1;
        let line = raw_line.trim();

        if line.is_empty() {
            continue;
        }

        let Some((lhs_raw, rhs_raw)) = line.split_once('=') else {
            return Err(OsrError::Parse {
                line: line_number,
                message: "missing '='".to_string(),
            });
        };

        let lhs_code = strip_code_whitespace(lhs_raw.as_bytes());
        let rhs_code = strip_code_whitespace(rhs_raw.as_bytes());
        let (repeat, anchor, lhs) = parse_lhs(&lhs_code, line_number)?;
        let action = parse_rhs(&rhs_code);

        rules.push(Rule {
            line_number,
            source: line.to_string(),
            repeat,
            anchor,
            lhs,
            action,
        });
    }

    Ok(Program { rules })
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
) -> Result<(RuleRepeat, Anchor, Vec<u8>), OsrError> {
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
        return Err(OsrError::Parse {
            line: line_number,
            message: "duplicated or unsupported left-side modifier order".to_string(),
        });
    }

    Ok((repeat, anchor, input.to_vec()))
}

fn parse_rhs(input: &[u8]) -> Action {
    if input.starts_with(TOK_START) {
        Action::MoveStart(input[TOK_START.len()..].to_vec())
    } else if input.starts_with(TOK_END) {
        Action::MoveEnd(input[TOK_END.len()..].to_vec())
    } else if input.starts_with(TOK_RETURN) {
        Action::Return(input[TOK_RETURN.len()..].to_vec())
    } else {
        Action::Replace(input.to_vec())
    }
}

impl<'program> Runtime<'program> {
    fn new(program: &'program Program, input: &[u8], trace: bool) -> Self {
        Self {
            program,
            state: input.to_vec(),
            steps: 0,
            trace,
            rule_states: vec![RuntimeRuleState::Fresh; program.rules.len()].into_boxed_slice(),
        }
    }

    fn run(mut self, max_steps: usize) -> Result<RunResult, OsrError> {
        if self.trace {
            eprintln!("0: {}", String::from_utf8_lossy(&self.state));
        }

        loop {
            if self.steps >= max_steps {
                return Err(OsrError::StepLimit {
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
                self.state = replace_at(&self.state, position, lhs_len, rhs);
                self.steps += 1;
            }
            Action::MoveStart(rhs) => {
                self.state = move_start_at(&self.state, position, lhs_len, rhs);
                self.steps += 1;
            }
            Action::MoveEnd(rhs) => {
                self.state = move_end_at(&self.state, position, lhs_len, rhs);
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
                        String::from_utf8_lossy(output),
                    );
                }

                return Some(RunResult {
                    output: output.clone(),
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
                String::from_utf8_lossy(&self.state),
            );
        }

        None
    }
}

fn find_match(state: &[u8], rule: &Rule) -> Option<usize> {
    match rule.anchor {
        Anchor::Anywhere => find_subslice(state, &rule.lhs),
        Anchor::Start => state.starts_with(&rule.lhs).then_some(0),
        Anchor::End => {
            if state.ends_with(&rule.lhs) {
                Some(state.len().saturating_sub(rule.lhs.len()))
            } else {
                None
            }
        }
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }

    if needle.len() > haystack.len() {
        return None;
    }

    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn replace_at(state: &[u8], position: usize, lhs_len: usize, rhs: &[u8]) -> Vec<u8> {
    let mut next = Vec::with_capacity(state.len() - lhs_len + rhs.len());
    next.extend_from_slice(&state[..position]);
    next.extend_from_slice(rhs);
    next.extend_from_slice(&state[position + lhs_len..]);
    next
}

fn move_start_at(state: &[u8], position: usize, lhs_len: usize, rhs: &[u8]) -> Vec<u8> {
    let mut next = Vec::with_capacity(state.len() - lhs_len + rhs.len());
    next.extend_from_slice(rhs);
    next.extend_from_slice(&state[..position]);
    next.extend_from_slice(&state[position + lhs_len..]);
    next
}

fn move_end_at(state: &[u8], position: usize, lhs_len: usize, rhs: &[u8]) -> Vec<u8> {
    let mut next = Vec::with_capacity(state.len() - lhs_len + rhs.len());
    next.extend_from_slice(&state[..position]);
    next.extend_from_slice(&state[position + lhs_len..]);
    next.extend_from_slice(rhs);
    next
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
    "usage: osr <program-file> [input] [--max-steps N] [--trace]".to_string()
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
            eprintln!("{}", OsrError::Io(error));
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

    let runtime = Runtime::new(&program, &cli.input, cli.trace);

    match runtime.run(cli.max_steps) {
        Ok(result) => {
            println!("{}", String::from_utf8_lossy(&result.output));

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
            .run(10_000)
            .unwrap();

        String::from_utf8(result.output).unwrap()
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

        let first = Runtime::new(&program, b"aa", false).run(10_000).unwrap();
        let second = Runtime::new(&program, b"aa", false).run(10_000).unwrap();

        assert_eq!(String::from_utf8(first.output).unwrap(), "bc");
        assert_eq!(String::from_utf8(second.output).unwrap(), "bc");
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
    fn palindrome_example_returns_true_or_false() {
        let source = "\
b=a|a|
c=a|aa|
a|-=\n--=(return)false\n(start)a|=(end)-\n(start)a=(end)|-\n=(return)true";

        assert_eq!(run_source(source, "aba"), "true");
        assert_eq!(run_source(source, "ab"), "false");
    }
}