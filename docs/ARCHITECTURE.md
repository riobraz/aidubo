# Rust Harness — Architecture

Status: draft · 2026-11-08

## 1. What we are building

A native, GPU-accelerated desktop client for the OpenCode agent — a Rust/GPUI
reimplementation of OpenChamber's chat and session experience.

The agent engine stays OpenCode. The harness is a native frontend that launches
and supervises `opencode serve`, consumes its HTTP/SSE API, and renders sessions
in GPUI. No TypeScript runs in our process; no OpenChamber code is embedded.

**MVP scope**

- Session list: create, rename, select, archive/delete.
- Chat: load history, send prompts, stream assistant output.
- Tool-call, reasoning, and error display.
- Abort a running turn.
- Permission prompts and replies.
- Reuse OpenCode config, providers, auth, and session storage as-is.

**Explicit non-goals for the MVP**

- Diff/change review, editor, terminal, file tree.
- Multi-run, goals, scheduling, preview, remote access, mobile, VS Code.
- Multi-window and collaboration.
- Reimplementing the agent loop.

These are the differentiators to earn after the core loop is solid.

## 2. Verified baseline (2026-11-08)

| Component | Fact |
| --- | --- |
| OpenChamber | `openchamber/openchamber`, MIT, TypeScript/Bun monorepo (`web`, `ui`, `electron`, `mobile`, `vscode`, `sdk`). Desktop is Electron. A client of OpenCode, not an engine. |
| OpenCode | `anomalyco/opencode`, MIT, ~225k stars, v2. The installed v2.1.4 serves the JSON API under `/api` with basic auth, an OpenAPI 3.1 spec at `/openapi.json`, and SSE at `/api/event`. Newer docs describe `/global/health`, `/doc`, and unscoped paths — the version is part of the contract. |
| OpenChamber link | Consumes `@opencode/client` and `@opencode/schema` 2.2.6, generated from OpenCode's authoritative Effect `HttpApi`. |
| GPUI | Official `gpui` crate 0.2.2 (Apache-2.0, Zed, macOS + Linux per docs). |
| Components | `gpui-component` 0.9.0 / `gpui-kit` 0.9.0 (Apache-2.0, Longbridge): 75+ widgets, markdown, virtual lists, editor, dock, accessibility, UI tests. They pin `gpui-pre` =0.3.11 — a third-party republished Zed snapshot. |

The important consequence: **the UI/engine boundary is already a network API**,
so this project is a native client, not a rewrite of OpenCode.

## 3. System context

```
┌──────────────────────────────────────────────┐
│ rust-harness (one process)                   │
│                                              │
│   GPUI views ── Entity state ── Engine actor │
│        ▲              ▲              │       │
│        └──── EngineEvent channel ────┘       │
└───────────────────────────┬──────────────────┘
                            │ HTTP (OpenAPI) + SSE
┌───────────────────────────▼──────────────────┐
│ opencode serve (child process, loopback)     │
│ sessions · config · providers · tools · LSP  │
│ MCP · permissions · file edits · git         │
└───────────────────────────┬──────────────────┘
                            │
              project files · ~/.config/opencode
              server-owned session storage
```

Trust boundary: the sidecar is the only component that touches the project,
credentials, and session data. The UI never reads OpenCode's storage directly.

## 4. Crate layout

```
crates/
  app/        binary: GPUI bootstrap, window, composition root, actions
  ui/         theme, shared widgets, rendering helpers (facade over the UI kit)
  views/      session list, chat transcript, composer, dialogs
  engine/     sidecar lifecycle, HTTP client, SSE stream, reconnect, batching
  protocol/   typed DTOs + event enum for the MVP subset (no GPUI, no IO)
  core/       pure state machines/reducers: sessions, transcript, permissions
```

Rules:

- `protocol` and `core` have no GPUI dependency. Most logic is testable headlessly.
- `engine` owns the tokio runtime and never touches entities.
- `views` never performs IO; it reads `core` state and dispatches commands.
- `app` wires the three together.

## 5. Engine integration

### 5.1 Sidecar lifecycle

1. **Locate the binary**: bundled resource → `$OPENCODE_BIN` → `PATH`. Refuse to
   start with a clear diagnostic if none is found.
2. **Pick a port**: bind `127.0.0.1:0`, read the assigned port, close, then pass it
   to `opencode serve --hostname 127.0.0.1 --port <port>`. Retry on collision.
