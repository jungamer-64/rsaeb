# rsaeb

`rsaeb` is a Rust 2024 `no_std + alloc`, byte-oriented interpreter for A=B
ordered rewrite programs.

A=B: <https://store.steampowered.com/app/1720850/AB/>

## Unofficial Project Notice

This project is an unofficial, independently developed interpreter library. It
is not affiliated with, endorsed by, or maintained by Artless Games or the
original A=B author.

A=B's compact `lhs=rhs` ordered rewrite system is an unusually elegant
programming-puzzle idea. This crate exists because that design is worth
studying, testing, and reimplementing. If this interpreter interests you,
please support the original game.

## Design boundary

The important split is deliberately strict: program source and runtime input are
different byte domains.

- Program code is compact printable ASCII syntax.
- ASCII whitespace in program code is ignored before parsing.
- `#` starts a comment for the rest of the source line.
- Comments may contain non-ASCII or non-UTF-8 bytes.
- Executable code outside comments must be ASCII.
- Program payloads cannot contain whitespace, `=`, `#`, `(`, `)`, non-ASCII
  bytes, or ASCII control bytes.
- Runtime input is ASCII data and may contain spaces, ASCII control bytes, and
  reserved syntax bytes.
- Normal rewrites preserve runtime-only bytes that program code cannot construct
  or match.
- `(return)` stops execution and replaces the whole output with its return
  payload.

The crate intentionally contains no filesystem, process, stdout/stderr,
argument parsing, environment access, or lossy display boundary. Those belong in
a CLI or host application, not in the interpreter core.

## `no_std + alloc` boundary

The library crate is `#![no_std]` and uses `alloc` for owned buffers such as
parsed rules, runtime input state, per-run `(once)` state, results, preserved
step-limit states, and trace snapshots. It requires an allocator, but not
`std`.

Allocation is explicit and fallible. Parser/runtime paths reserve explicitly and
report `AllocationError` instead of relying on accidental `Vec` growth. Runtime
expansion is also budgeted through `RunLimits`; the runtime checks size limits
before allocating oversized states, return outputs, or trace snapshots. Owned
public values that contain byte buffers intentionally do not implement `Clone`;
copying bytes is an explicit materialization step, not a hidden infallible API.

```sh
cargo check -p rsaeb --lib
```

A downstream `std` application can use the library normally. A downstream
`no_std` application must provide an allocator before calling APIs that allocate.

## Basic usage

One-shot parse and run from UTF-8 source:

```rust
use rsaeb::{run_str, RunLimits};

fn main() -> Result<(), rsaeb::AebError> {
    let result = run_str("a=b", b"a", RunLimits::default())?;
    assert_eq!(result.output(), b"b");
    Ok(())
}
```

One-shot parse and run from raw source bytes:

```rust
use rsaeb::{run_bytes, RunLimits};

fn main() -> Result<(), rsaeb::AebError> {
    let result = run_bytes(b"a=b#\xff is allowed in comments\n", b"a", RunLimits::new(10))?;
    assert_eq!(result.output(), b"b");
    Ok(())
}
```

Reusable parsed program:

```rust
use rsaeb::{Program, RunLimits};

fn main() -> Result<(), rsaeb::AebError> {
    let program = Program::parse_str("(once)a=b\na=c")?;

    let first = program.run(b"aa", RunLimits::new(10_000))?;
    let second = program.run(b"aa", RunLimits::new(10_000))?;

    assert_eq!(first.output(), b"bc");
    assert_eq!(second.output(), b"bc");
    Ok(())
}
```

`(once)` consumption is runtime-local. Reusing `Program` is safe because parsed
programs are immutable; each run owns its own compact `(once)` state table. Only
`(once)` rules get state entries, so ordinary rules do not inflate runtime state
just by existing.

## Parser behavior

The parser is byte-oriented. Comments are removed before executable-code
validation, so comments can contain arbitrary non-ASCII bytes:

```rust
use rsaeb::Program;

fn main() -> Result<(), rsaeb::ParseError> {
    let program = Program::parse_bytes(b"a=b#\xff\xfe\n")?;
    assert_eq!(program.rule_count(), 1);
    Ok(())
}
```

ASCII whitespace in program code is ignored, but spaces in runtime input are
data:

```rust
use rsaeb::{Program, RunLimits};

fn main() -> Result<(), rsaeb::AebError> {
    let result = Program::parse_str("ab=bb")?.run(b"a bc", RunLimits::new(10))?;
    assert_eq!(result.output(), b"a bc");
    Ok(())
}
```

