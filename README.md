# A=B Interpreter Library

A small Rust 2024 `no_std + alloc` library for ordered `lhs=rhs` rewrite
programs.

## Unofficial Project Notice

This project is an unofficial, independently developed interpreter library. It
is not affiliated with, endorsed by, or maintained by Artless Games or the
original A=B author.

A=B's tiny `lhs=rhs` rule system is an unusually elegant programming puzzle
idea, and this project exists because that design is worth studying, testing,
and reimplementing. Thank you to the original A=B author and Artless Games for
creating and publishing the game.

If this interpreter interests you, please support the original game:

- Steam store page: <https://store.steampowered.com/app/1720850/AB/>

The important split is deliberately boring and strict: program code and runtime
input are different domains. Program code is compact ASCII syntax. Runtime input
is ASCII data. The interpreter preserves input bytes that the program syntax
cannot write, such as spaces and reserved characters.

## `no_std` Library Boundary

`src/lib.rs` is `#![no_std]` and uses `alloc` for owned buffers such as
`Vec<u8>`, boxed per-run rule state, `RunResult`, trace events, and step-limit
error state. This means the interpreter core does not depend on `std`, files,
processes, host I/O streams, environment variables, or OS error types. It still
requires an allocator; this is `no_std + alloc`, not a fixed-capacity heapless
interpreter.

For embedded, WASM-core, kernel, or other non-`std` consumers, depend on this
library package or build only the library target:

```sh
cargo check -p rsaeb --lib
```

A downstream `std` application can use the library exactly the same way. A
`no_std` downstream must provide an allocator before calling APIs that allocate.

## Library Usage

This crate exposes the parser, runtime, tracing data, and structured error
types. External I/O and presentation formatting are outside the library
boundary. The library does not expose `std::io` errors, because the interpreter
does not read files.

Basic one-shot execution:

```rust
use rsaeb::{run, RunOptions};

fn main() -> Result<(), rsaeb::AebError> {
    let result = run("a=b", b"a", RunOptions::default())?;
    assert_eq!(result.output(), b"b");
    Ok(())
}
```

Reusable parsed program:

```rust
use rsaeb::{Program, RunOptions};

fn main() -> Result<(), rsaeb::AebError> {
    let program = Program::parse("(once)a=b\na=c")?;

    let first = program.run(b"aa", RunOptions::new(10_000))?;
    let second = program.run(b"aa", RunOptions::new(10_000))?;

    assert_eq!(first.output(), b"bc");
    assert_eq!(second.output(), b"bc");
    Ok(())
}
```

`(once)` consumption is runtime-local. Reusing `Program` is safe because parsed
programs are immutable; each run owns its own rule state.

The parser is byte-oriented. Comments may contain non-ASCII or even non-UTF-8
bytes because the library ignores bytes after `#` before validating executable
code:

```rust
use rsaeb::Program;

fn main() -> Result<(), rsaeb::ParseError> {
    let program = Program::parse(b"a=b#\xff\xfe\n")?;
    assert_eq!(program.rule_count(), 1);
    Ok(())
}
```

Trace output is library-owned data, not hard-coded side effects. State snapshots are owned per event and are only materialized when tracing is enabled:

```rust
use rsaeb::{Program, RunOptions};

fn main() -> Result<(), rsaeb::AebError> {
    let program = Program::parse("a=b\nb=(return)ok")?;
    let mut events = Vec::new();
    let result = program.run_with_trace(b"a", RunOptions::new(10_000), |event| {
        events.push(event);
    })?;

    assert_eq!(result.output(), b"ok");
    assert!(result.returned());
    assert_eq!(events.len(), 3);
    Ok(())
}
```

Trace step events carry borrowed `RuleInfo<'program>` metadata, not a cloned
display string and not a globally reusable identifier. The metadata is tied to
the `Program` that produced the trace:

```rust
use rsaeb::{Program, RunOptions, TraceEvent};

fn main() -> Result<(), rsaeb::AebError> {
    let program = Program::parse("a = b # comment")?;
    let mut applied_rule_index = None;

    program.run_with_trace(b"a", RunOptions::new(10_000), |event| {
        if let TraceEvent::Step { rule, .. } = event {
            assert_eq!(rule.line_number(), 1);
            assert_eq!(rule.compact_source(), b"a=b");
            applied_rule_index = Some(rule.index());
        }
    })?;

    let rule_index = applied_rule_index.expect("trace should apply a rule");
    let rule = program.rule(rule_index).expect("rule metadata should exist");
    assert_eq!(rule.index().as_usize(), 0);
    Ok(())
}
```

