# Rust Harness

A native, GPU-accelerated desktop client for the [OpenCode](https://opencode.ai)
agent, built in Rust with [GPUI](https://gpui.rs).

The agent engine stays OpenCode: the app supervises an `opencode serve` sidecar
and talks to its loopback HTTP/SSE API. See
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) and
[`docs/adr/0001-use-opencode-as-agent-engine.md`](docs/adr/0001-use-opencode-as-agent-engine.md).

## Status

**M6 — polish.** The window shows a project-scoped session sidebar
(create, select, rename, delete) and the open session's transcript, loaded from
the server. Typing a message and pressing **Enter** queues a prompt; assistant
text streams into the transcript over OpenCode's event stream and renders as
markdown, and **Stop** interrupts the running turn. The composer is multiline:
Shift+Enter inserts a newline. Keyboard shortcuts cover session creation,
composer focus, theme switching, and dismissing overlays. The color scheme
follows the system appearance at startup and can be toggled at any time. If the
event stream drops, a banner says live updates are paused and it clears when the
stream reconnects, at which point the transcript is resnapshotted.

Reasoning streams live under a **Thinking** label. Tool calls render as cards
showing the tool name, argument summary, status (streaming → running → done or
failed), and the result text or error; every update lands in the same card.
Provider and tool errors render as bordered error blocks.

When a tool asks for permission, a prompt appears above the composer with the
action, its resources, and **Allow once**, **Always allow**, and **Deny**.
Replies go to the server and unblock the waiting turn; a decline fails the tool
with the server's message and ends the turn. Nothing is approved on the user's
behalf, and the window stays responsive while a request is pending.

Not implemented yet: collapsible tool output and packaging.

Sessions are scoped with the canonical project directory (on macOS, `/tmp` is
stored as `/private/tmp`); a new session is pinned to that directory through the
API. The transcript is reconciled with the server whenever a turn finishes or
the event stream reconnects, so dropped frames cannot leave stale text.

## Requirements

- Rust stable with the 2024 edition (tested on 1.99)
- macOS or Linux — GPUI does not officially support Windows
- OpenCode v2 on `PATH`, or pointed to explicitly

## Run

```sh
cargo run -p rust-harness -- /path/to/project
```

The first build compiles GPUI and will take a while.

## Keyboard shortcuts

On macOS use Cmd; on Linux use Ctrl.

| Shortcut | Action |
| --- | --- |
| `Enter` | Send the composer text |
| `Shift+Enter` | Insert a newline in the composer |
| `Cmd/Ctrl+N` | New session |
| `Cmd/Ctrl+L` | Focus the composer |
| `Cmd/Ctrl+Shift+L` | Toggle light/dark theme |
| `Esc` | Dismiss the error or the open rename/delete prompt |

## Tests

```sh
cargo test --workspace
```

Protocol parsing and the pure state reducers have unit tests. The sidecar
integration tests in `crates/engine/tests/sidecar.rs` launch a real
`opencode serve`: the session test needs `opencode` on `PATH` (skip it with
`RUST_HARNESS_SKIP_E2E=1`), and the streaming test runs only when
`RUST_HARNESS_MOCK_PROVIDER` points at an OpenAI-compatible mock endpoint.

## Configuration

| Variable | Purpose |
| --- | --- |
| `RUST_HARNESS_PROJECT` | Project directory (alternative to the positional argument) |
| `RUST_HARNESS_OPENCODE_BIN` | Explicit path to the `opencode` binary |
| `RUST_HARNESS_AUTOQUIT_MS` | Exit after N milliseconds; used for smoke tests |
| `RUST_LOG` | Tracing filter (default: `info,harness_engine=debug`) |

The sidecar is launched on `127.0.0.1` with a random port and a random
per-launch password; the password is passed through
`OPENCODE_SERVER_PASSWORD` and never written to disk.

## Layout

```
crates/
  app/        GPUI bootstrap and composition root
  ui/         theme tokens and shared visual primitives
  views/      application views
  engine/     sidecar lifecycle, HTTP client, SSE stream, supervision
  protocol/   OpenCode wire types
  core/       GPUI-free state
```