The reserved syntax bytes `=`, `#`, `(`, and `)` cannot appear in executable
payloads. They may appear in runtime input and will be preserved unless a rule
rewrites around them or `(return)` replaces the whole output.

Parse error columns are one-based byte positions in the original source line
before whitespace compaction. Diagnostics point at the user's source text, not
at the internal compacted representation.

## Program format

A program source is a byte sequence containing one rewrite rule per non-empty
code line:

```text
lhs=rhs
```

Each line is parsed in this order:

1. `#` starts a comment. Everything from `#` to the end of the line is ignored.
2. Non-ASCII bytes are rejected in the remaining code part.
3. ASCII whitespace in the code part is removed completely.
4. Remaining non-whitespace code bytes must be printable ASCII.
5. Empty compact code is ignored.
6. Non-empty compact code must contain exactly one `=`.
7. The left side and right side are parsed as compact rule syntax.

Internally, parser/runtime phases stay separate instead of passing a naked
`Vec<u8>` through every stage:

```text
raw line bytes
  -> code bytes        # comment removed, executable code ASCII validated
  -> compact code      # whitespace removed, source columns retained
  -> compact syntax    # tokens such as =, (once), (start), (end), (return)
  -> ProgramByte       # bytes that program code may construct and match

runtime input bytes
  -> AsciiByte         # runtime input domain validation
  -> RuntimeByte       # private Editable(ProgramByte) or Opaque(AsciiByte)
```

Program payloads are stored as `ProgramByte`, not raw `u8`. Runtime state is
stored as `RuntimeByte`: payload-compatible input and rule output become
editable program bytes, while whitespace, control bytes, and reserved syntax
bytes from input become opaque ASCII bytes. Ordinary rules match only editable bytes.
Opaque input bytes are preserved by surrounding rewrites but cannot be directly
matched, created, or deleted by program payloads. Runtime state is converted
back to public `Vec<u8>` only when returning results, traces, or errors. During
execution, the active state and the rewrite scratch buffer are distinct typed
buffers; the runtime swaps them only after a successful continuation step, so a
partially built rewrite cannot become the committed state.

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

ASCII control bytes are invalid in executable code, except for ASCII whitespace
that is removed during compaction. Runtime input is separate and may contain
ASCII control bytes as data.

## Reserved characters

The following characters are reserved in program code:

```text
= # ( )
```

Their meanings are fixed:

- `=` separates the left side from the right side.
- `#` starts a comment.
- `(` and `)` are only allowed as part of supported modifier/action tokens.

Internally, payload construction rejects all reserved syntax bytes at the
program-payload boundary. `=` and `#` are normally handled before payload
parsing, but they still cannot become payload data even if a future parser path
tries to feed them there. The implementation does not rely on “this should never
arrive here” as a safety boundary.

A second `=` in compact code is a parse error:

```text
a=b=c
```

A second `=` inside a comment is ignored:

```text
a=b#=c
```

Reserved syntax where payload data is expected is always a parse error:

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
rule data. Because `=`, `#`, `(`, and `)` are reserved, program payloads also
refuse them as rule data.

Runtime input is different. Input bytes are runtime data, not program code.
Input must be ASCII, but it may contain whitespace, ASCII control bytes, and
reserved characters. Ordinary rewrite actions cannot match, create, or delete
those bytes directly. The bytes themselves remain runtime data, although nearby
editable bytes may be inserted, removed, or moved.

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

`(return)` is intentionally different from ordinary rewrite actions. It stops
execution and replaces the final output with the return payload, so runtime-only
input bytes are not preserved after a matching return rule:

```text
program: a=(return)x
input:   a=()#c
output:  x
```

## Left-side modifiers

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

## Right-side actions

The right side selects the action for a matching rule:

- `text`: replace the matched left side with `text`.
- `(start)text`: remove the match and insert `text` at the start of the state.
- `(end)text`: remove the match and append `text` to the end of the state.
- `(return)text`: stop execution immediately and output `text`, discarding the
  current runtime state.

The action payload is still program data, so it cannot contain whitespace,
reserved characters, non-ASCII bytes, or ASCII control bytes. `(return)` can
therefore output only program-representable bytes, even if the discarded runtime
state contained spaces or reserved characters from the original input.

Examples:

```text
a=b
x=(start)y
x=(end)y
x=(return)y
```

## Empty sides

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
That is legal syntax; execution remains governed by `RunLimits`.

## Execution semantics

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

## Resource limits

