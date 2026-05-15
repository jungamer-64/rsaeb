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
parsed rules, runtime input state, per-run `(once)` state, run results, and
trace snapshots. It requires an allocator, but not `std`.

Allocation is explicit and fallible. Parser/runtime paths reserve explicitly and
report `AllocationError` instead of relying on accidental `Vec` growth. Runtime
expansion is also budgeted through `RunLimits`; the runtime checks size limits
before allocating oversized states or return outputs. Trace snapshot
materialization is budgeted separately through an explicit
`TraceSnapshotByteLimit`. Owned public values that contain byte buffers
intentionally do not implement `Clone`; copying bytes is an explicit
materialization step, not a hidden infallible API.

```sh
cargo check -p rsaeb --lib
```

A downstream `std` application can use the library normally. A downstream
`no_std` application must provide an allocator before calling APIs that allocate.

## Basic usage

Parse and run from UTF-8 source:

```rust
use rsaeb::{DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_STEPS, Program, ProgramSource, RunLimits, RunOutcome, RuntimeInput};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let program = Program::parse(ProgramSource::from_str("a=b"))?;
    let input = RuntimeInput::validate(b"a")?;
    let result = program.run(&input, RunLimits::new(DEFAULT_MAX_STEPS, DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN))?;
    assert!(matches!(
        result.outcome(),
        RunOutcome::Stable(output) if output.as_bytes() == b"b"
    ));
    Ok(())
}
```

Parse and run from raw source bytes:

```rust
use rsaeb::limits::StepLimit;
use rsaeb::{DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, Program, ProgramSource, RunLimits, RunOutcome, RuntimeInput};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let program = Program::parse(ProgramSource::from_bytes(b"a=b#\xff is allowed in comments\n"))?;
    let input = RuntimeInput::validate(b"a")?;
    let result = program.run(&input, RunLimits::new(StepLimit::new(10), DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN))?;
    assert!(matches!(
        result.outcome(),
        RunOutcome::Stable(output) if output.as_bytes() == b"b"
    ));
    Ok(())
}
```

Reusable parsed program with typed runtime input:

```rust
use rsaeb::limits::StepLimit;
use rsaeb::{DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, Program, ProgramSource, RunLimits, RunOutcome, RuntimeInput};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let program = Program::parse(ProgramSource::from_str("(once)a=b\na=c"))?;
    let limits = RunLimits::new(StepLimit::new(10_000), DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN);
    let input = RuntimeInput::validate(b"aa")?;

    let first = program.run(&input, limits)?;
    let second = program.run(&input, limits)?;

    assert!(matches!(
        first.outcome(),
        RunOutcome::Stable(output) if output.as_bytes() == b"bc"
    ));
    assert!(matches!(
        second.outcome(),
        RunOutcome::Stable(output) if output.as_bytes() == b"bc"
    ));
    Ok(())
}
```

`(once)` consumption is runtime-local. Reusing `Program` is safe because parsed
programs are immutable. Each execution owns runtime rule state derived directly
from the parsed rule list, so `(once)` state cannot drift away from rule order.

## Stepwise execution

Use `Program::start_execution` when a host needs to regain control after each
applied rule instead of running to completion in one call:

