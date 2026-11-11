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

**M7 — packaging.** `make bundle` builds `dist/Rust Harness.app` with the
OpenCode sidecar inside, records and verifies its checksum, and signs it
(ad-hoc locally; Developer ID and notarization for a release). See
[Packaging](#packaging).

The window uses a custom title bar above the workspace: dragging and
double-click are handled by the bar, the system keeps the macOS traffic lights,
and the bar shows the project directory and engine status.

**M8 — rich composer.** Typing `@` opens a file picker backed by the project
search (`@` also lists agents when the server has them), `/` at the start of a
message lists slash commands, `$` lists skills, and `#` lists built-in
snippets. Selecting a file inserts an `@path` mention and attaches it to the
prompt with server-compatible offsets; commands replace the prompt with a
`POST /command` call. Attached files, agents, skills, and the selected command
appear as removable chips above the input. The model and agent badges below
the input open searchable pickers and persist the choice for the session. A
picker that fails or finds nothing leaves the typed text untouched.

Not implemented yet: collapsible tool output, Linux packaging, and auto-update.

**Notifications.** When a turn finishes or fails, a permission is requested, or
the agent asks a question while the window is not focused on that session, the
OS shows a notification. The body carries no message text by default; previews
are opt-in and truncated. The right panel has a global switch, per-kind
switches, and the preview switch. macOS delivery uses `osascript` with the
title and body passed as argv; a denial turns the feature off and shows an
inline notice. Settings live in
`~/Library/Application Support/Rust Harness/notifications.json`.

The post-M7 sequence — rich composer, conversation control, git review,
projects/worktrees/multi-run, automation, integrations, terminal, preview — is
in [`docs/ROADMAP.md`](docs/ROADMAP.md), with executable per-feature specs, a
dependency graph, and unit/e2e gates in [`docs/specs/`](docs/specs/README.md).

Sessions are scoped with the canonical project directory (on macOS, `/tmp` is
stored as `/private/tmp`); a new session is pinned to that directory through the
API. The transcript is reconciled with the server whenever a turn finishes or
the event stream reconnects, so dropped frames cannot leave stale text.

## Requirements

- Rust stable with the 2024 edition (tested on 1.99)
- macOS or Linux — GPUI does not officially support Windows
- OpenCode v2 on `PATH`, or pointed to explicitly
- Xcode command line tools for macOS packaging (`codesign`, `notarytool`)

## Run

```sh
cargo run -p rust-harness -- /path/to/project
```

The first build compiles GPUI and will take a while.

## Packaging

```sh
make bundle    # dist/Rust Harness.app
make install   # copies it into /Applications
```

The bundle contains the release app binary plus the OpenCode sidecar under
`Contents/Resources/opencode`, with an `opencode.sha256` manifest written next
to it. The engine verifies that hash before spawning the sidecar; a mismatch
stops startup with "The bundled OpenCode binary failed its integrity check".
Developer builds without a manifest skip the check.

Local bundles are ad-hoc signed. For a distributable build, set:

| Variable | Purpose |
| --- | --- |
| `SIGN_IDENTITY` | "Developer ID Application" identity; enables hardened runtime and a trusted timestamp |
| `NOTARY_PROFILE` | `notarytool` keychain profile; notarizes and staples the app |
| `APPLE_ID`, `APPLE_TEAM_ID`, `APPLE_APP_PASSWORD` | notarization credentials instead of a profile |
| `UNIVERSAL=1` | builds arm64 + x86_64; needs a universal sidecar or `OPENCODE_ARM64_BIN` + `OPENCODE_X86_64_BIN` |
| `OPENCODE_BIN` | sidecar to bundle (default: `opencode` on `PATH`) |
| `ARCHIVE=1` | also writes `dist/RustHarness-<version>.zip` and its `.sha256` |

Measured with OpenCode v2.2.6 (x86_64): the app is 265 MB, of which 235 MB is
the sidecar; the zip is 119 MB. The release workflow builds a bundle for the
runner architecture on GitHub Actions and signs it when the certificate secrets
are configured.

## Keyboard shortcuts

On macOS use Cmd; on Linux use Ctrl.

| Shortcut | Action |
| --- | --- |
| `Enter` | Send the composer text, or choose the highlighted picker item |
| `Shift+Enter` | Insert a newline in the composer |
| `@` / `/` / `$` / `#` | Open the file (or agent), command, skill, or snippet picker |
| `Up` / `Down` | Move the picker highlight |
| `Tab` | Choose the highlighted picker item |
| `Cmd/Ctrl+N` | New session |
| `Cmd/Ctrl+L` | Focus the composer |
| `Cmd/Ctrl+Shift+L` | Toggle light/dark theme |
| `Esc` | Close the picker, or dismiss the error or the open rename/delete prompt |

## Tests

```sh
cargo test --workspace
```

Protocol parsing and the pure state reducers have unit tests. The sidecar
integration tests in `crates/engine/tests/sidecar.rs` launch a real
`opencode serve`: the session test needs `opencode` on `PATH` (skip it with
`RUST_HARNESS_SKIP_E2E=1`), and the streaming test runs only when
`RUST_HARNESS_MOCK_PROVIDER` points at an OpenAI-compatible mock endpoint. The
attachment test uses the same mock and also checks that file/agent attachments
and a model/agent switch reach the server. The notification test runs when
`RUST_HARNESS_NOTIFY_SINK` names a file and asserts one sink line per turn.

## Configuration

| Variable | Purpose |
| --- | --- |
| `RUST_HARNESS_PROJECT` | Project directory (alternative to the positional argument) |
| `RUST_HARNESS_OPENCODE_BIN` | Explicit path to the `opencode` binary |
| `RUST_HARNESS_AUTOQUIT_MS` | Exit after N milliseconds; used for smoke tests |
| `RUST_HARNESS_NOTIFY_SINK` | Append notifications as JSON lines to this file instead of the OS |
| `RUST_HARNESS_NOTIFY_SETTINGS` | Override the notification settings file path |
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