3. **Authenticate**: generate a random per-launch password and set
   `OPENCODE_SERVER_PASSWORD` (username defaults to `opencode`). Never bind to a
   non-loopback interface. Verified on v2.1.4: every `/api/*` route returns
   `401 {"_tag":"UnauthorizedError"}` without credentials, while `/` serves the
   web UI unauthenticated.
4. **Set the working directory** to the selected project root so config discovery
   and the agent's tool sandbox match the user's project.
5. **Wait for readiness**: poll `GET /api/session` with basic auth until it
   returns 200 (v2.1.x). Newer builds expose `GET /global/health`; try it first
   when present. Surface child stderr on failure.
6. **Shut down**: on app quit, request graceful termination and kill after a grace
   period. Track the child in one place; never leave orphans.
7. **Version check**: compare the health `version` against the range we were built
   and tested against; warn (do not block) on mismatch.

OpenCode owns `opencode.json`, provider auth, and session storage. The app passes
the project directory and otherwise stays out of the way — that is what makes
"reuse config and sessions" work.

> Version note (verified 2026-11-08 on v2.2.6): the transcript is
> `GET /api/session/:id/message?order=asc` returning `{data: [...], cursor}`.
> Messages are tagged by `type` (`user`, `assistant`, `system`, `synthetic`,
> `idle`, plus control messages); assistant output is a `content` array of
> `text` / `reasoning` / `tool` parts. `POST /api/session/:id/prompt` takes
> `{"text": …}`, returns the admitted inbox message (same ID as the user
> message), and runs the turn asynchronously. `POST /api/session/:id/interrupt`
> returns `{"interrupted": bool}` (false when idle). SSE frames are
> `data: {json}\n\n` with `{id, type, data, created?, location?, durable?}`;
> streaming text arrives as `session.text.started/delta/ended` carrying
> `assistantMessageID` and the part `ordinal`, and turns end with
> `session.execution.succeeded/failed/interrupted`. Unknown events are ignored.
>
> History (v2.1.4): OpenCode v2.1.4 stores session locations as
> canonical absolute paths and `GET /api/session` accepts `directory`, `parentID`,
> `limit`, `order`, `search`, and `cursor`. The engine canonicalizes the project
> directory (on macOS `/tmp` becomes `/private/tmp`); a non-canonical path
> silently returns an empty list. `POST /api/session` accepts
> `Location.PublicRef { directory }`, which pins a new session to the project
> even if the process cwd ever differs. The published v2.2 docs also describe
> `/global/*` routes; the engine targets v2.1.x first and keeps version-specific
> routing in one adapter.

### 5.2 Transport

- **HTTP**: `reqwest` with rustls, plain HTTP to loopback. One client, shared.
- **SSE**: `GET /api/event` for the workspace stream (first event is
  `server.connected`), parsed with `eventsource-stream`. Newer servers also
  expose `GET /global/event`.
- **Runtime**: a dedicated multi-threaded tokio runtime owned by `engine`, running
  on its own OS thread. GPUI's executor is for UI-bound tasks; do not run the
  network stack on it.
- **Bridge**: `engine` sends `EngineEvent` values over an unbounded channel; a
  GPUI foreground task drains them and updates entities. Commands travel the
  other way over a second channel (`EngineCommand`): the supervisor performs the
  HTTP call on its own runtime and emits a result event. The UI never blocks on
  the network.

### 5.3 Event model

OpenCode v2 emits granular deltas, which is ideal for native rendering. Confirmed
event families include:

- Session: `session.created`, `session.metadata.updated`, `session.renamed`,
  `session.archived`, `session.deleted`, `session.idle`, `session.status`,
  `session.usage.updated`.
- Text stream: `session.text.started`, `session.text.delta`, `session.text.ended`.
- Reasoning: `session.reasoning.started`, `session.reasoning.delta`,
  `session.reasoning.ended`.
- Steps/tools: `session.step.started`, `session.step.ended`,
  `session.tool.called`, `session.tool.progress`, `session.tool.success`,
  `session.tool.failed`.
- Permissions: `permission.asked`, `permission.replied`.
- Misc: `file.edited`, `vcs.branch.updated`, `server.connected`.

Handling rules:

- **Upsert by ID**, never append blindly. Events can repeat after reconnect.
- **Coalesce text deltas**: accumulate into the active part and call `cx.notify()`
  at most once per frame (~16 ms), not per token.
- **Resnapshot on reconnect**: refetch the session list and the open transcript,
  then reapply live events. Do not assume gap-free delivery.
