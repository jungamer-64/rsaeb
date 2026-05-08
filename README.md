# A=B Interpreter

A small Rust 2024 command-line interpreter for ordered `lhs=rhs` rewrite
programs.

The important rule is simple: program code and runtime input are different
things. Program code is compact ASCII syntax. Runtime input is ASCII data. The
interpreter preserves input bytes that the program syntax cannot write, such as
spaces and reserved characters.

## Usage

Run through Cargo:

```sh
cargo run -- <program-file> [input] [--max-steps N] [--trace]
```

The binary usage is:

```text
usage: aeb <program-file> [input] [--max-steps N] [--trace]
```

Arguments:

- `<program-file>`: path to the rewrite program.
- `[input]`: optional initial input string. If omitted, the input is empty.
- `--max-steps N`: maximum rewrite steps before execution fails. The default is
  `1000000`.
- `--trace`: print the initial state, each applied rule, and final execution
  metadata to stderr.

## Program Format

A program is a text file containing one rewrite rule per non-empty code line:

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

For `(end)` rules, an empty left side matches at the end of the current state:

```text
(end)=x
```

This appends `x` to the end of the state.

Empty-left-side rules can always match, so they usually need careful ordering, a
terminating rule, or a step limit. A common default-return pattern is:

```text
success=(return)true
=(return)false
```

## Execution Semantics

At each step, rules are scanned from top to bottom. The first rule that is both
eligible and matching is applied.

For unanchored rules, the leftmost contiguous byte match in the current state is
used. For anchored rules, only the selected edge of the state is checked.

Ignored whitespace in the program does not let a rule skip over whitespace in
the input. Matching remains contiguous over the actual input bytes.

Example:

```text
program: a b=bb
input:   abc
output:  bbc
```

The program code `a b=bb` is compacted to `ab=bb`, so `ab` matches `abc`.

```text
program: a b=bb
input:   a bc
output:  a bc
```

The input contains a real space between `a` and `b`, so compact `ab` does not
match.

```text
program: ab=bb
input:   a bc
output:  a bc
```

The rule still does not match, for the same reason.

```text
program: a=b
input:   a bc
output:  b bc
```

Only the `a` is rewritten. The input space is preserved.

Execution stops when:

- no eligible rule matches the current state;
- a rule with `(return)` is applied;
- the `--max-steps` limit is reached.

If the step limit is reached, the interpreter exits with an error and reports
the state at the limit.

## Internal Safety Model

The implementation intentionally separates the byte domains:

- `CodeByte`: bytes that may exist as rule payload data. Reserved characters,
  whitespace, and non-ASCII bytes are impossible to construct through the parser.
- `Payload`: a sequence of `CodeByte`, used for rule left sides and right-side
  payloads.
- `RuntimeByte`: any ASCII input/runtime byte. This includes spaces and reserved
  characters.
- `State`: a sequence of `RuntimeByte`, used for execution state and final output.

So a rule payload cannot accidentally contain `=`, `#`, `(`, `)`, whitespace, or
non-ASCII data. Runtime state can still hold those ASCII bytes when they came
from input. In other words, the parser does not merely check the rule data and
then throw it back into an untyped `Vec<u8>` swamp. We do not need another tiny
byte swamp. Humanity has enough swamps.

## Development Checks

Run tests with:

```sh
cargo test
```

Useful source searches:

```sh
rg "Vec<u8>|CodeByte|RuntimeByte|Payload|State" src README.md
rg "reserved|whitespace|non-ASCII|multiple '='|parentheses" src README.md
```
