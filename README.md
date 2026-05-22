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

## Quick Start

Parse source into an immutable `Program`, validate runtime input, and run with
explicit limits:

```rust
use rsaeb::limits::{
    DEFAULT_MAX_INPUT_LEN, DEFAULT_PARSE_LIMITS, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_STEPS,
};
use rsaeb::{Program, ProgramSource, RunLimits, RunOutcome, RuntimeInput, RuntimeInputSource};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let program = Program::parse(ProgramSource::from_text("a=b"), DEFAULT_PARSE_LIMITS)?;
    let limits = RunLimits::new(DEFAULT_MAX_STEPS, DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN);
    let input = RuntimeInput::validate(RuntimeInputSource::from_bytes(b"a"), DEFAULT_MAX_INPUT_LEN)?;
    let result = program.run(input, limits)?;

    assert!(matches!(
        result.outcome(),
        RunOutcome::Stable(output) if output.as_slice() == b"b"
    ));

    Ok(())
}
```

Construct `ProgramSource` explicitly with `ProgramSource::from_text` or
`ProgramSource::from_bytes`; there is no implicit source conversion at the API
boundary. Use `from_bytes` when source comments may contain non-UTF-8 bytes.
Reuse parsed programs freely: a `Program` is immutable, and `(once)`
consumption is local to each execution.

## Execution APIs

The primary execution path is:

1. Construct `ProgramSource` with `from_text` or `from_bytes`.
2. Parse it with `Program::parse`.
3. Label host bytes with `RuntimeInputSource::from_bytes` and validate them with `RuntimeInput::validate`.
4. Consume `RuntimeInput` with `Program::run` or `Program::start_run`.

The crate intentionally contains no filesystem, process, stdout/stderr,
argument parsing, environment access, or lossy display boundary. Hosts should
perform I/O outside the interpreter and pass already-loaded bytes to
`ProgramSource` and `RuntimeInput`.

### Stepwise Execution

Use `Program::start_run` when a host needs control after each applied
rule instead of running to completion in one call. The public typestate API
lives under `rsaeb::execution`: only `RunSession` can step, while
`AppliedStep`, `StableRun`, and `ReturnedRun` represent
post-step states. `(return)` is terminal, not an ordinary continuation step.
Running, applied, and stable executions expose borrowed `RuntimeStateView`
values for observation. A failed step returns `RunStepError`, preserving
the uncommitted execution so a host can inspect its state, retry with
replacement limits through `RunStepError::retry_with_limits`, or discard it
explicitly.

The docs.rs crate page contains a complete doctested stepwise example.

### Resource Limits

`ParseLimits` is the parser contract. It bounds source bytes, executable
code-line bytes, parsed payload bytes, and executable rule count before the
parser accepts host-provided source into the program domain.

`RunLimits` is the execution contract. Step count alone is not enough for a
rewrite system because a short run can still expand state aggressively.

```rust
use rsaeb::error::{LimitError, RunError};
use rsaeb::limits::{
    DEFAULT_MAX_INPUT_LEN, DEFAULT_PARSE_LIMITS, DEFAULT_MAX_RETURN_LEN, DEFAULT_MAX_STATE_LEN, StepLimit,
};
use rsaeb::{Program, ProgramSource, RunLimits, RuntimeInput, RuntimeInputSource};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let limits = RunLimits::new(StepLimit::new(0), DEFAULT_MAX_STATE_LEN, DEFAULT_MAX_RETURN_LEN);
    let input = RuntimeInput::validate(RuntimeInputSource::from_bytes(b"a"), DEFAULT_MAX_INPUT_LEN)?;
    let result = Program::parse(ProgramSource::from_text("a=b"), DEFAULT_PARSE_LIMITS)?.run(input, limits);

    assert!(matches!(
        result,
        Err(RunError::Limit(LimitError::Step { completed_steps, .. }))
            if completed_steps.get() == 0
    ));

    Ok(())
}
```

Execution may succeed exactly at the step limit. The step limit becomes an
error only when another rule would still apply after the configured number of
completed steps.

Runtime input validation is bounded by `RuntimeInputByteLimit` before the
interpreter materializes owned input state. Trace snapshot materialization has
its own `TraceSnapshotByteLimit` because tracing is outside runtime execution.

