# Rust Harness — Roadmap after M7

Status: proposal · 2026-11-08

Executable form: this roadmap is the sequence; [`specs/`](specs/README.md) is
the per-feature contract set, with a dependency graph, waves, unit tests, and
e2e gates. Read the spec for a milestone before writing its plan.

This roadmap sequences the features worth building after the MVP (M0–M7). It is
derived from OpenChamber's shipped feature set, then filtered through what the
OpenCode v2.2.6 HTTP/SSE API actually exposes and what a native GPUI client can
support well. It is not a parity checklist: several OpenChamber features exist
only because it ships its own web server, database, and multi-surface runtime,
and those do not transfer to a single-user native app.

Sources:

- OpenChamber `README.md` and `packages/docs/content/docs/*` (feature behavior).
- Live OpenCode v2.2.6 `/openapi.json` (139 paths) and its event type list,
  fetched from a throwaway loopback server on 2026-11-08.
- `docs/ARCHITECTURE.md` and ADR 0001.

Verify endpoint shapes again before implementing a milestone; the live spec is
the contract, not this document.

## Where we are

| Milestone | State |
| --- | --- |
| M0–M5 | Sidecar, sessions, transcript, streaming, tools, permissions — done |
| M6 | Markdown, shortcuts, themes, reconnect UX — done |
| M7 | Signed macOS app with bundled, checksummed sidecar — done |
| — | Custom title bar — done |

The current client is one project, one window, one session at a time, with a
composer that sends plain text and a transcript that renders text, reasoning,
tools, errors, and permission requests.

## Principles for sequencing

1. **Deepen the core loop before widening the surface.** Rich prompts, message
   actions, and context control help every session; git review and terminals are
   used occasionally.
2. **Prefer server-native features.** If OpenCode owns the state (inbox
   delivery, forks, reverts, worktrees, schedules), the app is a view over it.
   Reimplementing state locally creates drift and migration work.
3. **Read-only before write.** Diff review is safe and high value; staging,
   committing, and pushing need a separate security and architecture decision.
4. **No new network or subprocess surface without an ADR.** Terminal, preview,
   git write operations, remote servers, and MCP management each introduce a new
   trust boundary.
5. **Every milestone keeps the invariants**: upsert by ID, resnapshot after
   reconnect, ignore unknown events, no auto-approve, loopback only, no
   prompt/content/token logging.

## Feature map

| OpenChamber capability | OpenCode v2.2.6 support | Rust Harness plan |
| --- | --- | --- |
| Chat, streaming, tools, permissions | Yes (M0–M6) | Done |
| `@` files, `/` commands, `$` skills, attachments | Prompt accepts `files`, `agents`, `skills`; `/api/fs/find`, `/api/command`, `/api/skill` | M8 |
| Model/agent selection | `/api/model`, `/api/agent`, per-session selectors | M8 |
| Queued and steering messages | Inbox with `delivery: "steer" \| "queue"` | M9 |
| Fork, revert, compact, context meter, cost/tokens | `/fork`, `/revert/*`, `/compact`, `/context`, `session.usage.updated` | M9 |
| Agent questions/forms | `/api/session/{id}/form/*`, `form.created` | M9 |
| Git status, branch, diff review | `/api/vcs/{status,diff,base,branch}`, `/api/session/{id}/diff` | M10 |
| Staging, commits, push, stash, PRs | **Not in the OpenCode API** | Deferred; needs ADR |
| Projects, worktrees, parallel model runs | `/api/project`, `/api/worktree`, worktree sessions | M11 |
| Goals, scheduled/recurring prompts, notifications | `/api/experimental/schedule`; goals are app-side; notifications are OS-side | M12 |
| MCP servers, providers, saved permission rules | `/api/mcp`, `/api/experimental/mcp/*`, `/api/provider`, `/api/integration`, `/api/permission/saved` | M13 |
| Embedded terminal | `/api/pty`, `/api/pty/{id}/connect-token`, session terminals | M14 |
| Preview + desktop browser + element picking | No server endpoint; app-side webview | M15 (optional) |
| Linux build, updates, export/import | `/api/experimental/session/{id}/export`, `/import` | M16 |
| Relay, mobile, web/PWA, VS Code, Slack/Matrix | Out of scope for a native single-user client | Icebox |
| Notes/todos/plans, snippets, recaps, auto-routing | OpenChamber-local features and tools | Icebox / design needed |

## Milestones