```rust
use rsaeb::{
    DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, ExecutionTransition, Program, ProgramSource,
    RunLimits, RuntimeInput,
};
use rsaeb::limits::StepLimit;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let program = Program::parse(ProgramSource::from_str("a=b\nb=c"))?;
    let input = RuntimeInput::validate(b"a")?;
    let execution = program.start_execution(
        &input,
        RunLimits::new(StepLimit::new(10), DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN),
    )?;

    let execution = match execution.step().map_err(|step| step.into_error())? {
        ExecutionTransition::Applied(applied) => {
            assert!(applied.state().bytes().eq(b"b".iter().copied()));
            applied.into_running()
        }
        ExecutionTransition::Stable(_) | ExecutionTransition::Returned(_) => {
            return Err("expected first applied step".into());
        }
    };

    let execution = match execution.step().map_err(|step| step.into_error())? {
        ExecutionTransition::Applied(applied) => {
            assert!(applied.state().bytes().eq(b"c".iter().copied()));
            applied.into_running()
        }
        ExecutionTransition::Stable(_) | ExecutionTransition::Returned(_) => {
            return Err("expected second applied step".into());
        }
    };

    match execution.step().map_err(|step| step.into_error())? {
        ExecutionTransition::Stable(stable) => {
            assert_eq!(stable.steps().get(), 2);
            assert!(stable.state().bytes().eq(b"c".iter().copied()));
        }
        ExecutionTransition::Applied(_) | ExecutionTransition::Returned(_) => {
            return Err("expected stable completion".into());
        }
    }
    Ok(())
}
```

`(return)` is a completion state, not an ordinary continuation step. Its output
is exposed as a borrowed parsed payload; callers that need ownership can
materialize it explicitly from the payload view:

```rust
use rsaeb::limits::StepLimit;
use rsaeb::{DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN, ExecutionTransition, Program, ProgramSource, RunLimits, RuntimeInput};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let program = Program::parse(ProgramSource::from_str("a=(return)ok"))?;
    let input = RuntimeInput::validate(b"a")?;
    let execution = program.start_execution(
        &input,
        RunLimits::new(StepLimit::new(10), DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN),
    )?;

    let owned_output = match execution.step().map_err(|step| step.into_error())? {
        ExecutionTransition::Returned(returned) => returned.output().to_vec()?,
        _ => Vec::new(),
    };

    assert_eq!(owned_output, b"ok");
    Ok(())
}
```

## Parser behavior

The parser is byte-oriented. Comments are removed before executable-code
validation, so comments can contain arbitrary non-ASCII bytes:

```rust
use rsaeb::{Program, ProgramSource};

fn main() -> Result<(), rsaeb::error::ParseError> {
    let program = Program::parse(ProgramSource::from_bytes(b"a=b#\xff\xfe\n"))?;
    assert_eq!(program.rule_count().get(), 1);
    Ok(())
}
```

ASCII whitespace in program code is ignored, but spaces in runtime input are
data:

```rust
use rsaeb::limits::StepLimit;
use rsaeb::{DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, Program, ProgramSource, RunLimits, RunOutcome, RuntimeInput};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let limits = RunLimits::new(StepLimit::new(10), DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN);
    let result = Program::parse(ProgramSource::from_str("ab=bb"))?
        .run(&RuntimeInput::validate(b"a bc")?, limits)?;
    assert!(matches!(
        result.outcome(),
        RunOutcome::Stable(output) if output.as_bytes() == b"a bc"
    ));
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
  -> RawSourceLine
  -> CodeLine                # comment removed, executable code ASCII validated
  -> CompactCodeLine         # whitespace removed, SourceColumn retained
  -> NonEmptyCompactCodeLine # empty compact lines cannot enter rule parsing
  -> RuleSyntaxLine          # exactly one '=' has been proven
  -> LeftSyntax / RightSyntax
  -> ProgramByte             # bytes that program code may construct and match

runtime input bytes
  -> AsciiByte         # runtime input domain validation
  -> RuntimeByte       # private ProgramConstructible(ProgramByte) or Opaque(AsciiByte)
```

The implementation follows the same boundaries as the data flow. Parser stages
live under `parser/`: source location construction, raw-line cleanup, compact
code lines, and left/right rule syntax are separate steps. Program-facing types
live under `program/`: resource limits, run results, parsed rule storage, and
trace convenience methods are separate from the `Program` entrypoint. Runtime
execution lives under `runtime/`: validated input materialization, mutable
state, rewrite scratch buffers, rule matching, `(once)` state, step budgeting,
and the execution loop are separate modules. These module boundaries are not a
second public API; they exist so the internal source of truth for each domain is
singular.