### Tracing

Tracing has two layers:

- Borrowed tracing does not materialize owned event snapshots. Events borrow
  runtime state or return payload bytes only for the callback invocation.
- Snapshot tracing materializes owned event bytes under `TraceSnapshotLimits`.

Fallible borrowed sinks use `try_run_with_borrowed_trace`, which separates
runtime errors from trace-sink errors with `TracedRunError`. Snapshot tracing
adds one more failure domain: `run_with_trace_snapshots` returns
`TraceSnapshotRunError`, and `try_run_with_trace_snapshots` returns
`FallibleTraceSnapshotRunError`.

Parsed rule views inside trace events borrow from the parsed `Program`, so
retained trace events cannot outlive that program.

## A=B Language Reference

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

Examples:

```text
a=b# this is parsed as a=b
#a=b  this whole line is a comment
a b = b b  # this is parsed as ab=bb
```

Comments may contain arbitrary non-ASCII or non-UTF-8 bytes when source is
provided with `ProgramSource::from_bytes`. Executable code outside comments must
be ASCII. ASCII control bytes are invalid in executable code except for ASCII
whitespace that is removed during compaction.

Parse error columns are one-based byte positions in the original source line
before whitespace compaction. Diagnostics point at the user's source text, not
at the internal compacted representation.

### Reserved Characters

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
those bytes directly.

```text
program: a=b
input:   a=()#c
output:  b=()#c
```

Rules cannot match across preserved runtime-only bytes:

```text
program: ab=bb
input:   a bc
output:  a bc
```

`(return)` stops execution and replaces the final output with its return
payload, so runtime-only input bytes are not preserved after a matching return
rule:

```text
program: a=(return)x
input:   a=()#c
output:  x
```

### Left-Side Modifiers

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

### Right-Side Actions

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

### Empty Sides

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

### Ordered Execution

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

## Byte-Domain Boundary

Program source and runtime input are deliberately different byte domains:

- Program code is compact printable ASCII syntax.
- ASCII whitespace in program code is ignored before parsing.
- `#` starts a comment for the rest of the source line.
- Comments may contain non-ASCII or non-UTF-8 bytes.
- Executable code outside comments must be ASCII.
- Program payloads cannot contain whitespace, `=`, `#`, `(`, `)`, non-ASCII
  bytes, or ASCII control bytes.
- Runtime input is ASCII data and may contain spaces, ASCII control bytes, and
  reserved syntax bytes.
- Normal rewrites preserve runtime-only bytes that program code cannot
  construct or match.
- `(return)` stops execution and replaces the whole output with its return
  payload.

Internally, parser and runtime phases stay separate instead of passing raw byte
buffers through every stage:

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
  -> RuntimeByte       # private ProgramConstructible(ProgramByte) or Opaque(NonProgramAsciiByte)
  -> RunSession        # consumes RuntimeInput and owns mutable execution state
```

Program payloads are stored as `ProgramByte`, not raw `u8`. Runtime state is
stored as `RuntimeByte`: payload-compatible input and rule output become
editable program bytes, while whitespace, control bytes, and reserved syntax
bytes from input become opaque ASCII bytes. Ordinary rules match only editable
bytes. Opaque input bytes are preserved by surrounding rewrites but cannot be
directly matched, created, or deleted by program payloads.

Runtime input and runtime state stay in the typed byte domain during execution. Public
observation crosses an explicit materialization boundary: `RuntimeStateView`
materializes to `RuntimeStateSnapshot`, stable run results use
`RuntimeStateSnapshot`, `(return)` outputs use `ReturnOutput`, parsed payload
inspection materializes to `PayloadBytes`, and snapshot tracing materializes
owned event bytes under `TraceSnapshotLimits`. During execution, the active
state and the rewrite scratch buffer are distinct typed buffers, and the
runtime swaps them only after a successful continuation step. `(once)` rules
carry private slots assigned during parsing; only a committed application can
consume that slot.

## `no_std + alloc` Boundary

The library crate is `#![no_std]` and uses `alloc` only at owned-buffer
boundaries such as parsed rules, runtime input validation, per-run `(once)` state,
run results, canonical rule source, explicit view materialization, and trace
snapshots. It requires an allocator, but not `std`.

