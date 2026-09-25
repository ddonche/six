# Changelog

Every fix or feature bumps the version — including REPL changes. `six --version`,
the REPL banner, and `six --help` all report it.

## v0.1.4
- Host I/O (phase 8 of the host model, landed first because it is independent):
  add the `entropy()` and `time(mode)` host primitives.
  - `entropy()` returns a uniform integer in `0 ..= 2^53-1` from a secure OS
    source (`/dev/urandom` on Unix, `BCryptGenRandom` on Windows); no weak
    fallback — a runtime error if secure randomness is unavailable.
  - `time("utc")` / `time("steady")` return integer microseconds; steady is
    monotonic non-decreasing; an invalid mode is a runtime error.
  - New `src/host.rs` seeds the host boundary. `print`/`input` are untouched for
    now; their removal and the `open`/`in`/`out`/`close` primitives land in
    later phases.

## v0.1.3
- REPL: a comment-only or blank line no longer hangs on a continuation prompt;
  it is a complete (empty) entry and evaluates to nothing.
- Lexer: an unterminated `## ... ##` block comment is now an error (like an
  unterminated string) instead of silently swallowing to end of input. This
  also lets the REPL gather a multi-line block comment correctly.

## v0.1.2
- REPL: add `clear <name> ...` (remove one or more bindings) and `clear_all`
  (reset the whole session; builtins survive). This lets a name — variable or
  function — be defined afresh without weakening the redeclaration rule.

## v0.1.1
- REPL: redeclaring a name with `:` is now an error, exactly as in a `.six`
  file. (The REPL had been silently rebinding at the top level.)

## v0.1.0
- Initial Six v0.1: lexer, parser, and a tree-walking interpreter with
  guaranteed tail-call optimization. Six fundamental concepts; six builtins
  (`print`, `input`, `size`, `insert`, `remove`, `has`) plus two conversions
  (`number`, `text`); the `@` module system (a module is an ordinary Group); an
  interactive REPL; and `six spec` (the specification embedded in the binary).
  No standard library — libraries live outside the core as `.six` modules.