Sizes are relative (`S` ≈ days, `M` ≈ 1–2 weeks, `L` ≈ 3–5 weeks, `XL` ≈ more,
for one developer who knows the codebase). They are estimates, not commitments.

### M8 — Rich composer (`M`)

**Outcome.** A prompt can carry files, an agent, a skill, and a command, and the
user can switch the session's model and agent without opening OpenCode's config.

**OpenChamber counterpart.** Chat input: `/` commands, `$` skills, `#` snippets,
`@` files/agents, `+` attachments, image paste, drag-and-drop, shell mode (`!`).

**Server API (verified).**

- `GET /api/fs/find`, `GET /api/fs/list`, `GET /api/fs/read/*` — file discovery
  and content.
- `GET /api/command`, `GET /api/skill`, `GET /api/agent`, `GET /api/model`,
  `GET /api/model/default`.
- `POST /api/session/{id}/prompt` with `files[]`, `agents[]`, `skills[]` and
  `mention` offsets.
- `POST /api/session/{id}/command` for slash commands.
- `POST /api/session/{id}/model`, `POST /api/session/{id}/agent`.

**Design.**

- `PromptDraft` in `harness-core` owns text plus attachment chips and mention
  ranges; it serializes to the wire shape. Keep mention offsets recomputed on
  every edit instead of patching them in place.
- Pickers are a pure reducer: query, kind, results, highlighted index. The
  engine exposes `FindFiles`, `ListCommands`, `ListSkills`, `ListAgents`,
  `ListModels` commands; results carry a request ID so stale responses are
  dropped.
- Snippets are app-local text expansions and never hit the server.
- `harness-ui` gets attachment chips, picker popover, and model/agent badges.
- Shell mode (`!`) is included only if it reuses the permission/confirmation
  path; otherwise defer to M14.

**Acceptance.**

- `@` searches project files, inserts a mention, and the sent prompt contains
  the matching `files[]` entry with `uri` and `mention`.
- `/` runs a command via `/command`; `$` attaches a skill; `#` inserts text.
- Model and agent pickers reflect the server lists and persist per session.
- Large or missing results degrade to plain text; no panic on unknown variants.
- Protocol fixtures, picker reducer tests, engine request-body tests, and one
  GPUI trigger-to-selection test pass.

**Risks.** File search latency (debounce + cancel), huge/binary files (server
read limits; show a size error), model list size, and mention offset drift after
IME edits.

---

### M9 — Conversation control (`L`)

**Outcome.** The user can steer a running turn, queue the next one, fork or
revert to a message, compact context, see token/cost usage, and answer agent
questions — all without leaving the transcript.

**OpenChamber counterpart.** Chat actions, context panel, compaction, forms,
queued prompts, fork/revert, token and cost totals.

**Server API (verified).**

- `GET /api/session/{id}/inbox`; `PATCH`/`DELETE /api/session/{id}/inbox/{id}`;
  prompt `delivery: "steer" | "queue"`.
- `POST /api/session/{id}/interrupt` (already used by Stop).
- `POST /api/session/{id}/fork` with `{before}`.
- `POST /api/session/{id}/revert/stage` with `{messageID, files}`;
  `/revert/commit`, `DELETE /revert` (clear).
- `POST /api/session/{id}/compact`; `GET /api/session/{id}/context`.
- `GET /api/session/{id}/form`, `POST .../form/{formID}/reply`,
  `DELETE .../form/{formID}`; events `form.created`, `form.replied`,
  `form.cancelled`.
- `Session.Info` carries `tokens` and `cost`; event `session.usage.updated`.
- `GET /api/experimental/usage/limits` for provider budgets when available.

**Design.**

- Extend `TranscriptStore` with a per-message action model and a pending
  revert state. A staged revert is previewed before commit; `files: false`
  means transcript-only.
- Add `InboxStore` (queue vs steer, IDs) and `FormStore` (fields can be
  string/number/boolean/multiselect; unknown types render read-only).
- Context meter is computed in `harness-core` from usage data; never parse
  markdown to estimate tokens.
- Fork and revert are destructive-ish: confirm, then resnapshot the transcript
  and session list. Revert may change files on disk; show that explicitly.
- Forms must not auto-answer. Missing required answers disable Reply.

**Acceptance.**

- A prompt sent while running can be queued or steered; both appear in the
  inbox and can be removed before delivery.