## Public API Surface

Constants:

- `DEFAULT_MAX_STEPS`: default rewrite step limit.

Program construction and execution:

- `Program`
- `Program::parse(source)`: parse reusable program bytes.
- `Program::parse_bytes(source)`: explicit byte parser.
- `Program::parse_str(source)`: explicit UTF-8 string parser.
- `Program::rule_count()`: count executable rules.
- `Program::run(input, options)`: execute without tracing.
- `Program::run_with_trace(input, options, callback)`: execute and receive
  trace events.
- `run(source, input, options)`: one-shot parse and execute helper.

Rule metadata:

- `RuleIndex`: program-local zero-based parsed-rule index.
- `RuleIndex::as_usize()`: zero-based rule position in parse order.
- `RuleInfo`: read-only parsed-rule metadata borrowed from a `Program`.
- `RuleInfo::index()`: return the program-local rule index.
- `RuleInfo::line_number()`: return the one-based source line number.
- `RuleInfo::compact_source()`: return whitespace-stripped executable code.
- `Program::rule(rule_index)`: read parsed rule metadata for tracing/debug UIs.
- `Program::rules()`: iterate parsed rule metadata in execution order.

Runtime configuration and result:

- `RunOptions`: currently holds the step limit.
- `RunOptions::new(max_steps)`: create options with an explicit step limit.
- `RunResult`: owns output bytes plus `steps` and `returned` metadata.
- `RunResult::output()`: borrow final output bytes.
- `RunResult::into_output()`: consume the result and return final output bytes.
- `RunResult::steps()`: return the number of applied rewrite steps.
- `RunResult::returned()`: report whether execution stopped by `(return)`.

Tracing:

- `TraceEvent<'program>`: `Initial` state and `Step` events emitted by tracing runs.
  `Step` carries `RuleInfo<'program>`.
- `TraceEvent::bytes()`: borrow the bytes carried by a trace event.

Errors:

- `AebError`: one-shot `run` union of `ParseError` and `RunError`.
- `ParseError`: structured source parse failure.
- `ParseError::line()`: one-based source line.
- `ParseError::column()`: one-based source column when available.
- `ParseError::kind()`: structured parse error kind.
- `ParseErrorKind`: concrete parse failure category.
- `PayloadKind`: identifies which payload rejected a reserved byte.
- `RunError`: structured runtime failure.
- `InputError`: runtime input validation failure.
- `InputError::column()`: one-based input column.
- `InputError::byte()`: rejected input byte.
- `StepLimitError`: step-limit failure with preserved runtime state.
- `StepLimitError::max_steps()`: configured limit.
- `StepLimitError::state()`: borrow state bytes at the limit.
- `StepLimitError::into_state()`: consume the error and return state bytes.

## Execution Semantics

Execution is ordered and single-step.

On each step, the runtime scans rules from top to bottom and applies the first
rule that matches the current state. For an unanchored non-empty left side, the
leftmost match in the current state is used. After one rewrite step, scanning
restarts from the first rule.

Example:

```text
program:
aa=x
a=y

input:
aaaa

output:
xx
```

The first rule is preferred over the second rule, and each application rewrites
the leftmost matching `aa`.

## Step Limit Semantics

`RunOptions::max_steps` is the maximum number of rewrite steps that may be
applied successfully. A run that becomes stable exactly at the limit succeeds.
The limit is an error only when another matching rule would need to be applied
after that many steps.

Examples:

```rust
use rsaeb::{Program, RunError, RunOptions};

fn main() -> Result<(), rsaeb::AebError> {
    let exact = Program::parse("a=b")?.run(b"a", RunOptions::new(1))?;
    assert_eq!(exact.output(), b"b");
    assert_eq!(exact.steps(), 1);

    let no_match = Program::parse("a=b")?.run(b"x", RunOptions::new(0))?;
    assert_eq!(no_match.output(), b"x");
    assert_eq!(no_match.steps(), 0);

    let would_apply = Program::parse("a=b")?.run(b"a", RunOptions::new(0));
    assert!(matches!(would_apply, Err(RunError::StepLimit(_))));
    Ok(())
}
```

## Program Format

A program source is a byte sequence containing one rewrite rule per non-empty
code line:

```text
lhs=rhs
```

Each line is parsed in this order:

1. `#` starts a comment. Everything from `#` to the end of the line is ignored.
2. Non-ASCII bytes are rejected in the remaining code part.
3. ASCII whitespace in the code part is removed completely.
4. Empty compact code is ignored.
5. Non-empty compact code must contain exactly one `=`.
6. The left side and right side are parsed as compact rule syntax.

