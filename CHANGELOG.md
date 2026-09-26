# Changelog

Every fix or feature bumps the version — including REPL changes. `six --version`,
the REPL banner, and `six --help` all report it.

## v0.1.10
- Cleanup and migration: the frozen Six surface is now final — six core builtins
  (size insert remove has number text) and six host builtins (open in out close
  entropy time). `print` and `input` are removed from the language; all I/O
  flows through the runtime channels, `out(output …)` and `in(input)`.
  - builtins.rs/interp.rs: print/input and their dispatch removed; BUILTINS and
    the module docs describe the 6 + 6 surface. Runtime input is injectable so
    captured/test interpreters read an empty (EOF) source, never real stdin.
  - Examples migrated: output-only programs use a local `line` helper; the
    interactive guessing_game and tiny_inventory use a buffered line reader over
    in(input). New reference/io.six documents the userland console-I/O pattern.
  - Tests migrated (integration observes values via a `shown` helper or the
    captured output channel; modules use a userland `say`); docs/repl_tour.txt
    and the REPL help no longer mention print/input.
  - Embedded spec (`six spec`): §37/§38 restated for the 6 + 6 surface, new
    §38.1 Host I/O documents entropy/time, the four relationship primitives, the
    runtime channels, and the file/network/process/device domains; illustrative
    examples use the host-channel forms.
  - New tests/acceptance.rs: a spec-level pass over the whole frozen surface —
    the twelve builtins, the absence of print/input, and an end-to-end program
    for every host capability.

## v0.1.9
- Host I/O device domain (Addendum D): the extensible adapter boundary.
  - in(["device"]) discovers host-exposed devices; with no adapters registered
    in v1 it always returns [] (discovery establishes no relationship and is
    in-only — open/out/close on ["device"] are errors).
  - open(["device" kind identifier? options?]) is the only way to establish a
    device relationship. v1 registers no concrete adapters, so a well-formed
    descriptor fails cleanly with "unknown device adapter"; a non-text kind and
    the bare discovery descriptor are distinct errors.
  - A ["device" kind ...] descriptor is open-only: direct in/out report that the
    relationship must be opened first. The dispatch shape is what future
    adapters plug into — no new Six syntax, primitives, or value types.

## v0.1.8
- Host I/O process domain (Addendum C). Current-process state through direct
  descriptors, child processes as persistent runtime-backed Groups:
  - ["process" "arguments"] -> in: the program's arguments as a Group of Text
    ([] when none; observational, never writable).
  - ["process" "environment"] -> in: the whole environment as [name value] Text
    pairs (order unspecified; a non-text name/value is an error).
  - ["process" "environment" name] -> in returns the value or .. when unset; out
    sets it (Text) or removes it (..).
  - ["process" "directory"] -> in/out: read or change the working directory.
  - ["process"] -> out code: exit the current Six process (integer status).
  - open(["process" program args repr options?]) spawns a child and returns a
    Group [["input" _] ["output" _] ["error" _]] of runtime-backed channels.
    Options are ["directory" path] and ["environment" overrides] (two-element
    keyed Groups; unknown/duplicate/malformed options are errors). Child env and
    directory are snapshotted at open; a .. override removes a variable.
  - Channels: out(child["input"] …)/close; in(child["output"])/in(child["error"])
    /close. Text = UTF-8 (split characters buffered); binary = Group of bytes
    0..255; .. at EOF ("" and [] are not EOF).
  - in(child) blocks for termination: a non-negative Number exit status, or ..
    for signal/abnormal termination; the result is stable and never closes a
    channel. out(child ..) requests termination; close(child) releases only the
    lifecycle relationship, leaving separately held channels live. Aliasing keeps
    associations; :: deep-copy strips them.
- Runtime input is now injectable, so captured/test interpreters read an empty
  (EOF) source instead of blocking on real stdin.

## v0.1.7
- Host I/O network domain (Addendum B): persistent TCP and UDP via open/in/out/close.
  - TCP connection ["network" "tcp" addr port repr]: byte-stream in/out (text or
    binary, fixed at open); .. at EOF; text buffers split UTF-8 across reads.
  - TCP listener ["network" "tcp" "listener" addr port repr]: open(listener)
    accepts the next connection (inherits representation); closing the listener
    does not close accepted connections.
  - UDP ["network" "udp" addr port repr]: in returns [source_addr source_port
    data] (one datagram); out sends [dest_addr dest_port data]; no truncation.
  - Network descriptors are not direct in/out targets — they must be opened.
  - No connect/listen/accept/send/receive primitives; addresses are Text, ports
    integer Numbers. Sockets close on drop (close / interpreter shutdown).

## v0.1.6
- Host I/O file domain (Addendum A). Direct, one-shot in/out (no open/close):
  - ["file" path "text"] / ["file" path "binary"] read and write complete
    contents; missing path -> .., invalid UTF-8 / bad byte -> error.
  - ["file" path] with .. removes an entry (non-empty directory -> error;
    already absent -> success).
  - ["file" path "directory"]: in lists entry names (no "."/".."); out [] creates
    (parent must exist, target must not).
  - ["file" path "metadata"]: keyed Group {kind,size,modified,created,readonly};
    unavailable fields / directory size -> ...
  - ["file" source "path"] renames/moves via the host's native rename; target
    must not already exist; cross-device is an error (never copy+delete).
  - Note: per frozen Addendum A the file domain has no open/close; the middle
    doc's persistent-file-via-open form is superseded (flagged for review).

## v0.1.5
- Host I/O foundation (phases 1-3, 7 of the host model):
  - Hidden host association bound to Group identity (Box<Assoc> on GroupData).
    Aliasing shares it; `::` deep-copy strips it; visible mutation never touches
    it — the Runtime Association Law.
  - The four host primitives open/in/out/close with central capability dispatch.
  - The runtime channels input/output/error, pre-bound as runtime-backed Groups
    before any user code (re-established after clear_all). input decodes UTF-8
    (buffering split characters, `..` at EOF); output/error encode UTF-8 with no
    automatic newline; the channels obey the association law and capability
    rules (in(output), out(input …), open(input), etc. all error).
  - print/input are still present during migration; their removal and the
    file/network/process/device domains land in later phases.

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