- Fork creates a new session at the chosen boundary and it opens correctly.
- Revert stages, shows the affected messages/files, commits, or clears.
- Compaction keeps the session usable; the context meter updates from usage
  events and drops stale values after reconnect.
- A form of each supported field type round-trips; cancellation unblocks the
  turn.
- Reducer tests cover repeated/replayed events and unknown field types.

**Risks.** Revert semantics differ by OpenCode version; verify on the pinned
version with a real project before shipping. Usage numbers can be stale or
absent for some providers — show "unknown", not zero.

---

### M10 — Git and change review (`L`)

**Outcome.** A right-hand Changes panel shows the working tree status, branch,
and diffs, including the files a session changed, with large patches handled
without freezing the window.

**OpenChamber counterpart.** Git tab (staged/unstaged groups, diff review,
branches, history) and Changes Walkthrough.

**Server API (verified).**

- `GET /api/vcs/status`
- `GET /api/vcs/diff`
- `GET /api/vcs/base`
- `GET /api/vcs/branch`
- `POST /api/vcs/init`
- `GET /api/session/{id}/diff`
- Events `vcs.branch.updated`, `filesystem.changed`, `session.step.ended`.

**Scope decision.** This milestone is **read-only**. Staging, committing,
pushing, stashing, history browsing, and PRs are not in the OpenCode API. Doing
them means running `git` directly or adding a git library, which is a new trust
boundary and a new source of truth — it needs its own ADR before any code.

**Design.**

- `ChangedFile`/`FileDiff` types in `harness-protocol`; patch parsing and
  grouping in `harness-core` so the view only renders rows.
- A `DiffView` with virtualized rows, side-by-side or unified (start unified),
  and per-file collapse. Cap syntax highlighting to the visible range.
- Refresh on `filesystem.changed` with debounce; resnapshot on stream reconnect.
- Patch text is untrusted: render as text, never as HTML or clickable commands.

**Acceptance.**

- The panel lists modified/added/deleted files with insertion/deletion counts
  and status; selecting one shows its diff.
- The branch indicator updates on `vcs.branch.updated`.
- A 5,000-line patch scrolls at frame rate and does not reparse per frame.
- Binary files and `more: true` truncated patches show a clear notice.
- Tests: patch parser fixtures (including malformed patches), reducer tests for
  repeated `filesystem.changed` events, one GPUI diff-rendering test.

**Risks.** Diff parser edge cases (renames, binary, CRLF), very large patches,
and scope creep into write operations.

---

### M11 — Projects, worktrees, and multi-run (`XL`)

**Outcome.** Several projects are open in one window/sidebar; a worktree can be
created for isolated work; the same prompt can run on several models in
isolated worktrees and be compared, then kept or discarded.

**OpenChamber counterpart.** Projects, project actions, worktree rows, multi-run
(up to five models/worktrees), session grouping.

**Server API (verified).**

- `GET /api/project`, `PATCH /api/project/{id}`.
- `GET/POST/DELETE /api/worktree`, `POST /api/worktree/refresh`;
  `Worktree.CreateInput {projectID, from, branch, directory, name}`.
- Sessions carry `projectID`; `POST /api/session/{id}/move` moves a session.
- Events `project.updated`, `worktree.updated`, `worktree.resolved`,
  `session.moved`.
- `Location` query/body fields scope requests to a directory.

**Design.**

- Replace the engine's single `project_dir` with a `ProjectStore` and a list of
  locations. The sidecar is still one process; the engine routes each request
  with the right location and scopes session IDs to projects.
- App-local persistence gains the project list and last active project.
- A run group is app-local metadata: prompt, model set, session IDs, worktree
  IDs, state. Each run is a normal session in its own worktree.
- Comparison view: tabs or a two/three-column transcript view; on narrow
  windows fall back to tabs. "Keep" marks the winner; "Discard" deletes the
  worktree and its sessions after confirmation.
- Cleanup: track worktrees created by the app; offer removal on discard and
  detect orphans on launch.

**Acceptance.**

- Two projects open at once; sessions are listed and created under the right
  project; switching projects keeps drafts and scroll positions.
- A worktree can be created from a chosen branch, a session opened in it, and
  the worktree removed with its sessions.
- A prompt runs on three models in isolated worktrees; one failure does not
  cancel the others; comparison shows each transcript; discarding one leaves
  the others usable.
- No orphan worktree or session is left after discard; disk paths are shown.

