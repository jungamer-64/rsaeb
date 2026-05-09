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
input are different domains. Program code is compact printable ASCII syntax.
Runtime input is ASCII data. Ordinary rewrites preserve input bytes that the
program syntax cannot write, such as spaces and reserved characters, except when
execution stops with `(return)`, which replaces the whole output with its return
payload.

## `no_std` Library Boundary

The library crate is `#![no_std]` and uses `alloc` for owned buffers such as
parsed rules, runtime input state, per-run `(once)` state, `RunResult`, trace
events, and step-limit error state. This means the interpreter core does not
depend on `std`, files, processes, host I/O streams, environment variables, or
OS error types. It still requires an allocator; this is `no_std + alloc`, not a
fixed-capacity heapless interpreter.

Allocation is deliberately fallible inside the library API. Parser/runtime paths
that allocate reserve explicitly and report `AllocationError` instead of relying
on accidental `Vec` growth. Internally, parsed program payloads and runtime
state are stored in distinct byte domains, so bytes constructible by code are
not confused with input-derived bytes such as spaces or reserved syntax
characters. Public `Vec<u8>` values are materialized only at API boundaries such
as final output, `(return)` output, step-limit errors, and trace snapshots.

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

Trace output is library-owned data, not hard-coded side effects. State snapshots
are owned per event and are only materialized when tracing is enabled:

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

Fallible trace sinks can use `try_run_with_trace`:

```rust
use rsaeb::{Program, RunOptions, TracedRunError};

fn main() -> Result<(), rsaeb::AebError> {
    let program = Program::parse("a=b\nb=c")?;
    let result = program.try_run_with_trace(b"a", RunOptions::new(10_000), |_event| {
        Err::<(), _>("trace sink full")
    });

    assert_eq!(result, Err(TracedRunError::Trace("trace sink full")));
    Ok(())
}
```

Trace step events carry borrowed `RuleView<'program>` data, not a cloned
display string and not a globally reusable lookup key. The view is borrowed
from the `Program` that produced the trace. There is intentionally no public
`Program::rule(index)` API: a numeric index cannot prove which program produced
it, so the library exposes rule data directly instead of accepting forged
handles back from callers. The view includes the parsed repeat policy, anchor,
left-side payload, and right-side action; `compact_source()` is display metadata,
not a source string consumers are expected to parse again.

```rust
use rsaeb::{Program, RuleActionView, RuleAnchor, RuleRepeat, RunOptions, TraceEffect, TraceEvent};

fn main() -> Result<(), rsaeb::AebError> {
    let program = Program::parse("a = b # comment")?;
    let first_rule = program.rules().next().expect("one parsed rule");

    assert_eq!(first_rule.position().zero_based(), 0);
    assert_eq!(first_rule.line_number(), 1);
    assert_eq!(first_rule.repeat(), RuleRepeat::Always);
    assert_eq!(first_rule.anchor(), RuleAnchor::Anywhere);
    assert!(first_rule.lhs().eq_bytes(b"a"));
    assert!(matches!(first_rule.action(), RuleActionView::Replace(payload) if payload.eq_bytes(b"b")));
    assert_eq!(first_rule.compact_source(), b"a=b");

    program.run_with_trace(b"a", RunOptions::new(10_000), |event| {
        if let TraceEvent::Step { rule, effect, .. } = event {
            assert_eq!(rule.position().zero_based(), 0);
            assert!(rule.lhs().eq_bytes(b"a"));
            assert!(matches!(rule.action(), RuleActionView::Replace(payload) if payload.eq_bytes(b"b")));
            assert_eq!(rule.compact_source(), b"a=b");
            assert!(matches!(effect, TraceEffect::Continue { .. }));
        }
    })?;

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
- `Program::rules()`: iterate borrowed structured parsed-rule views in execution order.
- `Program::run(input, options)`: execute without tracing.
- `Program::run_with_trace(input, options, callback)`: execute and receive
  infallible trace events.
- `Program::try_run_with_trace(input, options, callback)`: execute and receive
  fallible trace events.
- `run(source, input, options)`: one-shot parse and execute helper.

Rule data:

- `RulePosition`: program-local parsed-rule position used only as metadata.
- `RulePosition::zero_based()`: zero-based rule position in parse order.
- `RulePosition::one_based()`: one-based rule number for display.
- `RuleRepeat`: `Always` or `Once`.
- `RuleRepeat::is_once()`: report whether the repeat policy is `(once)`.
- `RuleAnchor`: `Anywhere`, `Start`, or `End`.
- `PayloadView<'program>`: read-only borrowed executable payload bytes.
- `PayloadView::len()`: payload byte length.
- `PayloadView::is_empty()`: report whether the payload is empty.
- `PayloadView::bytes()`: iterate payload bytes without allocating.
- `PayloadView::eq_bytes(expected)`: compare payload bytes without forcing callers to allocate.
- `RuleActionView<'program>`: read-only right-side action view, one of `Replace`, `MoveStart`, `MoveEnd`, or `Return`.
- `RuleActionView::payload()`: return the action payload.
- `RuleActionView::is_return()`: report whether the action is `(return)`.
- `RuleView<'program>`: read-only structured parsed-rule view borrowed from a `Program`.
- `RuleView::position()`: return the program-local metadata position.
- `RuleView::zero_based_position()`: return the zero-based position directly.
- `RuleView::line_number()`: return the one-based source line number.
- `RuleView::repeat()`: return the parsed repeat policy.
- `RuleView::anchor()`: return the parsed match anchor.
- `RuleView::lhs()`: return the parsed left-side payload.
- `RuleView::action()`: return the parsed right-side action.
- `RuleView::compact_source()`: return whitespace-stripped executable code for diagnostics/display.

