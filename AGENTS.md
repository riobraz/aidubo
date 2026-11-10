# Rust Harness — Agent Guide

A native, GPU-accelerated desktop client for the OpenCode agent, written in
Rust with GPUI. The app supervises an `opencode serve` sidecar over loopback
HTTP/SSE; it does not reimplement the agent engine.

## Read first

1. **`docs/ARCHITECTURE.md` is required reading** before any design or code
   change. It defines the crate boundaries, engine integration, event model,
   UI data flow, concurrency model, security rules, and MVP scope. If a change
   contradicts it, stop and discuss rather than working around it.
2. **`docs/adr/`** records accepted decisions. Do not silently reverse one; add
   a new ADR that supersedes it.
3. **`README.md`** is the current status and run/test instructions.
   **`CONTRIBUTING.md`** has build, lint, and commit conventions.
4. For external behavior, treat the OpenCode HTTP/SSE API as a contract and
   verify against the live `/openapi.json` or existing tests. Do not invent
   endpoints, event names, or payload shapes.

## Plan before implementing

Any non-trivial task (new feature, milestone slice, refactor, protocol change)
starts with a checked-in plan.

### Location and naming

```
docs/plans/<plan-name>/PLAN.md
docs/plans/<plan-name>/STATUS.md
```

- `<plan-name>` is lowercase kebab-case, short and descriptive, e.g.
  `m4-live-tool-events`, `m5-permissions`, `fix-sse-reconnect`.
- One directory per plan. Never overwrite an unrelated plan.
- Plans are part of the repository, not scratch files.

### `PLAN.md` must contain

- **Goal** — the user-visible outcome.
- **Context** — links to the relevant `ARCHITECTURE.md` sections and ADRs.
- **Scope / non-goals** — what this plan does *not* do.
- **Design** — affected crates, data flow, protocol additions, edge cases.
- **Steps** — an ordered checklist.
- **Test plan** — unit tests, integration tests, and manual checks.
- **Risks** — API drift, GPUI limits, security, performance.

### `STATUS.md` must contain

- **Status** — `draft`, `in progress`, `blocked`, or `done`.
- **Last updated** — absolute date (`YYYY-MM-DD`).
- **Completed** — checked-off steps with the commit or date.
- **In progress / next** — what is being worked on now.
- **Blockers** — what is blocking and who/what can unblock it.
- **Verification** — commands run and their real results (never claim a test
  passed without running it).

Update `STATUS.md` at every meaningful step: start, after each completed
checklist item, when blocked, and when done. If work is abandoned, mark the
plan `done` with the reason instead of deleting it.

A tiny fix (one file, no behavior change) may skip the plan, but if a plan
already exists for the area, update its `STATUS.md`.

## Architecture rules

The crate boundaries are load-bearing:

- `harness-protocol` — wire types only. No GPUI, no IO. Unknown server variants
  must deserialize to a catch-all instead of failing.
- `harness-core` — pure state machines and reducers. No GPUI, no IO. Most
  correctness lives here, so it must be heavily unit-tested.
- `harness-engine` — owns the tokio runtime, HTTP client, SSE stream, and
  sidecar lifecycle. Never touches GPUI entities; communicate through the
  event/command channels.
- `harness-views` — rendering and input handling. No network or disk access;
  read `harness-core` state and dispatch engine commands.
- `harness-app` — the composition root; only this crate wires the others.
- `harness-ui` — theme tokens and shared visual primitives.

Required invariants:

- Upsert entries by ID; never append blindly. Events can repeat after
  reconnect.
- Coalesce streaming text; call `cx.notify()` at most once per frame, not per
  token.
- Resnapshot the transcript after a turn ends and when the SSE stream
  reconnects; do not assume gap-free delivery.
- Ignore unknown events and message variants so a server upgrade cannot crash
  the UI.
- Keep version-specific OpenCode routing in one engine adapter.
- No shared mutable state between GPUI and the tokio runtime.

## Testing rules

- Run `cargo test --workspace` before declaring work done.
- Put pure-logic tests next to the code (`#[cfg(test)]` in `harness-core`,
  `harness-protocol`, and the SSE parser in `harness-engine`).
- Protocol tests use serde fixtures and must cover unknown variants and missing
  fields; never let a server addition break parsing.
- Sidecar integration tests live in `crates/engine/tests/sidecar.rs` and launch
  a real `opencode serve`. They must not fail on machines without the binary:
  guard them with the existing environment variables and skip with a clear
  message.