**Risks.** This is the largest architectural change since M0: location routing,
project-scoped state, and cleanup. Do it as several plan slices (projects →
worktrees → multi-run) rather than one. Guard against branch-name collisions and
unbounded disk growth.

---

### M12 — Automation: goals, schedules, notifications (`L`)

**Outcome.** A goal keeps working until the objective is verified done or a cap
is hit; saved prompts run on a schedule while the app is running; the OS
notifies the user when a turn finishes, fails, asks a question, or needs
permission.

**OpenChamber counterpart.** Session goals, scheduled tasks/recurring prompts,
native notifications, "run as goal".

**Server API (verified).**

- `GET/POST /api/experimental/schedule`, `DELETE /api/experimental/schedule/{id}`,
  `/pause`, `/resume`; `Schedule.CreatePayload {sessionID, prompt, when, if,
  until, ends, name}`.
- Turn lifecycle events: `session.idle`, `session.execution.failed`,
  `permission.asked`, `form.created`.
- No goal endpoint: goals are an app-side loop.

**Design.**

- **Goals** live in the engine as a supervised task: on `session.idle`, queue an
  auditor run in a *separate* small session using a cheap model; the auditor
  sees the objective and the latest reply only, and answers
  continue/finish/block. Enforce a token budget, a continuation cap, and a
  stop on error. Persist goal state app-locally so a relaunch can report
  "interrupted", not silently resume.
- **Schedules** are a view over the server's schedules. The sidecar must be
  running; state that clearly in the UI. Loop files (OpenChamber's
  `.agents/loops/*.md`) are out of scope for this milestone.
- **Notifications** use the OS notification center on macOS, triggered by
  events. Default to session title only; previews of message text are opt-in,
  because lock screens are public. Deduplicate by event ID/sequence so
  reconnects do not spam.

**Acceptance.**

- A goal continues after each idle until verified done, a cap, or a manual
  stop; the UI shows the auditor's decision and the remaining budget.
- A schedule fires, creates a run, records success/failure, and can be paused
  and resumed; it does not fire while the sidecar is stopped.
- Notifications respect per-event toggles and never include credentials or
  full message content by default.
- Quitting mid-goal leaves no runaway sidecar work; next launch shows the goal
  as interrupted.

**Risks.** Runaway cost is the main one: caps are mandatory, and the auditor
must use a cheap model. App sleep and clock changes affect schedules; defer to
the server where possible. Notification permission denial must degrade
silently.

---

### M13 — Integrations and tool management (`L`)

**Outcome.** Users can see and manage MCP servers, provider accounts, saved
permission rules, and plugin status from the app, without editing config files
by hand.

**OpenChamber counterpart.** Settings → MCP, providers, permissions,
extensions/integrations.

**Server API (verified).**

- `GET /api/mcp`, `GET /api/mcp/resource`; `PUT/DELETE
  /api/experimental/mcp/{server}`, `/connect`, `/disconnect`; events
  `mcp.status.changed`, `mcp.resources.changed`.
- `GET /api/provider`, `/api/provider/{id}`; `GET /api/integration`,
  `/api/integration/{id}` and the `connect/{oauth,command,key,external}` flows.
- `GET/PATCH/DELETE /api/permission/saved/{id}`; `GET /api/permission/saved`.
- `GET /api/plugin`, `/api/plugin/check`, `/api/plugin/gui` (read-only status).
- `GET /api/websearch/provider`; `POST /api/websearch`.

**Design.**

- **MCP**: list servers and status; add/edit server at runtime via the
  experimental endpoints. Be explicit that runtime-only MCP configuration may
  not survive a sidecar restart, and offer a "copy config" snippet instead of
  writing `opencode.json`. Never write OpenCode's config directly.
- **Provider/integration auth**: OAuth opens an external browser and completes
  through the attempt endpoints; API-key entry uses a masked field and lives
  only in memory until sent. Command-based auth shows the exact command and
  requires confirmation. Never log request bodies or tokens.
- **Saved permissions**: read, edit, delete rules created by "Always allow".
  The app still never auto-approves new requests.
- **Plugins**: show installed/update status; no arbitrary install execution
  without confirmation.

**Acceptance.**

- MCP server list shows connected/disconnected/error and survives refresh.
- Adding a runtime MCP server makes its tools available to a new turn; a
  restart caveat is visible.
- Provider list shows auth status; an OAuth attempt can be started and
  cancelled; no token appears in logs or error toasts.
- A saved rule can be removed and no longer applies.
- Security review signs off on secret handling and external URL handling.