Program payloads are stored as `ProgramByte`, not raw `u8`. Runtime state is
stored as `RuntimeByte`: payload-compatible input and rule output become
editable program bytes, while whitespace, control bytes, and reserved syntax
bytes from input become opaque ASCII bytes. Ordinary rules match only editable bytes.
Opaque input bytes are preserved by surrounding rewrites but cannot be directly
matched, created, or deleted by program payloads. Runtime state is materialized
only at output boundaries, and the owned public values keep their purpose in the
type: stable states use `RuntimeStateSnapshot`, while `(return)` payloads use
`ReturnOutput`. During execution, the active state and the rewrite scratch
buffer are distinct typed buffers; the runtime swaps them only after a
successful continuation step, so a partially built rewrite cannot become the
committed state. `(once)` rules carry private slots assigned during parsing;
matching only observes whether a slot is still fresh, and only a committed
application can consume that slot. There is no public constructor for `(once)`
slots, so callers cannot forge indexes into the per-run table.

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
use rsaeb::{
    Program, ProgramSource, RunLimits, RuntimeInput,
};
use rsaeb::error::{LimitError, RunError, StateLimitContext};
use rsaeb::limits::{ReturnByteLimit, StateByteLimit, StepLimit};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let limits = RunLimits::new(
        StepLimit::new(10_000),
        StateByteLimit::new(1024),
        ReturnByteLimit::new(1024),
    );

    let limits = limits.with_state_byte_limit(StateByteLimit::new(2));
    let input = RuntimeInput::validate(b"")?;
    let error = Program::parse(ProgramSource::from_str("=a"))?.run(&input, limits);
    assert!(matches!(
        error,
        Err(RunError::Limit(LimitError::State {
            context: StateLimitContext::Rewrite,
            limit,
            attempted_len,
        })) if limit == StateByteLimit::new(2)
            && attempted_len.get() == 3
    ));

    Ok(())
}
```

Execution may succeed exactly at the step limit. The step limit becomes an error
only when another rule would still apply after the configured number of steps.

```rust
use rsaeb::{
    DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, Program, ProgramSource,
    RunLimits, RunOutcome, RuntimeInput,
};
use rsaeb::error::{LimitError, RunError};
use rsaeb::limits::StepLimit;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let exact_limits = RunLimits::new(StepLimit::new(1), DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN);
    let exact = Program::parse(ProgramSource::from_str("a=b"))?.run(
        &RuntimeInput::validate(b"a")?,
        exact_limits,
    )?;
    assert!(matches!(
        exact.outcome(),
        RunOutcome::Stable(output) if output.as_bytes() == b"b"
    ));
    assert_eq!(exact.steps().get(), 1);

    let no_match_limits = RunLimits::new(StepLimit::new(0), DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN);
    let no_match = Program::parse(ProgramSource::from_str("a=b"))?.run(
        &RuntimeInput::validate(b"x")?,
        no_match_limits,
    )?;
    assert!(matches!(
        no_match.outcome(),
        RunOutcome::Stable(output) if output.as_bytes() == b"x"
    ));
    assert_eq!(no_match.steps().get(), 0);

    let would_apply_limits = RunLimits::new(StepLimit::new(0), DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN);
    let would_apply = Program::parse(ProgramSource::from_str("a=b"))?.run(
        &RuntimeInput::validate(b"a")?,
        would_apply_limits,
    );
    assert!(matches!(
        would_apply,
        Err(RunError::Limit(LimitError::Step {
            max_steps,
            completed_steps,
            state_len,
        })) if max_steps == StepLimit::new(0)
            && completed_steps.get() == 0
            && state_len.get() == 1
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
use rsaeb::inspect::{RuleActionView, RuleAnchor, RuleRepeat};
use rsaeb::{Program, ProgramSource};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let program = Program::parse(ProgramSource::from_str("( once ) ( start ) a = ( end ) b # comment"))?;
    let rule = program.rules().next().ok_or("missing parsed rule")?;

    assert_eq!(rule.position().number().get(), 1);
    assert_eq!(rule.line_number().get(), 1);
    assert_eq!(rule.repeat(), RuleRepeat::Once);
    assert_eq!(rule.anchor(), RuleAnchor::Start);
    assert!(rule.lhs().eq_bytes(b"a"));
    assert!(matches!(
        rule.action(),
        RuleActionView::MoveEnd(payload) if payload.eq_bytes(b"b")
    ));
    assert_eq!(rule.canonical_source()?, b"(once)(start)a=(end)b");
    Ok(())
}
```

## Tracing

Tracing has two layers.

Borrowed tracing is the allocation-free primitive. Events borrow the runtime
state or return payload only for the callback invocation:

```rust
use rsaeb::limits::StepLimit;
use rsaeb::trace::BorrowedTraceEvent;
use rsaeb::{DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN, Program, ProgramSource, RunLimits, RunOutcome, RuntimeInput};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let program = Program::parse(ProgramSource::from_str("a=b\nb=(return)ok"))?;
    let mut lengths = Vec::new();

    let limits = RunLimits::new(StepLimit::new(10), DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN);
    let result = program.run_with_borrowed_trace(&RuntimeInput::validate(b"a")?, limits, |event| {
        lengths.push(event.byte_count().get());
        if let BorrowedTraceEvent::Step { rule, .. } = event {
            let _line = rule.line_number();
        }
    })?;

    assert!(matches!(
        result.outcome(),
        RunOutcome::Return(output) if output.as_bytes() == b"ok"
    ));
    assert_eq!(lengths.as_slice(), &[1, 1, 2]);
    Ok(())
}
```

Trace snapshotting materializes state/output bytes into typed owned snapshots
under explicit `TraceSnapshotLimits`: runtime limits still govern interpreter
execution, while `TraceSnapshotByteLimit` governs one materialized event. Step
events still borrow `RuleView` from the parsed `Program`, so retained trace
snapshot events cannot outlive that program:

```rust
use rsaeb::limits::{StepLimit, TraceSnapshotByteLimit, TraceSnapshotLimits};
use rsaeb::trace::{TraceSnapshotEffect, TraceSnapshotEvent};
use rsaeb::{DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN, Program, ProgramSource, RunLimits, RunOutcome, RuntimeInput};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let program = Program::parse(ProgramSource::from_str("a=b\nb=(return)ok"))?;
    let mut events = Vec::new();

    let limits = TraceSnapshotLimits::new(
        RunLimits::new(StepLimit::new(10), DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN),
        TraceSnapshotByteLimit::new(1024),
    );
    let result = program.run_with_trace_snapshots(
        &RuntimeInput::validate(b"a")?,
        limits,
        |event| {
            events.push(event);
        },
    )?;

    assert!(matches!(
        result.outcome(),
        RunOutcome::Return(output) if output.as_bytes() == b"ok"
    ));
    let initial = events.first().ok_or("missing initial trace event")?;
    let first_step = events.get(1).ok_or("missing first step trace event")?;
    let second_step = events.get(2).ok_or("missing second step trace event")?;

    assert!(matches!(initial, TraceSnapshotEvent::Initial { state } if state.as_bytes() == b"a"));
    assert!(matches!(first_step, TraceSnapshotEvent::Step {
        effect: TraceSnapshotEffect::Continue { state },
        ..
    } if state.as_bytes() == b"b"));
    assert!(matches!(second_step, TraceSnapshotEvent::Step {
        effect: TraceSnapshotEffect::Return { output },
        ..
    } if output.as_bytes() == b"ok"));
    assert!(matches!(
        second_step,
        TraceSnapshotEvent::Step {
            effect: TraceSnapshotEffect::Return { .. },
            ..
        }
    ));
    Ok(())
}
```

Fallible borrowed sinks use `try_run_with_borrowed_trace`, which separates
runtime errors and trace-sink errors with `TracedRunError`. Snapshot tracing has
one more failure domain: `run_with_trace_snapshots` returns
`TraceSnapshotRunError`, and `try_run_with_trace_snapshots` returns
`FallibleTraceSnapshotRunError` so runtime failures, snapshot materialization
failures, and callback failures cannot collapse into one variant.

## Error model

The library error model is intentionally split:

```rust
use rsaeb::error::RuntimeInputError;
use rsaeb::limits::StepLimit;
use rsaeb::{DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, Program, ProgramSource, RuntimeInput, RunLimits};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let limits = RunLimits::new(StepLimit::new(10), DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN);
    match Program::parse(ProgramSource::from_str("a=b=c")) {
        Err(parse_error) => assert_eq!(parse_error.line().get(), 1),
        Ok(_) => return Err("expected parse error".into()),
    }

    let input_error = RuntimeInput::validate("aあ".as_bytes());

    if let Err(RuntimeInputError::NonAscii { column, .. }) = input_error {
        assert_eq!(column.get(), 2);
    }

    Ok(())
}
```

Allocation failures are structured:

```rust
use rsaeb::error::{AllocationContext, RunError, TraceSnapshotError};

