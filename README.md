# A=B Interpreter

A small Rust command-line interpreter for ordered `lhs=rhs` rewrite
programs. The interpreter treats the program and input as byte/string data,
then repeatedly applies the first eligible rule until no rule matches, a rule
returns, or the configured step limit is reached.

## Usage

Run through Cargo:

```sh
cargo run -- <program-file> [input] [--max-steps N] [--trace]
```

The current CLI help text reports the binary usage as:

```text
usage: osr <program-file> [input] [--max-steps N] [--trace]
```

Arguments:

- `<program-file>`: path to the rewrite program.
- `[input]`: optional initial input string. If omitted, the input is empty.
- `--max-steps N`: maximum rewrite steps before execution fails. The default
  is `1000000`.
- `--trace`: print the initial state, each applied rule, and final execution
  metadata to stderr.

## Program Format

A program is a text file containing one rewrite rule per non-empty line:

```text
lhs=rhs
```

Blank lines are ignored. A non-empty line without `=` is a parse error.

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

### Right-Side Actions

The right side selects the action for a matching rule:

- `text`: replace the matched left side with `text`.
- `(start)text`: remove the match and insert `text` at the start of the state.
- `(end)text`: remove the match and append `text` to the end of the state.
- `(return)text`: stop execution immediately and output `text`.

## Execution Semantics

At each step, rules are scanned from top to bottom. The first rule that is both
eligible and matching is applied.

For unanchored rules, the leftmost match in the current state is used. For
anchored rules, only the selected edge of the state is checked.

Execution stops when:

- no eligible rule matches the current state;
- a rule with `(return)` is applied;
- the `--max-steps` limit is reached.

If the step limit is reached, the interpreter exits with an error and reports
the state at the limit.

## Examples

### Basic Replacement

Program:

```text
aa=x
a=y
```

If saved as `examples/basic.ab`, run:

```sh
cargo run -- examples/basic.ab aaaa
```

Output:

```text
xx
```

The first rule is scanned first and rewrites `aa` to `x`, so `aaaa` becomes
`xx`.

### Explicit Return

Program:

```text
aa=(return)ok
a=x
```

If saved as `examples/return.ab`, run:

```sh
cargo run -- examples/return.ab aabb
```

Output:

```text
ok
```

The `(return)` action discards the current state and returns its right-side
payload.

## Current Status

In the current working tree used for this README update, `cargo test` reports
11 passed tests and 0 failed tests.
