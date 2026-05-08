use std::env;
use std::fs;
use std::process;

use rsaeb::{DEFAULT_MAX_STEPS, Program, RuleId, RunOptions, TraceEvent};

#[derive(Debug, Clone, PartialEq, Eq)]
struct Cli {
    program_path: String,
    input: Vec<u8>,
    max_steps: usize,
    trace: bool,
}

fn parse_cli() -> Result<Cli, String> {
    let mut args = env::args().skip(1);
    let mut max_steps = DEFAULT_MAX_STEPS;
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

    let mut positional = positional.into_iter();
    let Some(program_path) = positional.next() else {
        return Err(usage());
    };
    let input = positional
        .next()
        .map_or_else(Vec::new, |value| value.as_bytes().to_vec());

    Ok(Cli {
        program_path,
        input,
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

    let source = match fs::read(&cli.program_path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("io error reading '{}': {error}", cli.program_path);
            process::exit(1);
        }
    };

    let program = match Program::parse(&source) {
        Ok(program) => program,
        Err(error) => {
            eprintln!("{error}");
            process::exit(1);
        }
    };

    let options = RunOptions::new(cli.max_steps);
    let result = if cli.trace {
        program.run_with_trace(&cli.input, options, |event| {
            print_trace_event(&program, event);
        })
    } else {
        program.run(&cli.input, options)
    };

    match result {
        Ok(result) => {
            println!("{}", String::from_utf8_lossy(result.output()));

            if cli.trace {
                eprintln!("steps: {}, returned: {}", result.steps(), result.returned());
            }
        }
        Err(error) => {
            eprintln!("{error}");
            process::exit(1);
        }
    }
}

fn print_trace_event(program: &Program, event: TraceEvent) {
    match event {
        TraceEvent::Initial { state } => {
            eprintln!("0: {}", String::from_utf8_lossy(&state));
        }
        TraceEvent::Step {
            step,
            rule,
            line_number,
            output,
            returned,
        } => {
            let source = compact_source_for_trace(program, rule);

            if returned {
                eprintln!(
                    "{step}: line {line_number}: {source} => return {}",
                    String::from_utf8_lossy(&output),
                );
            } else {
                eprintln!(
                    "{step}: line {line_number}: {source} => {}",
                    String::from_utf8_lossy(&output),
                );
            }
        }
    }
}

fn compact_source_for_trace(program: &Program, rule: RuleId) -> String {
    program.rule(rule).map_or_else(
        || format!("<unknown rule {}>", rule.index()),
        |info| String::from_utf8_lossy(info.compact_source()).into_owned(),
    )
}