**Risks.** Credential handling is the highest-risk surface in this roadmap.
Experimental endpoints may change shape. External URLs opened from model output
must never be auto-opened; only user actions open browsers.

---

### M14 — Terminal (`L`–`XL`)

**Outcome.** A dockable terminal runs in the project directory, survives
switching sessions, supports resize/scrollback/copy-paste, and leaves no orphan
process on quit.

**OpenChamber counterpart.** Integrated terminal (libghostty-vt based), reattach,
touch support.

**Server API (verified, needs a spike).**

- `POST /api/pty`, `GET/DELETE /api/pty/{id}`.
- `POST /api/pty/{id}/connect-token` (ticket + expiry).
- `GET /api/pty/{id}/connect` with `ticket` and `cursor`; the OpenAPI document
  describes JSON responses, so the streaming transport must be confirmed with a
  spike before committing to a design.
- `GET/POST /api/experimental/session/{id}/terminal` and `/terminal/read` as an
  alternative for session-scoped persistent terminals.
- Events `pty.created`, `pty.updated`, `pty.exited`, `pty.deleted`.

**Design.**

- New `harness-term` module (engine side) owning the socket, resize messages,
  and reconnect with a cursor.
- VT parsing and a grid model in a pure crate (evaluate `vte` +
  `alacritty_terminal` or a minimal parser); `harness-core` exposes a render
  model of styled cells/rows.
- GPUI renders only the visible viewport; a terminal is a text grid, not
  markdown.
- Input encoding (bracketed paste, modifiers, IME) is its own test area.

**Acceptance.**

- Open a terminal, run a command, see output, resize, scroll back, copy/paste,
  detach and reattach, and see the exit code.
- Closing the app or the tab terminates PTYs; no process remains listening.
- A burst of 10 MB output does not freeze the UI; output is batched per frame.
- VT tests use recorded byte streams; no live shell in unit tests.

**Risks.** VT completeness, rendering performance, WebSocket auth, and orphan
processes. If the transport spike fails, fall back to the session terminal
endpoint or defer the milestone.

---

### M15 — Preview and desktop browser (optional, `XL`)

**Outcome.** A dev server can be previewed beside the chat; a picked DOM element
or drawn annotation becomes a reference in the composer.

**OpenChamber counterpart.** Preview panel, desktop browser, element picking,
remote preview via tunnels.

**Server API.** None. This is entirely app-side.

**Design.**

- macOS: embed `WKWebView` through `objc2`/`objc2-web-kit` in a GPUI-hosted
  native view. Start with a URL bar and localhost-only navigation.
- Detect listening local ports only as a convenience; always let the user type a
  URL. Do not scan the network.
- Element picker injects a small script (only into the preview webview) and
  returns a selector + text snippet; insert it as text/attachment, never as an
  executable instruction.
- Remote preview/tunnels are out of scope.

**Acceptance.**

- `localhost:5173` renders, reloads, and survives window resize.
- Element picking inserts a stable selector and a short snippet into the draft.
- Non-local URLs are blocked by default with an explicit override.
- Disabling the panel destroys the webview and its process.

**Risks.** WKWebView integration with GPUI is unproven; macOS-only; adds process
and memory surface; page content is untrusted. Treat this as the first feature
that may be cut.

---

### M16 — Distribution and portability (`M`–`L`)

**Outcome.** Linux builds, update checks, session export/import, and safer
diagnostics.

**Server API (verified).**

- `GET /api/experimental/session/{id}/export`
- `POST /api/experimental/session/import`
- `GET /api/experimental/session/stats`
- `GET /api/experimental/usage/limits`

**Design.**

- Linux packaging (AppImage or deb) with the same checksummed sidecar rule.
- Update check: fetch a signed manifest, verify signature, never auto-install
  silently; macOS can use Sparkle later.
- Export/import a session as a file from the app; no direct storage access.
- "Copy diagnostics" builds a redacted report (versions, engine status, last
  errors, no prompts/content/tokens/URLs with secrets).

**Acceptance.**

- Linux bundle launches, verifies the sidecar, and exits cleanly.
- Update check reports "up to date", "available", or "unavailable" without
  blocking startup.
- Exported session imports into a fresh project with messages intact.
- Diagnostics contain no message content, tokens, or credential-bearing URLs.

**Risks.** Platform-specific packaging and signing; the update channel is a
security-sensitive download path.

## Dependency order