fn inspect_run(error: RunError) {
    if let RunError::Allocation(error) = error {
        match error.context() {
            AllocationContext::RuntimeRewriteState => {
                eprintln!("failed to allocate next rewrite state");
            }
            _ => {}
        }
    }
}

fn inspect_snapshot(error: TraceSnapshotError) {
    if let TraceSnapshotError::Allocation(error) = error {
        if error.context() == AllocationContext::TraceSnapshot {
            eprintln!("failed to allocate trace snapshot");
        }
    }
}
```

State length arithmetic overflow is separate from allocation failure and is
reported as `RunError::StateSize`. Configured byte budgets and step budgets are
reported as `RunError::Limit(LimitError::...)`. Runtime input validation owns
the typed input bytes, and stepwise execution uses terminal types, so the old
runtime invariant error path is no longer part of the public execution model.
Step-limit errors report the last state length, not the state bytes, so
reporting the step limit cannot turn into an allocation failure. Trace snapshot
byte limits are reported through `TraceSnapshotError`, not `RunError::Limit`,
because snapshot materialization is outside runtime execution. Use borrowed
tracing when the last state bytes are needed for diagnostics.
Filesystem failures are not part of the library error model. External I/O must
be handled before bytes enter `ProgramSource::from_bytes`,
`ProgramSource::from_str`, or `RuntimeInput::validate`.

## Public API overview

The generated rustdoc is the complete API reference. The crate root is kept to
the primary execution path:

- `ProgramSource`
- `RuntimeInput`
- `RuntimeInput::validate(bytes)`
- `Program`
- `RunningExecution`
- `ExecutionTransition`
- `AppliedExecution`
- `StableExecution`
- `ReturnedExecution`
- `ExecutionStepError`
- `RunLimits`
- `RunResult`
- `RunOutcome`
- `RuntimeStateSnapshot`
- `ReturnOutput`
- `DEFAULT_MAX_STEPS`
- `DEFAULT_MAX_STATE_LEN`
- `DEFAULT_MAX_RETURN_LEN`

Secondary domains live under explicit namespaces:

- `rsaeb::limits`: `StepLimit`, `StateByteLimit`, `ReturnByteLimit`,
  `TraceSnapshotByteLimit`, `TraceSnapshotLimits`, byte-count value types,
  `StepCount`, and `DEFAULT_MAX_TRACE_SNAPSHOT_LEN`
- `rsaeb::inspect`: `RuleView`, `RuleActionView`, `PayloadView`, rule
  position/count types, `RuleRepeat`, and `RuleAnchor`
- `rsaeb::trace`: borrowed trace events/effects, snapshot trace events/effects,
  and `RuntimeStateView`
- `rsaeb::error`: parse, input, runtime, allocation, limit, and trace error
  types, including rejected-byte diagnostic value types
- `rsaeb::source`: source-position value types used by parser diagnostics