- Existing e2e gates:
  - `RUST_HARNESS_SKIP_E2E=1` — skip the session lifecycle test.
  - `RUST_HARNESS_MOCK_PROVIDER` — OpenAI-compatible mock for text streaming.
  - `RUST_HARNESS_MOCK_TOOL_PROVIDER` — mock that streams reasoning and calls
    a tool.
  - `RUST_HARNESS_MOCK_PERMISSION_PROVIDER` — mock that calls a shell tool
    behind an `ask` permission.
- E2E tests must verify durable effects on the server, not just local events,
  and must assert clean shutdown (no process left listening on the port).
- Never weaken, delete, or skip a failing test to make a change pass. Fix the
  cause, or report the failure and the exact output.
- Tests must be deterministic and isolated: use unique temp project
  directories, clean them up, and never depend on a developer's real OpenCode
  config, credentials, or session data.
- Before finishing, run:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Common commands

```sh
cargo build --workspace
cargo run -p rust-harness -- /path/to/project
cargo test --workspace
make bundle   # dist/Rust Harness.app with the bundled, checksummed sidecar
```

Useful environment variables: `RUST_HARNESS_PROJECT`,
`RUST_HARNESS_OPENCODE_BIN`, `RUST_HARNESS_AUTOQUIT_MS`, `RUST_LOG`.

## Coding conventions

- Rust 2024 edition; workspace MSRV is 1.90.
- User-facing error messages are short, safe to display, and never include
  secrets.
- Never log prompts, message content, tokens, or credentials. Redact URLs
  that could carry secrets.
- Prefer small, testable functions in `harness-core` over logic hidden in
  render code.
- Keep changes focused; unrelated refactors belong in their own commit/PR.
- Update `README.md` when milestone status, configuration, or commands change.

## Rust guidelines

Adapted from Zed's [`.rules`](https://github.com/zed-industries/zed/blob/main/.rules)
(retrieved 2026-11-08). Zed is upstream of the GPUI APIs this project uses.
Keep these rules high-signal; add one only when a pattern is repeatedly hit
here, and prefer clarifying an existing rule over adding a vague new one.

- Prioritize correctness and clarity. Treat speed and efficiency as secondary
  until the behavior is right and measured.
- Do not write organizational comments or code summaries. Comment only to
  explain a non-obvious "why".
- Prefer implementing functionality in an existing file unless it is a new
  logical component; avoid many tiny files.
- Avoid panic-prone code in production paths: no `unwrap()`/`expect()` outside
  tests, and be careful with indexing. Propagate with `?` or handle explicitly.
- Never silently discard a fallible result with `let _ =`. Propagate with `?`,
  handle it with `match`/`if let`, or log it with `tracing` when ignoring is
  deliberate. Cleanup paths may ignore errors, but the intent must be obvious.
- Ensure async failures reach the UI layer with a meaningful, user-safe
  message.
- Never create `mod.rs`; use `src/some_module.rs` instead.
- Avoid creative additions beyond what was requested.
- Use full words for variable names (`queue`, not `q`).
- Use variable shadowing to scope clones in async blocks so borrowed lifetimes
  end before the first `.await`.
- Entities and rendering live on the GPUI foreground thread: use `cx.spawn`
  for UI-bound work and `cx.background_spawn`/the engine runtime for IO. Never
  update an entity that is already being updated (it panics). Register handlers
  with `cx.listener` and call `cx.notify()` once per meaningful state change.
- In GPUI tests, prefer `cx.background_executor().timer(...)` over
  `smol::Timer::after(...)` so GPUI's scheduler can track the timer.
- Lint via `make clippy` (the same `cargo clippy --workspace --all-targets -- -D
  warnings` invocation).

## Clean room

- Implement every feature from documented contracts, published specs, and
  black-box observation. Never copy source, identifiers, comments, assets, or
  string tables from OpenChamber, Monocode, OpenCode's UI, or any other
  reference implementation.
- The OpenAPI document, `/doc`, the event list, and the reference projects'
  published behavior are contracts, not code to port.
- Do not vendor reference code. Reimplement behavior in this codebase's own
  terms, with its own tests, and keep the provenance in the plan.

## Security

- Bind only to loopback; never expose the sidecar on a non-loopback interface.
- Never auto-approve permission requests.
- Treat tool output and file contents as untrusted text. Never execute links or
  commands that originate from model or file output.
- Do not commit credentials, session data, or anything under `target/`.
- Report suspected vulnerabilities per `SECURITY.md`, not in a public issue.

## Scope

The MVP scope and explicit non-goals are in `docs/ARCHITECTURE.md`. Do not add
non-goals (diff review, editor, terminal, multi-window, collaboration, mobile,
VS Code, remote access) without an issue and an ADR. Update the milestone table
and the README status when a milestone or partial milestone is completed.