`RunLimits` is the execution contract. Step count alone is not enough for a
rewrite system because a short run can still expand a state aggressively.

```rust
use rsaeb::{LimitError, Program, RunError, RunLimits, StateLimitContext};

fn main() -> Result<(), rsaeb::AebError> {
    let limits = RunLimits::bounded(
        10_000, // max steps
        1024,   // max runtime state length
        1024,   // max `(return)` output length
        1024,   // max trace snapshot bytes
    );

    let error = Program::parse_str("=a")?.run(b"", limits.with_max_state_len(2));
    assert!(matches!(
        error,
        Err(RunError::Limit(LimitError::State {
            context: StateLimitContext::Rewrite,
            limit: 2,
            attempted_len: 3,
        }))
    ));

    Ok(())
}
```

Execution may succeed exactly at the step limit. The step limit becomes an error
only when another rule would still apply after the configured number of steps.

```rust
use rsaeb::{LimitError, Program, RunError, RunLimits};

fn main() -> Result<(), rsaeb::AebError> {
    let exact = Program::parse_str("a=b")?.run(b"a", RunLimits::new(1))?;
    assert_eq!(exact.output(), b"b");
    assert_eq!(exact.steps(), 1);

    let no_match = Program::parse_str("a=b")?.run(b"x", RunLimits::new(0))?;
    assert_eq!(no_match.output(), b"x");
    assert_eq!(no_match.steps(), 0);

    let would_apply = Program::parse_str("a=b")?.run(b"a", RunLimits::new(0));
    assert!(matches!(
        would_apply,
        Err(RunError::Limit(LimitError::Step {
            max_steps: 0,
            completed_steps: 0,
            state_len: 1,
        }))
    ));
    Ok(())
}
```

## Rule inspection

Parsed rule inspection is structural. `RuleView` exposes repeat policy, anchor,
left payload, and right action directly. There is no stored `compact_source`
blob: canonical source is generated from the structured rule when requested.
This removes the second source of truth.

```rust
use rsaeb::{Program, RuleActionView, RuleAnchor, RuleRepeat};

fn main() -> Result<(), rsaeb::AebError> {
    let program = Program::parse_str("( once ) ( start ) a = ( end ) b # comment")?;
    let rule = program.rules().next().expect("one parsed rule");

    assert_eq!(rule.position().zero_based(), 0);
    assert_eq!(rule.line_number(), 1);
    assert_eq!(rule.repeat(), RuleRepeat::Once);
    assert_eq!(rule.anchor(), RuleAnchor::Start);
    assert!(rule.lhs().eq_bytes(b"a"));
    assert!(matches!(
        rule.action(),
        RuleActionView::MoveEnd(payload) if payload.eq_bytes(b"b")
    ));
    assert_eq!(
        rule.canonical_source().expect("canonical source"),
        b"(once)(start)a=(end)b"
    );
    Ok(())
}
```

## Tracing

Tracing has two layers.

Borrowed tracing is the allocation-free primitive. Events borrow the runtime
state or return payload only for the callback invocation:

```rust
use rsaeb::{BorrowedTraceEvent, Program, RunLimits};

fn main() -> Result<(), rsaeb::AebError> {
    let program = Program::parse_str("a=b\nb=(return)ok")?;
    let mut lengths = Vec::new();

    let result = program.run_with_borrowed_trace(b"a", RunLimits::new(10), |event| {
        lengths.push(event.len());
        if let BorrowedTraceEvent::Step { rule, .. } = event {
            let _line = rule.line_number();
        }
    })?;

    assert_eq!(result.output(), b"ok");
    assert_eq!(lengths.as_slice(), &[1, 1, 2]);
    Ok(())
}
```

Trace snapshotting materializes state/output bytes as `Vec<u8>` under
`RunLimits::max_trace_snapshot_len()`. Step events still borrow `RuleView` from
the parsed `Program`, so retained trace snapshot events cannot outlive that
program:

```rust
use rsaeb::{Program, RunLimits, TraceSnapshotEvent};

fn main() -> Result<(), rsaeb::AebError> {
    let program = Program::parse_str("a=b\nb=(return)ok")?;
    let mut events = Vec::new();

    let result = program.run_with_trace_snapshots(b"a", RunLimits::new(10), |event| {
        events.push(event);
    })?;

    assert_eq!(result.output(), b"ok");
    assert!(matches!(events.first(), Some(TraceSnapshotEvent::Initial { .. })));
    assert_eq!(events[0].bytes(), b"a");
    assert_eq!(events[1].bytes(), b"b");
    assert_eq!(events[2].bytes(), b"ok");
    assert!(events[2].is_return_step());
    Ok(())
}
```

