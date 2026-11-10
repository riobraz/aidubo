# ADR 0002 — A dependency-free terminal core crate

- **Status:** Accepted
- **Date:** 2026-11-10
- **Deciders:** Rust Harness maintainers
- **Related:** [Architecture](../ARCHITECTURE.md), [M14-02 spec](../specs/M14-02-vt-model.md)

## Context

M14 needs a terminal. The parser and grid are not a reducer, are large enough to
deserve their own tests, and must never touch GPUI, IO, or the network. The
existing crates are `protocol`, `core`, `engine`, `ui`, `views`, and `app`.
`harness-core` is for pure *state machines over engine data*; a VT parser plus
a cell grid is a different kind of component and would dominate that crate.

Two libraries were considered: `alacritty_terminal` (complete, but pulls a
large dependency tree and its own event model) and `vte` (the parser only,
leaving the grid to us). Both are viable. The reference implementations use a
full terminal emulator, but this product needs a shell dock, not a terminal
emulator with mouse reporting, images, or ligatures.

## Decision

Add a new workspace crate, `harness-term`:

- no dependencies at all in the first version;
- a byte-fed parser with an explicit state machine and a cell grid;
- a small width function (`char_width`) instead of a Unicode-width crate;
- no IO, no clock, no GPUI;
- the terminal view in `harness-views` consumes `visible()` and `take_dirty()`.

The supported sequence set is the one a shell uses for agent output: C0, CSI
cursor/erase/insert/delete/scroll, SGR, scroll regions, alternate screen,
bracketed paste, and window-title OSC. Unknown sequences are consumed and
discarded.

## Consequences

- The crate can be tested without a server, a runtime, or a terminal, and its
  tests run in milliseconds.
- Full Unicode width tables are not included. Wide CJK and combining marks are
  handled by ranges; a future crate swap is possible behind `char_width` and
  the `Cell` model.
- If the dock later needs mouse reporting or images, that is a new decision:
  either extend this crate or replace it with `alacritty_terminal` behind the
  same view interface.
- `cargo tree -p harness-term` must stay empty except for the crate itself;
  that is a testable invariant.