- **Reconnect** with exponential backoff (250 ms → 5 s) and a health probe.
  Announce a drop once per outage (`StreamDisconnected`) and announce each
  reopen (`StreamConnected`) so the view can show a banner and resnapshot.
- Treat unknown event variants as ignorable so a server upgrade cannot crash the UI.

### 5.4 Protocol types

Start hand-written for the MVP subset and validate them against the live `/doc`
spec in a contract test. The v2 API is generated from an Effect `HttpApi`; an
OpenAPI 3.1 → Rust generator (`progenitor`, `openapi-generator`) may not cover it
cleanly yet. Adopt codegen only after the hand-written client proves the subset,
and only if the generated output preserves discriminants and optionality.

MVP endpoints:

| Area | Endpoints (verified on v2.1.4) |
| --- | --- |
| Readiness | `GET /api/session`; newer builds: `GET /global/health` |
| Sessions | `GET /api/session`, `POST /api/session`, `GET/PATCH/DELETE /api/session/:id`, `POST /api/session/:id/interrupt` |
| Messages | `GET /api/session/:id/message`, `POST /api/session/:id/prompt` |
| Permissions | `GET /api/session/:id/permission`, `POST /api/session/:id/permission/:requestID/reply` |
| Events | `GET /api/event` (SSE) |
| Spec | `GET /openapi.json` (basic auth) |

`POST /api/session/:id/prompt` returns immediately; the SSE stream drives
rendering, so the UI stays responsive.

For the M1 session list, `GET /api/session` is scoped with
`directory=<canonical project>&parentID=null&order=desc`. `POST /api/session`
sends `{"location":{"directory": …}}`; `PATCH` sends `{"title": …}` and returns
204 with no body, so the UI applies the title locally.

For M2/M3 the supervisor holds one long-lived `GET /api/event` connection
(created without a request timeout, so it is not torn down mid-stream) and
reconnects with exponential backoff. Commands `LoadTranscript`, `SendPrompt`,
and `Interrupt` travel over the same command channel as session CRUD. Text
deltas update the transcript in place; when a turn ends or the stream
reconnects, the view reloads the timeline so the final text, finish reason, and
provider errors come from the server rather than from accumulated frames.

For M4 the stream also consumes `session.reasoning.started/delta/ended` and the
tool lifecycle (`session.tool.input.started/delta/ended`, `session.tool.called`,
`progress`, `success`, `failed`). Verified on v2.2.6: tool events carry the
assistant message ID, the tool-call ID, and result content arrays of
`text`/`file` items; the transcript keeps one card per tool-call ID and updates
it in place. Snapshot tool parts carry the same state
(`streaming|running|completed|error`) with the argument JSON as either a partial
string or an object, which the wire types normalize to text. Tool failures
render their error and any partial content; a user-initiated abort is not shown
as a turn failure.

For M5 permissions, verified on v2.2.6: `GET /api/session/:id/permission`
returns `{"data": PermissionRequest[]}` with only pending requests, oldest
first, and each request carries `id`, `sessionID`, `action`, `resources`,
optional `save` (the patterns "always" would cover), `message`, and `source`.
`POST …/permission/:requestID/reply` takes `{"decision":"once"|"always"|"reject",
"message":null}` — the field is `decision`, not `reply` — and returns 204. The
stream mirrors this with `permission.asked` (data is the request) and
`permission.replied` (`{sessionID, requestID, reply}`). Replying `once` lets the
tool run and the turn continue; `reject` fails the tool with an `aborted` error
whose message is "The user declined this tool call" and interrupts the turn, so
that tool-level message is shown even though turn-level aborts are hidden.
Approval is session-scoped on the server, so later identical calls in the same
session do not ask again; the app itself never auto-approves.

For M6, the stream pump emits `StreamDisconnected` once per outage (not on
every reconnect attempt, which would clear the banner between 250 ms retries)
and `StreamConnected` each time a stream opens; the workspace shows a
"Reconnecting" banner while the engine is ready but the stream is down. The
view reloads the selected session's transcript and pending permissions on
reopen, honoring the resnapshot rule.

## 6. UI architecture

### 6.1 GPUI data flow

- `App` owns all state as entities; views hold `Entity<T>` handles.
- A view implements `Render` and reads entities through `cx`; it never mutates
  another entity during render.
- Mutations happen in handlers/closures, followed by `cx.notify()`.
- Cross-entity reactions use `observe` with stored subscriptions (detach or drop
  deliberately).