Internally, the parser keeps these phases separate instead of passing a naked
`Vec<u8>` through every stage:

```text
raw line bytes
  -> CodeLine          # comment removed, code ASCII validated
  -> CompactCodeLine   # whitespace removed, original source columns retained
  -> Rule              # modifiers, anchors, payloads, and actions parsed
```

This keeps diagnostics tied to the original source columns even after whitespace
compaction. For example, `a = b = c` reports the second `=` at its original
source position, not at whatever column it happened to occupy after compaction.

Examples:

```text
a=b# this is parsed as a=b
#a=b  this whole line is a comment
a b = b b  # this is parsed as ab=bb
```

Non-ASCII text is allowed only in comments:

```text
a=b# 日本語コメントは許可
```

This is invalid because the non-ASCII byte is in code:

```text
a=あ
```

## Reserved Characters

The following characters are reserved in program code:

```text
= # ( )
```

Their meanings are fixed:

- `=` separates the left side from the right side.
- `#` starts a comment.
- `(` and `)` are only allowed as part of supported modifier/action tokens.

A second `=` in compact code is a parse error:

```text
a=b=c
```

A second `=` inside a comment is ignored:

```text
a=b#=c
```

Unsupported parenthesis usage is always a parse error:

```text
a=b(
a=b)
a=b()
a=()
a=b(start)
a=(once)b
a(once)=b
```

Because whitespace is removed from program code, spaces cannot be represented as
rule data. Because `=`, `#`, `(`, and `)` are reserved, they also cannot be
represented as rule data.

The input is different. Input bytes are runtime data, not program code. Input
must be ASCII, but it may contain whitespace and reserved characters. Rules
cannot match, create, or delete those bytes directly. The bytes themselves remain
runtime data, although nearby editable bytes may be inserted, removed, or moved
by ordinary rewrites.

Example:

```text
program: a=b
input:   a=()#c
output:  b=()#c
```

Rules also cannot match across preserved runtime-only bytes:

```text
program: ab=bb
input:   a bc
output:  a bc
```

## Left-Side Modifiers

The left side may start with one repeat modifier and one anchor modifier:

- `(once)`: the rule may be used at most once per runtime execution.
- `(start)`: the rule only matches at the start of the current state.
- `(end)`: the rule only matches at the end of the current state.

Supported modifier order is `(once)` first, then an optional anchor. Duplicated
or unsupported left-side modifier order is a parse error.

Examples:

```text
a=b
(once)a=b
(start)a=b
(end)a=b
(once)(start)a=b
```

Because code whitespace is ignored, this is also valid and equivalent to
`(once)(start)a=b`:

```text
( once ) ( start ) a = b
```

## Right-Side Actions

The right side selects the action for a matching rule:

- `text`: replace the matched left side with `text`.
- `(start)text`: remove the match and insert `text` at the start of the state.
- `(end)text`: remove the match and append `text` to the end of the state.
- `(return)text`: stop execution immediately and output `text`.

The action payload is still program data, so it cannot contain whitespace,
reserved characters, or non-ASCII bytes.

Examples:

```text
a=b
x=(start)y
x=(end)y
x=(return)y
```

## Empty Sides

The left side and right side may be empty.

An empty right side deletes the matched left side:

```text
a=
```

An empty left side matches an empty byte sequence. For unanchored rules and
`(start)` rules, it matches at the start of the current state:

```text
(once)=x
```

With input `ab`, this inserts `x` at the start and produces `xab`.

For `(end)` rules, an empty left side matches at the end of the current state:

```text
(once)(end)=x
```

With input `ab`, this inserts `x` at the end and produces `abx`.

An unanchored empty-left rule without `(once)`, `(return)`, or some later rule
that makes execution stop can rewrite forever until the step limit is reached.
That is legal syntax; execution remains governed by the configured step limit.

## Error Model

The library error model is intentionally split:

```rust
use rsaeb::{Program, RunError};

fn main() -> Result<(), rsaeb::AebError> {
    match Program::parse("a=b=c") {
        Err(parse_error) => assert_eq!(parse_error.line(), 1),
        Ok(_) => panic!("expected parse error"),
    }

    let run_error = Program::parse("a=b")?.run("aあ".as_bytes(), Default::default());

    if let Err(RunError::Input(input_error)) = run_error {
        assert_eq!(input_error.column(), 2);
    }

    Ok(())
}
```

Filesystem failures are not part of the library error model. External I/O must
be handled before bytes enter `Program::parse` or `run`.
