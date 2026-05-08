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

This crate exposes only the parser, runtime, tracing data, and structured error
types. External I/O and presentation formatting are outside the library
boundary. The library does not expose `std::io` errors, because the interpreter
does not read files.

Basic one-shot execution:

```rust
use rsaeb::{run, RunOptions};

let result = run("a=b", b"a", RunOptions::default())?;
assert_eq!(result.output(), b"b");
# Ok::<(), rsaeb::AebError>(())
```

Reusable parsed program:

```rust
use rsaeb::{Program, RunOptions};

let program = Program::parse("(once)a=b\na=c")?;

let first = program.run(b"aa", RunOptions::new(10_000))?;
let second = program.run(b"aa", RunOptions::new(10_000))?;

assert_eq!(first.output(), b"bc");
assert_eq!(second.output(), b"bc");
# Ok::<(), rsaeb::AebError>(())
```

`(once)` consumption is runtime-local. Reusing `Program` is safe because parsed
programs are immutable; each run owns its own rule state.

The parser is byte-oriented. Comments may contain non-ASCII or even non-UTF-8
bytes because the library ignores bytes after `#` before validating executable
code:

```rust
use rsaeb::Program;

let program = Program::parse(b"a=b#\xff\xfe\n")?;
assert_eq!(program.rule_count(), 1);
# Ok::<(), rsaeb::ParseError>(())
```

Trace output is library-owned data, not hard-coded side effects:

```rust
use rsaeb::{Program, RunOptions, TraceEvent};

let program = Program::parse("a=b\nb=(return)ok")?;
let mut events = Vec::new();
let result = program.run_with_trace(
    b"a",
    RunOptions::new(10_000),
    |event: TraceEvent| events.push(event),
)?;

assert_eq!(result.output(), b"ok");
assert!(result.returned());
assert_eq!(events.len(), 3);
# Ok::<(), rsaeb::AebError>(())
```

Trace step events carry a `RuleId`, not a cloned display string. Human-readable
rule text is metadata on `Program`:

```rust
use rsaeb::{Program, RunOptions, TraceEvent};

let program = Program::parse("a = b # comment")?;
let mut applied_rule = None;

program.run_with_trace(b"a", RunOptions::new(10_000), |event| {
    if let TraceEvent::Step { rule, .. } = event {
        applied_rule = Some(rule);
    }
})?;

let rule = program.rule(applied_rule.unwrap()).unwrap();
assert_eq!(rule.line_number(), 1);
assert_eq!(rule.compact_source(), b"a=b");
# Ok::<(), rsaeb::AebError>(())
```

Public API surface:

- `Program::parse(source)`: parse reusable program bytes.
- `Program::parse_bytes(source)`: explicit byte parser.
- `Program::parse_str(source)`: explicit UTF-8 string parser.
- `Program::run(input, options)`: execute without tracing.
- `Program::run_with_trace(input, options, callback)`: execute and receive
  trace events.
- `Program::rule(rule_id)`: read parsed rule metadata for tracing/debug UIs.
- `Program::rules()`: iterate parsed rule metadata.
- `run(source, input, options)`: one-shot parse and execute helper.
- `RunOptions`: currently holds the step limit.
- `RunResult`: owns output bytes plus `steps` and `returned` metadata.
- `ParseError`: structured source parse failure.
- `RunError`: structured runtime failure.
- `AebError`: one-shot `run` union of `ParseError` and `RunError`.

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
must be ASCII, but it may contain whitespace and reserved characters. Those
bytes are preserved through execution unless adjacent editable data is rewritten.
Rules cannot directly match, create, or delete spaces or reserved characters,
because the program syntax has no data representation for them.

Example:

```text
program: a=b
input:   a=()#c
output:  b=()#c
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
=x
```

This inserts `x` at the start of the state.

## Error Model

The library error model is intentionally split:

```rust
use rsaeb::{Program, RunError};

let parse_error = Program::parse("a=b=c").unwrap_err();
assert_eq!(parse_error.line(), 1);

let run_error = Program::parse("a=b")?
    .run("aあ".as_bytes(), Default::default())
    .unwrap_err();

if let RunError::Input(input_error) = run_error {
    assert_eq!(input_error.column(), 2);
}
# Ok::<(), rsaeb::AebError>(())
```

Filesystem failures are not part of the library error model. External I/O must
be handled before bytes enter `Program::parse` or `run`.
