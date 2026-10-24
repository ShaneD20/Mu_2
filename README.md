# muc: the Mu compiler

A compiler for Mu, a statically typed systems language with no heap, no
undefined behavior and a compile-time-computable stack depth, targeting
x86-64 ELF. Written in Rust.

- Language spec: `docs/specification.md`
- Grammar: `docs/grammar.ebnf`
- Plan and open decisions: `docs/ROADMAP.md`

## Status

Phases 0 and 1 are done: source management, diagnostics, the lexer with the
newline layout rules (§2.6), and the parser with the type-name pre-pass and
cast disambiguation (§6.7). Declaration collection and name resolution are
next.

## Usage

```sh
cargo build
cargo test
cargo run -- lex tests/examples/control_flow.mu
cargo run -- parse tests/examples/control_flow.mu
```

`muc lex` prints each token with its line, column and category. `muc parse`
prints the syntax tree as S-expressions. Both then print any diagnostics and
exit with status 1 if there were errors.

## Layout

```
src/
  main.rs          command-line driver
  source.rs        files, spans, line/column lookup
  diag.rs          diagnostics, rendered rustc-style
  ast.rs           syntax tree
  sexp.rs          S-expression printer for the AST
  lexer/
    mod.rs         raw lexer: sigils, operators, literals, comments
    layout.rs      newline rules N1–N8 and the same-line rule
    token.rs       token kinds and keywords
    tests.rs       lexer unit tests
  parser/
    mod.rs         type-name pre-pass, recursive descent, Pratt expressions
    tests.rs       parser unit tests
tests/
  examples/        Mu programs from the spec; every phase must accept them
  examples.rs      runs the examples through the compiler
docs/
```
