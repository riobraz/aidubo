# 000 — Conventions every spec inherits

Status: done · 2026-11-10

These rules are not repeated in every spec. A spec that needs to deviate says
so explicitly and justifies it.

## 1. Crate boundaries

| Crate | May do | May not do |
| --- | --- | --- |
| `harness-protocol` | wire types, serde, fixtures | GPUI, IO, time, randomness |
| `harness-core` | pure reducers, state machines | GPUI, IO, `std::time` |
| `harness-engine` | tokio, HTTP, SSE, sidecar, child processes | touch GPUI entities |
| `harness-views` | render, input, dispatch commands | network, disk, `block_on` |
| `harness-app` | composition, window, actions | business logic |
| `harness-ui` | tokens, shared widgets | feature state |

A feature that needs a new crate proposes it in its spec and gets an ADR
before creating one. The default is a module inside an existing crate.

## 2. Protocol rules

- Unknown server variants deserialize into a catch-all (`Unknown` /
  `#[serde(other)]`), never into an error.
- Optional fields are `Option<T>` with `#[serde(default)]`; a missing field is
  normal, not a parse failure.
- Envelope types keep the server shape: `{data, cursor}`, `{location, data}`,
  or a bare array, exactly as the endpoint returns it.
- Every new type ships a fixture recorded from the live server and a test for
  (a) the happy path, (b) an unknown variant, (c) missing optionals.
- Field names are never renamed to taste; they mirror the wire.

## 3. Core rules

- Reducers are pure functions over `&mut Store`; no clocks, no IO. Time comes
  in as a parameter.
- Upsert by ID. Events repeat after reconnect; a repeat must be a no-op, not a
  duplicate row.
- Ordering is by server timestamp, with the server's order as the tie-break.
- Every store exposes `replace(snapshot)`, `upsert(item)`, `remove(id)`, and a
  `clear()`. Snapshot wins over accumulated events.
- Unknown input is ignored, counted, and testable; it never panics.

## 4. Engine rules

- One `EngineCommand` variant per operation, one `EngineEvent` per result.
- Commands are fire-and-forget; results arrive on the event channel. The UI
  never blocks on the network.
- Every command carries the ids it needs; no implicit "current session" in the
  engine.
- HTTP errors map to a short, user-safe message. No URL, token, prompt, or
  message content in the message.
- Request timeouts: 10 s for CRUD, none for the event stream and PTY sockets.
- Reconnect: exponential backoff 250 ms → 5 s; `StreamConnected` triggers a
  resnapshot of every store that has live state.

## 5. Event expansion

Adding an event is three edits:

1. a `StreamEvent` variant in `harness-protocol`;
2. a match arm in `harness-engine/src/events.rs` (unknown event names are
   already ignored by the catch-all);
3. a reducer call in the view's event pump.

The mapping is total: a new variant must be handled or explicitly ignored with
a comment explaining why.

## 6. Rendering and performance budgets

- Coalesce deltas: at most one `cx.notify()` per frame per store.
- Virtualize any list that can exceed 200 rows (transcript, diff, inbox,
  terminal scrollback, session list).
- Parse patches, markdown, and VT sequences off the render path; cache the
  laid-out result and invalidate by ID + revision.
- A 5,000-line diff and a 10 MB terminal burst must not drop frames; measure
  with the budgets in the spec.
- Never re-render finished transcript entries; only the tail is mutable.

## 7. Security

- Loopback only; never bind a non-loopback interface.
- Never auto-approve a permission request; never auto-answer a form.
- Treat model output, tool output, patches, and file contents as untrusted
  text: no automatic link opening, no command execution.
- Secrets (API keys, tickets, tokens) live in memory only, are never logged,
  never written to diagnostics, and are masked in the UI.
- New network, process, credential, or web surfaces require a threat note in
  the spec and a review before merge.

## 8. Accessibility and keyboard

Every dialog, picker, and panel: reachable by Tab, dismissible by Esc, no focus
trap, and every action has a key binding or is reachable from the command
palette. Focus returns to the composer after a modal closes.

## 9. Test naming and structure

- Unit: `#[test] fn <feature>_<behavior>()` inside the module under test.
- Protocol fixtures: `crates/protocol/tests/fixtures/<endpoint>.json` plus a
  test named `<endpoint>_parses_and_round_trips`.
- Engine e2e: one file per node under `crates/engine/tests/`, named
  `<node-id>.rs`, using `mod support;` from `crates/engine/tests/support/mod.rs`.
- Every e2e file starts with the same guard:

```rust
if std::env::var_os("RUST_HARNESS_SKIP_E2E").is_some() {
    eprintln!("skipping <node> e2e test");
    return;
}
```

and then checks the binary and its own gate variable, skipping with a clear
message.

- E2E tests create a unique temp project, assert a server-durable effect,
  shut the engine down, assert the port is closed, and remove the directory.

## 10. Definition of done

A node is done when all of these are true and recorded in its `STATUS.md`:

1. `cargo fmt --all -- --check` passes.
2. `cargo clippy --workspace --all-targets -- -D warnings` passes.
3. `cargo test --workspace` passes.
4. The node's e2e test passes with its gate, or is skipped with a recorded
   reason (never silently).
5. Protocol fixtures cover unknown variants and missing optionals.
6. `README.md` / `ARCHITECTURE.md` are updated if the node changed a command,
   a crate boundary, or a user-visible behavior.
7. The spec header and `graph.json` say `done`.

## 11. Estimation scale

`S` ≈ under a day, `M` ≈ 1–2 days, `L` ≈ 3–5 days, `XL` ≈ more, for one
developer who knows the codebase. A node that is `XL` must be split before it
starts; the graph is not allowed to contain an `XL` node.

## 12. Reference evidence

The optimization choices in these specs come from measured behavior, not
taste:

- OpenChamber coalesces deltas for 50 ms before fan-out and keeps a 2048-event
  replay log, after measuring 3,402 deltas averaging 1.7 characters per answer.
- OpenChamber indexes settled messages in SQLite FTS5 with a trigram tokenizer.
- Monocode stores transcripts as blocks in SQLite, prefetches neighbouring
  sessions, memoizes sidebar cards, and limits syntax highlighting to one
  background job.
- Both virtualize long lists and move parsing off the render path.

The full note is in [`../research/references.md`](../research/references.md).

## 13. Clean room

Implementation is clean room. This is binding for every node:

- Build from the verified contract in the spec, the published OpenAPI/event
  list, and black-box behavior. Never copy source, identifiers, comments,
  assets, fixture strings, or error text from OpenChamber, Monocode, OpenCode's
  UI, or any other reference implementation.
- Do not vendor a reference project, paste its files, or "translate" its code
  line by line. Reading a reference to understand *behavior* is fine; reading
  it to reproduce *code* is not.
- Fixtures must be produced from the live server or written from the schema,
  not lifted from a reference repository.
- The provenance line in `PLAN.md` names what was observed (docs, live API,
  black-box run) and confirms that no reference code was copied.
- A reviewer who suspects copied code can block the node; the fix is a
  reimplementation from the contract, not a rename.