```
M8 Rich composer
   ↓
M9 Conversation control
   ↓
M10 Git review (read-only)
   ↓
M11 Projects → worktrees → multi-run
   ↓
M12 Automation (goals/schedules/notifications)
   ↓
M13 Integrations & tools
   ↓
M14 Terminal
   ↓
M15 Preview (optional)
   ↓
M16 Distribution & portability
```

M8 and M9 are independent of git and can ship without changing the architecture.
M11 is the only milestone that materially changes the engine's location model;
do it in slices and update `ARCHITECTURE.md` §5 and §6.2 as part of the plan.
M15 is the only milestone that may be cancelled without weakening the product.

## Cross-cutting work

Every milestone includes these, not as a separate phase:

- **Contract tests.** Record the live `/openapi.json` subset used by the
  milestone and assert paths, required fields, and event names still exist.
  Fail loudly on drift; tolerate unknown additions.
- **Event expansion.** Add new events to `StreamEvent` with a catch-all;
  protocol fixtures must cover unknown variants and missing optional fields.
- **Resnapshot.** Any new state (diffs, inbox, forms, worktrees, PTYs) reloads
  on stream reconnect and after a turn ends.
- **Performance.** Coalesce per frame, virtualize lists and diffs, batch
  terminal output, parse patches off the render path.
- **Security review.** New file, process, credential, or web surfaces get a
  written threat check in the plan and a security review before merge.
- **Accessibility and keyboard.** Every picker, dialog, and panel is reachable
  and dismissible from the keyboard; focus does not get trapped.
- **Docs.** Each milestone updates `README.md`, `ARCHITECTURE.md` where the
  architecture changes, and its `STATUS.md` at every step.

## Icebox: considered and deliberately not planned

| Feature | Why not now |
| --- | --- |
| Private Relay, QR pairing, mobile apps, web/PWA | Requires a hosted relay, push infrastructure, and a security model far beyond a local desktop client. |
| VS Code extension, Slack/Matrix extensions | Other surfaces; the native app is the product. |
| Multi-window, Mini Chat, collaboration | GPUI can do multiple windows, but state ownership and focus rules are a large refactor with little payoff at one project per user. |
| Remote instances, SSH, desktop tunnels | Could be a later "connect to a server URL" mode, but it changes the trust boundary from loopback child to network peer. Needs an ADR and TLS/auth design. |
| Staging/commit/push/stash, PR review | Not exposed by the OpenCode API. Would make the app a second git client; needs an ADR and a separate command runner. |
| Notes/todos/plans with agent tool access | OpenChamber implements its own knowledge tool and server database. A native equivalent needs an MCP server or OpenCode plugin plus app storage — a product of its own. Revisit after M13. |
| Snippets, recaps, auto model routing, "looks done" hints | Cheap app-local heuristics, but low value until the core loop and automation exist. Snippets ship in M8; the rest stay optional. |
| Embedded code editor, file tree | Explicit non-goal; use the user's editor. An "open in editor" action is enough. |
| Voice input, artifact viewer | Experimental server APIs with unclear stability; OS dictation covers most voice use. Revisit after the pinned version changes. |

## Definition of done for a milestone

1. A checked-in plan (`docs/plans/<name>/PLAN.md`) with goal, context, scope,
   design, steps, test plan, and risks.
2. `STATUS.md` updated at start, after each step, and when done.
3. `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D
   warnings`, and `cargo test --workspace` pass.
4. Protocol fixtures cover unknown variants; sidecar tests still skip cleanly
   without the binary.
5. Manual verification on macOS (and Linux when claimed) with real server
   effects, not only local events.
6. `README.md` and `ARCHITECTURE.md` updated if status, commands, or structure
   changed.

## Verification log

- 2026-11-08 — OpenCode v2.2.6 `/openapi.json` inspected from a loopback server:
  session, inbox, fork, revert, compact, context, form, vcs, worktree, pty,
  schedule, usage, permission, mcp, integration, project, and export/import
  paths confirmed.
- 2026-11-08 — Event list confirmed: `session.usage.updated`,
  `session.inbox.*`, `form.*`, `vcs.branch.updated`, `worktree.*`, `pty.*`,
  `mcp.status.changed`, `filesystem.changed`, and turn lifecycle events.
- 2026-11-08 — OpenChamber docs reviewed for chats, chat input, context,
  sessions-in-work, multi-run, session goals, git, preview, notes/todos/plans,
  scheduled tasks, notifications, and MCP.