Allocation is explicit and fallible. Parser/runtime paths reserve explicitly
and report `AllocationError` instead of relying on accidental `Vec` growth.
Runtime expansion is budgeted through `RunLimits`; the runtime checks size
limits before allocating oversized states or return outputs. Trace snapshot
materialization is budgeted separately through `TraceSnapshotByteLimit`.
Internal parser/runtime witnesses are borrowed slices or typed indexes; they do
not allocate just to strengthen invariants.

Owned public values that contain byte buffers intentionally do not implement
`Clone`; copying bytes is an explicit materialization step, not a hidden
infallible API. Parser payload validation is reported before payload storage
allocation, so invalid source bytes are not hidden behind allocation failures.

A downstream `std` application can use the library normally. A downstream
`no_std` application must provide an allocator before calling APIs that
allocate.

## Error Model

The library error model is intentionally split. Parse errors, runtime input
errors, runtime execution errors, allocation errors, configured limit errors,
and trace materialization errors have separate structured types under
`rsaeb::error`.

Allocation failures preserve the allocation boundary as `AllocationContext`.
Reservation failures also report a typed `RequestedCapacity`, so hosts can
distinguish failures while validating input, materializing state views,
building canonical rule source, producing final output, or retaining trace
snapshots without parsing display strings.

State length arithmetic overflow is separate from allocation failure and is
reported as `RunError::StateSize`. Configured byte budgets and step budgets are
reported as `RunError::Limit(LimitError::...)`. Trace snapshot byte limits are
reported through `TraceSnapshotError`, not `RunError::Limit`, because snapshot
materialization is outside runtime execution.

Filesystem failures are not part of the library error model. External I/O must
be handled before bytes enter `ProgramSource::from_bytes`,
`ProgramSource::from_text`, or `RuntimeInputSource::from_bytes`.

## Public API Overview

The generated rustdoc is the complete API reference. The crate root is kept to
the primary execution path:

- `ProgramSource`
- `RuntimeInputSource`
- `RuntimeInput`
- `Program`
- `ParseLimits`
- `RunLimits`
- `RunResult`
- `RunOutcome`
- `RuntimeStateSnapshot`
- `ReturnOutput`
- `RuntimeInput::validate(RuntimeInputSource::from_bytes(bytes), limit)`

Secondary domains live under explicit namespaces:

- `rsaeb::limits`: `ParseLimits`, `SourceByteLimit`, `CodeLineByteLimit`,
  `PayloadByteLimit`, `RuleLimit`, `StepLimit`, `RuntimeInputByteLimit`,
  `RuntimeStateByteLimit`, `ReturnByteLimit`, `TraceSnapshotByteLimit`,
  `TraceSnapshotLimits`, parser/runtime byte-count value types, `StepCount`,
  and default budget constants
- `rsaeb::execution`: `RunSession`, `StepTransition`,
  `AppliedStep`, `StableRun`, `ReturnedRun`,
  and `RunStepError`
- `rsaeb::inspect`: `RuleView`, `RuleActionView`, `PayloadView`,
  `PayloadBytes`, `CanonicalRuleSource`, rule position/count types,
  `OnceRuleCount`, `RuleRepeat`, and `RuleAnchor`
- `rsaeb::trace`: borrowed trace events/effects, snapshot trace events/effects,
  and `RuntimeStateView`
- `rsaeb::error`: parse, input, runtime, allocation, limit, and trace error
  types, including rejected-byte diagnostic value types and
  `RequestedCapacity`
- `rsaeb::source`: source-position value types used by parser diagnostics

## Development Checks

Run the public documentation and package checks before publishing changes:

```sh
rustup target add thumbv7em-none-eabihf
cargo fmt --check
cargo check --lib --all-features --target thumbv7em-none-eabihf
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo test --doc --all-features
latest_rlib="$(find target/debug/deps -maxdepth 1 -name 'librsaeb-*.rlib' -printf '%T@ %p\n' | sort -nr | awk 'NR == 1 { print $2 }')"
rustdoc --edition=2024 --test README.md -L dependency=target/debug/deps --extern "rsaeb=${latest_rlib}"
RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps
cargo package --list
cargo package
```