Runtime configuration and result:

- `RunOptions`: opaque runtime configuration for one execution.
- `RunOptions::new(max_steps)`: create options with an explicit step limit.
- `RunOptions::max_steps()`: inspect the configured step limit.
- `RunResult`: owns output bytes plus `steps` and structured termination metadata.
- `RunResult::output()`: borrow final output bytes.
- `RunResult::into_output()`: consume the result and return final output bytes.
- `RunResult::steps()`: return the number of applied rewrite steps.
- `RunResult::termination()`: return `RunTermination::Stable` or `RunTermination::Return`.
- `RunResult::returned()`: convenience check for `(return)` termination.
- `RunTermination`: typed reason why execution stopped.

Tracing:

- `TraceEvent<'program>`: `Initial` state and `Step` events emitted by tracing
  runs. `Step` carries `RuleView<'program>` and a typed `TraceEffect`.
- `TraceEffect`: `Continue { state }` or `Return { output }`, instead of an
  ambiguous byte buffer plus a boolean.
- `TraceEffect::bytes()`: borrow the bytes carried by a step effect.
- `TraceEffect::is_return()`: report whether the step executed `(return)`.
- `TraceEvent::bytes()`: borrow the bytes carried by a trace event.
- `TraceEvent::is_return_step()`: report whether this event is a returning step.

Errors:

- `AebError`: one-shot `run` union of `ParseError` and `RunError`.
- `ParseError`: structured source parse failure.
- `ParseError::line()`: one-based source line.
- `ParseError::column()`: one-based source column when available.
- `ParseError::kind()`: structured parse error kind.
- `ParseErrorKind`: concrete parse failure category, including parser
  allocation failure, reserved syntax inside payload data, and unsupported right-side action syntax.
- `PayloadKind`: identifies which payload rejected reserved syntax as data.
- `RunError`: structured runtime failure.
- `TracedRunError<E>`: runtime-or-callback error for `try_run_with_trace`.
- `InputError`: runtime input validation failure.
- `AllocationError`: fallible allocation failure with an `AllocationContext` and
  requested capacity.
- `AllocationContext`: parser/runtime allocation site.
- `StateSizeError`: runtime state length arithmetic failure.
- `StepLimitError`: step-limit failure with preserved runtime state.

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

`RunOptions::max_steps()` returns the maximum number of rewrite steps that may be
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
4. Remaining non-whitespace code bytes must be printable ASCII.
5. Empty compact code is ignored.
6. Non-empty compact code must contain exactly one `=`.
7. The left side and right side are parsed as compact rule syntax.

Internally, the parser and runtime keep phases separate instead of passing a
naked `Vec<u8>` through every stage:

```text
raw line bytes
  -> CodeLine          # comment removed, code ASCII validated
  -> CompactCodeLine   # whitespace removed, printable code validated, columns retained
  -> compact syntax    # tokens such as =, (once), (start), (end), (return)
  -> CodeByte payloads # non-reserved bytes that program code is allowed to construct

runtime input bytes
  -> RuntimeByte state # ASCII data, including bytes code cannot construct

CodeByte payloads are converted into RuntimeByte only when a rewrite inserts
bytes into the runtime state. RuntimeByte state is converted back to public
Vec<u8> only when returning results, traces, or errors.
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

ASCII control bytes are also invalid in executable code, except for ASCII
whitespace that is removed during compaction. Runtime input is separate and may
contain ASCII control bytes as data.

## Reserved Characters

The following characters are reserved in program code:

```text
= # ( )
```

Their meanings are fixed:

- `=` separates the left side from the right side.
- `#` starts a comment.
- `(` and `)` are only allowed as part of supported modifier/action tokens.

Internally, payload construction rejects all reserved syntax bytes at the
`CodeByte` boundary. `=` and `#` are normally handled before payload parsing,
but they still cannot become payload data even if a future parser path tries to
feed them there. The implementation does not rely on “this should never arrive
here” as a safety boundary, because that sentence is how small libraries become
archaeological sites.

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
rule data. Because `=`, `#`, `(`, and `)` are reserved, `CodeByte` also refuses
them as rule data.

The input is different. Input bytes are runtime data, not program code. Input
must be ASCII, but it may contain whitespace, ASCII control bytes, and reserved
characters. Ordinary rewrite actions cannot match, create, or delete those bytes
directly. The bytes themselves remain runtime data, although nearby editable
bytes may be inserted, removed, or moved.

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

Allocation failures are structured:

```rust
use rsaeb::{AllocationContext, RunError};

fn inspect(error: RunError) {
    if let RunError::Allocation(error) = error {
        match error.context() {
            AllocationContext::RuntimeState => {
                eprintln!("failed to allocate next rewrite state");
            }
            AllocationContext::FinalOutput => {
                eprintln!("failed to materialize final output bytes");
            }
            AllocationContext::StepLimitState => {
                eprintln!("failed to materialize step-limit state bytes");
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
reported as `RunError::StateSize`. Filesystem failures are not part of the
library error model. External I/O must be handled before bytes enter
`Program::parse` or `run`.