Fallible sinks use `try_run_with_borrowed_trace` or
`try_run_with_trace_snapshots`. Runtime errors and trace-sink errors are
separated by `TracedRunError`.

## Error model

The library error model is intentionally split:

```rust
use rsaeb::{Program, RunError, RunLimits};

fn main() -> Result<(), rsaeb::AebError> {
    match Program::parse_str("a=b=c") {
        Err(parse_error) => assert_eq!(parse_error.line(), 1),
        Ok(_) => panic!("expected parse error"),
    }

    let run_error = Program::parse_str("a=b")?.run("aあ".as_bytes(), RunLimits::default());

    if let Err(RunError::Input(input_error)) = run_error {
        assert_eq!(input_error.column(), 2);
    }

    Ok(())
}
```

Allocation failures are structured:

```rust
use rsaeb::{AllocationContext, RunError};

fn inspect(error: RunError) {
    if let RunError::Allocation(error) = error {
        match error.context() {
            AllocationContext::RuntimeState => {
                eprintln!("failed to allocate next rewrite state");
            }
            AllocationContext::TraceSnapshot => {
                eprintln!("failed to allocate trace snapshot");
            }
            _ => {}
        }
    }
}
```

State length arithmetic overflow is separate from allocation failure and is
reported as `RunError::StateSize`. Configured byte budgets and step budgets are
reported as `RunError::Limit(LimitError::...)`. Step-limit errors report the
last state length, not the state bytes, so reporting the step limit cannot turn
into an allocation failure. Use borrowed tracing when the last state bytes are
needed for diagnostics.
Filesystem failures are not part of the library error model. External I/O must
be handled before bytes enter `Program::parse_bytes`, `Program::parse_str`,
`run_bytes`, or `run_str`.

## Public API surface

Constants:

- `DEFAULT_MAX_STEPS`
- `DEFAULT_MAX_STATE_LEN`
- `DEFAULT_MAX_RETURN_LEN`
- `DEFAULT_MAX_TRACE_SNAPSHOT_LEN`

Program construction and execution:

- `run_bytes(source, input, limits)`
- `run_str(source, input, limits)`
- `Program`
- `Program::parse_bytes(source)`
- `Program::parse_str(source)`
- `Program::rule_count()`
- `Program::once_rule_count()`
- `Program::rules()`
- `Program::run(input, limits)`
- `Program::run_with_borrowed_trace(input, limits, callback)`
- `Program::try_run_with_borrowed_trace(input, limits, callback)`
- `Program::run_with_trace_snapshots(input, limits, callback)`
- `Program::try_run_with_trace_snapshots(input, limits, callback)`

Runtime configuration and result:

- `RunLimits`
- `RunLimits::new(max_steps)`
- `RunLimits::bounded(max_steps, max_state_len, max_return_len, max_trace_snapshot_len)`
- `RunLimits::with_max_steps(max_steps)`
- `RunLimits::with_max_state_len(max_state_len)`
- `RunLimits::with_max_return_len(max_return_len)`
- `RunLimits::with_max_trace_snapshot_len(max_trace_snapshot_len)`
- `RunResult`
- `RunResult::output()`
- `RunResult::into_output()`
- `RunResult::steps()`
- `RunResult::termination()`
- `RunResult::returned()`
- `RunTermination`

Rule data:

- `RulePosition`
- `RuleRepeat`
- `RuleAnchor`
- `PayloadView<'program>` (`bytes() -> impl Iterator<Item = u8>`, `to_vec()`)
- `RuleActionView<'program>`
- `RuleView<'program>`
- `RuleView::canonical_source()`

Tracing:

- `RuntimeStateView<'run>` (`bytes() -> impl Iterator<Item = u8>`, `to_vec()`)
- `BorrowedTraceEvent<'program, 'run>`
- `BorrowedTraceEffect<'program, 'run>`
- `TraceSnapshotEvent<'program>`
- `TraceSnapshotEffect`
- `TracedRunError<E>`

Errors:

- `AebError`
- `ParseError`
- `ParseErrorKind`
- `PayloadKind`
- `LeftModifierKind`
- `RightActionKind`
- `RunError`
- `InputError`
- `AllocationError`
- `AllocationErrorKind`
- `AllocationContext`
- `StateSizeError`
- `LimitError`
- `StateLimitContext`