### 6.2 Entity graph

```
Workspace (root view)
├── EngineStatus     connecting | ready | crashed | restarting
├── SessionStore     id → SessionSummary, active session, per-session status
├── ChatStore        id → Transcript { entries: Vec<Entry> }, stream buffers
├── ComposerState    draft text, selected agent/model, sending state
└── PermissionQueue  pending permission requests (FIFO)
```

`ChatStore` entries are an enum: user message, assistant text, reasoning, tool
call, error, notice. Every entry carries its OpenCode part/message ID for upserts.

### 6.3 Views

- **AppShell** — sidebar + transcript + composer; three regions, resizable later.
- **SessionList** — grouped, searchable, working/idle/failed indicators.
- **Transcript** — virtualized list, bottom-anchored; auto-follow only when the
  user is at the bottom ("stick to bottom" toggle).
- **MessagePart** — dispatch on part type; text uses markdown; tool calls show
  name, input summary, status, and collapsible output.
- **Composer** — multiline input, IME-safe, Enter sends / Shift+Enter newline,
  paste, undo, stop button while streaming.
- **PermissionSheet** — prompt above the composer with allow once/always/deny;
  it blocks the turn, not the UI.
- **StatusBar** — engine state, project, model, token/cost summary when available.
- **Theme** — `harness-ui` tokens are derived from the active gpui-component
  theme (`background`, `sidebar`, `foreground`, `border`, `primary`, status
  colors), so light and dark stay consistent. Startup follows the OS appearance
  (`Theme::sync_system_appearance`); a sidebar button and Cmd/Ctrl+Shift+L
  toggle the mode for the session.
- **Shortcuts** — actions live in `harness-views` and are bound by the app in the
  `Workspace` key context: Cmd/Ctrl+N (new session), Cmd/Ctrl+L (focus
  composer), Cmd/Ctrl+Shift+L (toggle theme), Esc (dismiss overlays). Both the
  macOS and the Linux modifiers are bound explicitly because static keystroke
  parsing does not resolve `secondary-`.

### 6.4 UI kit decision

Two viable bases, and they are mutually exclusive (`gpui-component` depends on
`gpui-pre`; you cannot mix it with the official `gpui`):

| | Official `gpui` 0.2.2 | `gpui-kit` / `gpui-component` 0.9 |
| --- | --- | --- |
| Source | Zed, official crate | Longbridge, pins `gpui-pre =0.3.11` (republished Zed snapshot) |
| Widgets | Low-level; build text input, lists, modals | 75+ incl. textarea, virtual list, markdown, editor, dock, charts |
| Markdown/streaming | Build it | Included (`markdown`, `stream-markdown` examples) |
| Accessibility/test | Basic | AccessKit + headless UI integration tests |
| Risk | API churn, more UI work | Exact snapshot pins, weekly upgrade pressure, third-party republish |

**Recommendation:** start with `gpui-kit`, pinned exactly, behind our own
`crates/ui` facade. The MVP is mostly text, lists, and markdown — rebuilding those
on raw GPUI would dominate the schedule. Confine the dependency so a later swap to
official `gpui` (or a fork) touches one crate. If the project has a strict
supply-chain policy, use official `gpui` and budget an extra milestone for
widgets.

### 6.5 Rendering performance

- Virtualize the transcript; do not build all messages every frame.
- Throttle markdown re-parsing of the streaming message (e.g., every 50–100 ms).
- Cache laid-out elements for finished messages; only the tail is mutable.
- Avoid `notify` storms: coalesce engine events, notify once per frame.

## 7. Concurrency model

| Thread | Owns | Never does |
| --- | --- | --- |
| GPUI main | entities, rendering, input | network, disk, `block_on` |
| Engine (tokio) | HTTP, SSE, sidecar I/O | touch entities |
| Worker (optional) | syntax highlighting, markdown parse | hold entity references |

Communication is channel-based (`EngineEvent` / `UiCommand`). No shared mutable
state between the UI and the network stack. Heavy CPU work (highlighting a large
message) goes to a worker and returns a plain render model.

## 8. App-local persistence

Only UI conveniences are stored by the app, under the platform app-data dir
(`~/Library/Application Support/<app>` on macOS):

- window geometry, last project path, last session ID
- panel sizes, theme override, stick-to-bottom preference
- unsent composer drafts per session

OpenCode's config and session storage are never modified directly. Provider auth
stays in OpenCode's credential store.

## 9. Security

- Loopback-only bind, random port, per-launch basic auth.
- Do not log prompts, message content, tokens, or credentials. Redact URLs that
  could carry secrets.
- Treat tool output and file contents as untrusted text when rendering (no
  automatic link/command execution).
- Bundle a pinned, checksummed OpenCode binary; verify before launch.
- Never auto-approve permissions in the MVP.

## 10. Packaging

- macOS first: `.app`, sidecar under `Contents/Resources`, hardened runtime,
  codesign both binaries, notarize.
- Universal app (arm64 + x86_64) needs a matching sidecar per architecture, or a
  universal sidecar if OpenCode ships one.
- Linux next (AppImage/deb). Windows is out of scope: GPUI's official docs target
  macOS and Linux.
- Size is a product decision: GPUI app is small; the OpenCode sidecar is not.
  Measure it before promising a download size.

## 11. Testing strategy

- **protocol**: serde fixtures recorded from a real server; round-trip tests.
- **core**: pure reducer tests for session/transcript/permission state — the bulk
  of correctness lives here.
- **engine**: integration tests that spawn `opencode serve` (CI installs it) and
  exercise health, sessions, prompt streaming, abort, and reconnect.
- **contract**: fetch `/openapi.json` (with credentials) and assert the MVP
  endpoints and event names still exist; fail loudly on drift.
- **UI**: `#[gpui::test]` / `TestAppContext` for input and focus behavior;
  screenshot review for visual regressions.
- **end-to-end**: scripted scenario against a fake model/provider where possible.

## 12. Milestones

| # | Milestone | Acceptance |
| --- | --- | --- |
| M0 | Workspace scaffold, sidecar boot, health, one window | App shows "ready" with a version; quitting leaves no orphan |
| M1 | Session list: create/select/rename/delete | Sessions mirror the server; restart preserves them |
| M2 | Transcript history | Opening a session renders ordered messages and parts |
| M3 | Prompt + streaming | Send a prompt; text streams into the transcript; abort works |
| M4 | Tool calls, reasoning, errors | Each renders distinctly and updates in place |
| M5 | Permissions | Requests appear, replies unblock the turn, UI stays responsive |
| M6 | Polish | Markdown, keyboard shortcuts, themes, reconnect UX |
| M7 | Packaging | Signed macOS app with bundled sidecar installs and runs clean |

## 13. Risks and open questions

| Risk | Mitigation |
| --- | --- |
| GPUI API churn / ecosystem split | Pin exact versions; isolate UI kit behind `crates/ui`; upgrade on a cadence |
| OpenCode v2 API instability across releases | Contract tests against `/openapi.json`; tolerate unknown events; pin and verify a tested sidecar version; keep routing in one adapter |
| Directory/location scoping | Resolved on v2.1.4: canonicalize the project path and pass it to `GET /api/session`; pin creates with `location.directory` |
| Markdown/streaming quality | Use `gpui-kit`'s markdown first; replace only if it fails |
| Sidecar startup latency | Show a real connecting state; start it during window creation |
| Sidecar size / distribution | Measure early; decide bundle vs first-run download in M7 |
| Windows | Out of scope; revisit if GPUI ships support |
| Licensing | All dependencies are Apache-2.0 or MIT. Reimplement UX; do not copy OpenChamber branding or assets. |

Open questions:

1. Which OpenCode version do we pin, and how do we communicate the tested range?
2. Do we allow connecting to an already-running server (advanced) in the MVP?
3. How do we present model/agent selection without duplicating OpenCode config?
4. What is the minimum viable error surface for provider auth failures?

## 14. Initial dependency set

```toml
# app
gpui = { package = "gpui-pre", version = "=0.3.11" }
gpui-component = "0.9.0"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "process", "time", "sync"] }
reqwest = { version = "0.12", features = ["json", "stream", "rustls-tls"] }
eventsource-stream = "0.2"
futures = "0.3"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
tracing = "0.1"
tracing-subscriber = "0.3"
directories = "6"
```

The exact GPUI entry point depends on the chosen base (`Application::new().run`
for official `gpui`; `gpui_platform::application().run` for the `gpui-pre`
snapshot). Encapsulate that difference in `crates/app` so the rest of the code is
unaffected.

## 15. Decision records

Architecture decisions live in [`docs/adr/`](adr/README.md). The foundational
choice — OpenCode as the engine, native GPUI as the client — is
[ADR 0001](adr/0001-use-opencode-as-agent-engine.md).
