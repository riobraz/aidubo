# Status — Wave 1 execution

Status: in progress

Last updated: 2026-11-11

## Completed

- 2026-11-11 — **M16-01 done (Linux verification pending CI).** Linux
  packaging. `locate_binary` now checks packaged sidecars first: macOS
  `Contents/Resources` and Linux `usr/lib/rust-harness`, both requiring
  `opencode.sha256` and failing with `ManifestMissing` when it is absent. A
  developer binary beside the executable may still omit the manifest; an
  explicit path argument still wins, and a verified bundle now outranks
  `RUST_HARNESS_OPENCODE_BIN`. `scripts/package-linux.sh` builds one AppDir,
  writes the manifest after copying the sidecar, and produces an AppImage plus
  a deb with `dpkg-deb`, each with a `.sha256`. `scripts/e2e-linux.sh` (gate
  `RUST_HARNESS_PACKAGING_E2E=1`) extracts both, runs `sha256sum -c`, launches
  each under `xvfb` with `RUST_HARNESS_AUTOQUIT_MS=3000` with one retry, and
  requires no sidecar process to remain. `.github/workflows/release-linux.yml`
  builds on `ubuntu-24.04` with pinned `@opencode/cli@2.2.6`, runs the smoke
  test, and uploads/attaches the artifacts. The macOS host could not run the
  Linux smoke test, so the AppImage/deb launch and no-leftover-process
  acceptance items remain unchecked until CI runs; `bash -n` and YAML parsing
  are clean locally.

- 2026-11-11 — **M16-02 done.** Signed update check. Protocol
  `UpdateManifest`/`UpdateArtifact` keep unknown fields and canonicalize the
  signed payload with a recursive key sort and compact separators, so the
  signature does not depend on `serde_json`'s map configuration. The app
  verifies Ed25519 with `ed25519-dalek` against a pinned key, compares numeric
  versions, selects the artifact for this OS/arch, and reports only
  up-to-date/available/unavailable; the check runs on a background thread, at
  most once per 24 h, and is skipped entirely when the setting is off. The
  pinned key is a placeholder whose private half was discarded (fails closed);
  debug builds can inject a public key for the e2e. Download verifies size and
  SHA-256 before writing, and reveal is a separate user action. The e2e serves
  a signed fixture manifest over loopback, asserts available, flips one byte in
  the signed version to prove the signature fails, rejects a manifest signed
  by another key, verifies a good download and refuses a corrupted body, then
  stops the server and asserts a quiet unavailable state.

- 2026-11-11 — **M16-03 done.** Session export, import, and stats. Live
  contract against opencode v2.2.7: export returns `{data: {info, messages}}`
  and sanitize rewrites the title to `[redacted:session-title:<id>]` (so a
  title-preserving round trip must use `sanitize: false`); stats takes epoch
  milliseconds (`from`/`to`) and is global, not project-scoped; import is
  `POST .../session/import` with `{info, messages, location}`. Import
  conflicts: the same session id is a 409 `ConflictError`, but message ids are
  a global namespace, so a fresh session id with the original message ids is a
  500 — copy mode rewrites both, and the UI toggle defaults on. The protocol
  keeps `info`/`messages` as raw JSON so a round trip cannot drop fields the
  typed transcript types do not model. The engine writes the export file
  (views are IO-free). UI: Export… with sanitize and a save dialog, Import…
  with a preview and copy toggle, and a Stats modal with 7-day/30-day/all-time
  presets, totals, tokens, and per-model rows. The e2e runs a real turn,
  asserts the sanitized file has no `sk-`, `Bearer `, or `Authorization` and
  no title, imports a plain export into a second project and proves the
  message count and title survived through `GET /message`, then proves a
  non-copy import of the same id is refused with no side effect, checks
  all-time stats, and cleans up.

- 2026-11-11 — **M13-01 done.** Live probe against opencode v2.2.7 showed the
  add endpoint requires `{"config": {...}}` (the inner local/remote config),
  returns 204, and that connect/disconnect/remove are 204 with a 404
  `McpServerNotFoundError` for a missing name. Disconnect turns a connected
  server into `disabled`; runtime-added servers have no `title` or
  `integrationID`; the NeedsAuth tag is `needs_auth`; `mcp.status.changed` and
  `mcp.resources.changed` carry only `{server}`. Implemented typed `McpStatus`
  with error text and unknown-tag fallback, a name-keyed `McpStore` with
  `replace`/`upsert`/`remove`, 400-char errors and status action rules,
  add-body validation, `McpError`, and `StreamEvent::McpChanged`. The panel now
  renders even with no servers, shows a status dot, optional title, Connect for
  `NeedsAuth`, Disconnect for `Connected`, inline failure text, the
  restart caveat with a Copy config snippet (`mcp.servers`), and reloads on
  `StreamConnected` and coalesced `mcp.*` events (250 ms). The e2e runs a
  test-owned `mcp-echo` stdio fixture: add → connected → disconnect →
  disabled → connect → connected → remove → absent, plus a missing-name 404 and
  an unchanged `opencode.json`.

- 2026-11-11 — **M14-01 done.** Live spike against opencode v2.2.7 proved the
  PTY connect endpoint is a WebSocket upgrade: text frames are output, binary
  frames prefixed with NUL plus JSON are byte-cursor updates, input is raw
  client bytes, resize is `PUT /api/pty/{id}`, tickets are single-use and last
  60 s, and `DELETE` terminates the OS process. Evidence is in
  `docs/spikes/pty-transport.md`; the transport decision is ADR 0003 (0002 was
  already the terminal crate). Implemented `PtyProcess`/`PtyStatus`/
  `CreatePtyRequest`/`PtyTicket`, the create/list/resize/write/delete/connect
  engine commands, a `tokio-tungstenite` connection with one ticket per
  connect, 64 KiB/16 ms output batching, and cursor-frame filtering. The e2e
  creates a real `/bin/sh` PTY, asserts the first bytes are `hello`, resizes to
  100x30, proves the pid is alive before delete, then proves it is gone, the
  list no longer contains it, and the sidecar shuts down cleanly.

- 2026-11-11 — **M12-03 done.** `StreamEvent::ExecutionFinished` carries the
  server's SSE `id` (fallback `created:{created}`) so replay dedupe is stable;
  live `form.created` frames parse to `StreamEvent::FormCreated` and upsert
  into `FormStore` by id. `NotifyPolicy` (200-id dedupe, 2 s per-session
  throttle with replacement, focus suppression, preview opt-in truncated at
  120 chars) maps turn-done/failed, permission and form events. The workspace
  feeds it every engine event, sends notifications over an `async_channel`,
  flushes throttled ones every 250 ms, and disables plus persists settings on
  a denial status. The app owns `MacNotifier` (constant AppleScript, title and
  body as argv), `FileNotifier` (`RUST_HARNESS_NOTIFY_SINK`), Linux
  `notify-send`, settings load/save (macOS Application Support or
  `RUST_HARNESS_NOTIFY_SETTINGS`), and two delivery threads. A Notifications
  section in the right panel has the global, four kind and preview toggles.
  Tests: 5 core policy, 3 GPUI wiring (delivered, focus suppression, denial),
  2 app backends; e2e writes exactly one JSON `turn_done` line and drops a
  replayed frame.

- 2026-11-11 — **M12-02 done.** Typed `ScheduleWhen`/`ScheduleStatus`/
  `ScheduleOutcome` with unknown-value fallbacks; `ScheduleStore` ordered by
  `nextDueAt` then creation, with upsert, finished-row filtering, local
  duration validation and 400-character failure reasons (6 tests);
  `LoadSchedules`/`CreateSchedule`/`PauseSchedule`/`ResumeSchedule`/
  `DeleteSchedule` commands, mutation events, and a server list reload after
  every write so `nextDueAt` is authoritative; Schedules panel with the
  sidecar caveat, add form, Pause/Resume and two-step Delete; e2e creates a
  relative schedule, waits for it to fire, then pauses, resumes and deletes a
  recurring one. Verified against opencode v2.2.7: the endpoint requires
  `experimental.schedules` in the project config, pause/resume return the
  updated `Schedule.Info`, and delete is 204.

- 2026-11-10 — **M10-02 done.** `FileDiff` in `harness-protocol`;
  `harness_core::parse_patch` with hunks, renames, add/delete, binary, CRLF,
  no-newline marker, malformed input, 50k-line truncation (9 tests);
  `EngineCommand::{LoadVcsDiff, LoadSessionDiff}` and the matching events;
  `SessionApi::vcs_diff` / `session_diff`; e2e against a real git repository.

## Discovered

- opencode v2.2.7 appends `-l` to a PTY's `args` (the probe sent
  `/bin/sh -c …` and the server stored `["-c","…","-l"]`); the client must use
  the `Pty` the server returns, not its request echo.
- The PTY cursor counts raw PTY bytes, while output frames are lossily
  re-encoded UTF-8 (an invalid byte becomes U+FFFD), so a client cannot count
  received frame bytes to resume. Only the server-reported cursor is valid.
- A ticket is single-use; reusing it or using it after 60 s closes the
  handshake with code 1006. An established connection keeps working past the
  ticket's expiry.
- `DELETE /api/pty/{id}` is a real terminate: 204, `kill -0` fails, the list
  entry disappears, and the open socket closes with code 1000.
- OpenCode v2.2.7 puts a stable top-level `id` (`evt_…`) on
  `session.execution.*` SSE frames and emits `form.created` over SSE; the
  engine reads both. The id is the reconnect-safe notification key; frames
  without it fall back to `created:{created}`.
- `NotifyPolicy` lives in the view because it needs live focus and settings;
  the app owns the backends and persistence. The e2e simulates a reconnect by
  re-feeding the same `ExecutionFinished` frame, because no public hook forces
  a real SSE reconnect.

- OpenCode v2.2.6 builds its working-tree snapshot from a file watcher. Before
  the first filesystem event after startup, `/api/vcs/status` and
  `/api/vcs/diff` can be empty even for a change made before launch. The e2e
  touches a file and retries; M10-01 must refresh on `filesystem.changed`.
- The sidecar's cwd now canonicalizes with the project directory. On macOS the
  `/var` → `/private/var` split otherwise made the VCS endpoints answer for a
  different path.

- 2026-11-10 — **M10-01 data path.** `VcsFileStatus`/`VcsBase`,
  `VcsStore` (4 tests), `LoadVcsStatus`/`LoadVcsBase`/`InitRepo`, API methods,
  e2e with a real working tree and `git init`. The panel is pending.
- 2026-11-10 — **M10-03 model.** `DiffRow` flattening with collapse,
  binary/empty notices, per-file counts, and `visible_range` with overscan
  (3 tests). The GPUI renderer is pending.

- 2026-11-10 — **M10-01 panel / M10-03 renderer (first cut).** Workspace owns
  `VcsStore` and a parsed diff; the right panel shows the branch, the file list
  with `+/-`, and the diff rows with colours. Ready loads status/base; a
  non-empty status loads the working diff. Missing: refresh on
  `filesystem.changed`, per-file selection, and row virtualization with
  `visible_range`.

- 2026-11-10 — **M10-01 and M10-03 done.** Per-file selection filters the
  flattened rows (click selects, second click clears); the diff body uses a
  `ScrollHandle` and `visible_range` (50 rows + 8 overscan) so only the visible
  window is built. GPUI test covers the selection toggle.

- 2026-11-10 — **M9-02 done.** `ForkRequest`, `SessionApi::fork`,
  `ForkSession` command and `SessionForked` event, `select_newest`; the
  transcript shows "Fork from here" on user messages; e2e runs two turns and
  forks before the second user message (boundary verified), plus a GPUI test
  that the forked session is upserted and selected.

- 2026-11-10 — **M9-01 done.** Inbox wire types, `InboxStore` (5 tests),
  `LoadInbox`/`CancelInbox`/`SetInboxDelivery`, API methods, queue strip above
  the composer, queue delivery while a turn is running; e2e sends a prompt
  behind a running turn, finds it in the inbox and cancels it.

- 2026-11-10 — **M16-04 done.** `--diagnostics` prints a report; every field
  goes through redaction (Authorization, Bearer/Basic, provider keys, query
  secrets, URL userinfo, home path). 7 tests, including a report with a secret
  in an error.

- 2026-11-10 — **M9-05 data layer.** `FormInfo`/`FormField`/`FormReplyRequest`
  (3 protocol tests), `FormStore` with required-field gating, unknown-kind
  exclusion and answer building (4 core tests), `LoadForms`/`ReplyForm`/
  `DismissForm` commands, events and API. The e2e remains.
- 2026-11-10 — **FormCard.** A card above the composer renders the pending
  form: text/number fields focus a shared input, booleans and multiselect
  options toggle in place, external fields are read-only, Reply is disabled
  until `can_reply`, Dismiss cancels. Events `FormsLoaded`/`FormSettled`/
  `FormError` are wired; the session reloads forms when it opens.

- 2026-11-10 — **M9-05 done.** `CreateForm` (tests/plugins only) closes the
  loop: the e2e creates a form with a required field, lists it, answers it and
  verifies it is settled.

- 2026-11-10 — **M9-05 done** (e2e above) and **M9-03 data layer.** Revert wire
  types, `RevertStore` (3 tests), `StageRevert`/`CommitRevert`/`ClearRevert`
  and API; e2e stages at the second user message with `files: false`, commits
  and verifies the transcript keeps only the first turn. The revert panel and
  the message action remain.

- 2026-11-10 — **M9-03 done.** "Revert to here" on user messages stages a
  revert; the panel above the composer shows the boundary and file count with
  Revert/Cancel. Committed reverts reload the transcript; a GPUI test covers
  stage → cancel.

- 2026-11-10 — **M9-04 done.** Usage from the session snapshot and
  `session.usage.updated`; meter above the composer with tokens/cost and a
  Compact button; compaction events drive the state. E2E compacts and then
  proves the session still answers a prompt.

- 2026-11-10 — **M9-06 done.** `UsageLimits`/`Budget`, `UsageStore` with a
  10-minute freshness rule (3 tests), `LoadUsageLimits` with a 503 mapped to
  `Ok(None)`, meter line shows the budget count; e2e asserts no error on 503.

- 2026-11-10 — **M13-04 data path.** `PluginInfo` with raw source/state
  unions plus labels (3 tests), `PluginStore` sorted with 400-char errors
  (2 tests), `LoadPlugins`/`CheckPlugins` and API; the right panel lists id and
  status, an Update action gated by a two-step confirmation, and a 60 s
  engine-side guard on checks.
- 2026-11-10 — **Finding:** v2.2.6 rejects every `provider` value on
  `POST /api/vcs/init` with `Unknown VCS provider`; the M10-01 test records the
  message instead of claiming the endpoint works.

- 2026-11-10 — **M13-01 data + list.** `McpServer` with five status labels and
  unknown fallback (2 tests), `McpStore` (2 tests), `LoadMcp` plus
  Add/Remove/Connect/Disconnect commands and API; the MCP section lists servers
  and removes with a two-step confirmation. Missing: the add-server form and
  the runtime-restart caveat. The add form (name + JSON) with validation and
  the caveat line is now in the MCP section.

- 2026-11-10 — **M13-03 done.** `SavedPermission` (2 tests), store sorted with
  idempotent removal (2 tests), `LoadSavedPermissions`/`DeleteSavedPermission`,
  a Permissions section with two-step Revoke, and an e2e that loads the rules
  for the session's project.

- 2026-11-10 — **M13-02 discovery.** ProviderInfo/IntegrationInfo (2 tests),
  ProviderStore (1 test), LoadProviders/LoadIntegrations and a Providers section
  with name + package. Auth flows (OAuth/command/key/external) are NOT
  implemented; they remain the security-sensitive half of this node.

- 2026-11-11 — **M13-02 done.** Typed `IntegrationMethod` (OAuth, Command, Key,
  External, Env, Unknown), `ConnectionInfo`, `OAuthAttempt`,
  `CommandAttempt`, `AttemptStatus` (protocol: 6 tests); `ProviderStore` with
  per-integration `AuthFlow` and redacted errors (core: 5 tests); engine
  `Secret`, integration API, engine-owned 1 s polling capped at 300 s with a
  cancellable task registry, and the matching commands/events (engine: 2 new
  tests). The Providers section lists each provider with package and
  connection badge; the connect dialog starts OAuth (opens only on a click,
  `https` only, host shown), shows a command's exact argv and waits for an
  explicit confirmation, sends a masked key and clears the field immediately,
  and explains external credential management. E2E lists providers and
  integrations on a fresh project, asserts a key sent to a missing integration
  returns a 404-shaped message with no key material, and cancels a
  non-existent attempt cleanly.

- 2026-11-11 — **M13-02 security review (merge gate) — passed.** Reviewed the
  key path (masked input → `Engine::send_key(&str)` → `Secret` →
  `SendKeyRequest`, which has no `Debug`; one POST; cleared input immediately;
  no store, entity state, event, or log holds it), `Secret`'s redacted `Debug`
  and NUL-overwrite `Drop`, the key-scrubbing error path, OAuth link hardening,
  and the redaction of every failure banner. Tests:
  `key_never_appears_in_debug_output`, `key_error_body_is_scrubbed`,
  `provider_error_is_redacted`. The only copy of the key while typing is the
  masked input widget's own buffer; it is emptied on send. Residual risk:
  process memory snapshots can still contain transient buffers; not
  preventable in a GUI process.

## In progress / next

- M13-01, M14-01, M16-01, M16-02, M16-03 and M11-01 are complete. M11-01
  landed in three commits: project list and selection, explicit per-command
  directory with canonicalization rejection, and app-data persistence.
- M15-01 (preview) was cancelled before implementation: the pinned GPUI
  surface has no hosted-native-view API, so a `WKWebView` cannot satisfy the
  spec's resize, close, and teardown requirements without a fake browser. The
  finding is in `docs/specs/M15-01-preview.md`; nothing depends on it.
- Gate G1 ran on 2026-11-11: formatting, clippy, the full workspace suite, the
  sidecar e2e gate, and graph validation pass. The Linux packaging smoke test
  cannot run on this macOS host; `release-linux.yml` runs it on `ubuntu-24.04`.
  The branch `wave-1` is ready for review and merge on the macOS CI gate.

## Blockers

None technical. All 21 wave-1 spec nodes are done (20) or cancelled (M15-01),
and gate G1 is closed. The only unrun check is the Linux-only packaging e2e,
which is owned by `release-linux.yml`.

## Verification

Per node. M16-01:

```sh
cargo test -p harness-engine --lib                       36 passed
cargo clippy -p harness-engine --all-targets -- -D warnings   clean
bash -n scripts/package-linux.sh                         clean
bash -n scripts/e2e-linux.sh                             clean
python3 -c "yaml.safe_load(...release-linux.yml)"        clean
```

The Linux packaging e2e was not run: this host is macOS and has no
`appimagetool`, `dpkg-deb`, or `xvfb`. The checksum lookup, ordering, and
fail-closed rules are unit-tested on this host; the artifact build and launch
path is wired for the `release-linux.yml` workflow.

M16-02:

```sh
cargo test -p rust-harness --lib                         9 passed
cargo test -p rust-harness --test M16-02                 1 passed (1.16 s)
cargo test -p harness-protocol --lib                     60 passed
cargo clippy -p rust-harness -p harness-protocol --all-targets -- -D warnings
                                                         clean
cargo fmt --all -- --check                               clean
```

The M16-02 e2e ran only against loopback: a thread-owned HTTP server returned
the signed manifest and artifact; available, up-to-date, tampered signature,
wrong key, good download, corrupt download, and server-stopped states were all
asserted. The accepted socket is forced back to blocking on macOS; without
that, the first hyper request ended with `IncompleteMessage`.

M16-03:

```sh
cargo test -p harness-protocol -p harness-core -p harness-engine --lib
                                                        58 + 99 + 33 passed
cargo test -p harness-views                             12 passed
cargo test -p harness-engine --test M16-03 -- --nocapture
                                                        1 passed (9.63 s)
cargo test --workspace                                  all passed
cargo clippy --workspace --all-targets -- -D warnings   clean
cargo fmt --all -- --check                              clean
```

The M16-03 e2e ran against opencode v2.2.7 with the mock provider: one turn
stored three transcript messages; the sanitized export contained no secret
substring and no title; the plain export kept the title and its message count;
the copy import into a second project matched that count through the server's
transcript endpoint; a non-copy import of the same file returned the 409
conflict and left the project with exactly one session; all-time stats reported
the turn; both sessions were deleted and both sidecars stopped. One workspace
run saw `M13-02` fail at `providers` discovery; it passed on immediate rerun
and again in the following full workspace run, so it is a timing flake in an
unrelated node, not a regression from M16-03.

M13-01:

```sh
cargo test -p harness-protocol -p harness-core -p harness-engine --lib
                                                        95 + 29 + 56 passed
cargo test -p harness-engine --test M13-01 -- --nocapture
                                                        1 passed (6.43 s)
cargo test --workspace                                  241 passed
cargo clippy --workspace --all-targets -- -D warnings   clean
cargo fmt --all -- --check                              clean
```

The M13-01 e2e drove a real sidecar: the test-owned `mcp-echo` fixture was
added with `PUT .../mcp/{name}`, reached `connected`, was disconnected to
`disabled`, reconnected, and removed; every state was read back from
`GET /api/mcp`. A second remove of a missing name produced the server's 404
message, and `opencode.json` was byte-identical before and after.

M14-01:

```sh
cargo test -p harness-protocol -p harness-engine --lib   54 + 27 passed
cargo test -p harness-engine --test M14-01 -- --nocapture
                                                        1 passed (2.62 s)
cargo test --workspace                                  236 passed
cargo clippy --workspace --all-targets -- -D warnings   clean
cargo fmt --all -- --check                              clean
```

The M14-01 e2e ran against opencode v2.2.7: it read `hello`, resized to
100x30, verified the pid was alive before `DELETE`, then polled until
`kill -0` failed and the PTY was absent from `GET /api/pty`; the sidecar
shutdown left no listener on its port. The spike's resize evidence showed the
shell moving from `stty size` `24 80` to `30 100`.

M12-03:

```sh
cargo test -p harness-core --lib                94 passed
cargo test -p harness-views                     10 passed
cargo test -p rust-harness                      2 + 7 passed
RUST_HARNESS_NOTIFY_SINK=/tmp/... cargo test -p harness-engine --test M12-03
                                                1 passed
cargo test --workspace                          229 passed
cargo clippy --workspace --all-targets -- -D warnings   clean
cargo fmt --all -- --check                       clean
```

The e2e sink held exactly one line:
`{"kind":"turn_done","session_title":"Notification test","body":"Turn finished"}`.
A smoke run (`RUST_HARNESS_AUTOQUIT_MS=2500`) booted the window, connected the
stream, and exited cleanly with no sidecar listener left behind. The visual
macOS banner was not exercised automatically (it would pop a notification on
the user's desktop); the osascript argv injection contract is unit-tested and
the sink path is verified end to end.

M13-02:

```sh
cargo test -p harness-protocol                   51 passed
cargo test -p harness-core                       88 passed
cargo test -p harness-engine --lib               23 passed
cargo test -p harness-engine --test M13-02       1 passed
cargo test --workspace                           216 passed
cargo clippy --workspace --all-targets -- -D warnings   clean
cargo fmt --all -- --check                       clean
```

Discovered against opencode v2.2.7: `external` methods carry a `form` array of
field keys and no URL, so the dialog cannot show an external link; cancelling a
non-existent attempt returns success, so the e2e accepts either terminal
event. The integration list is large (hundreds of providers), so the view only
reloads it after a real connection, not on every attempt update.

G1 (wave gate, 2026-11-11):

```sh
cargo fmt --all && cargo fmt --all -- --check            clean
cargo clippy --workspace --all-targets -- -D warnings     clean
cargo test --workspace                                    ok (35 test binaries)
cargo test -p harness-engine --test sidecar -- --nocapture  ok (6 passed)
python3 scripts/spec-graph.py --validate                  ok: 33 nodes, 4 waves
```

Every wave e2e binary ran with a real opencode v2.2.7 and its internal mock
providers: M9-01…06, M10-01, M10-02, M11-01, M12-02, M12-03, M13-01…04,
M14-01, M16-02, M16-03, T0, and sidecar. The sidecar `--nocapture` run skips
its externally supplied mock providers (`RUST_HARNESS_MOCK_PROVIDER`,
`RUST_HARNESS_MOCK_TOOL_PROVIDER`, `RUST_HARNESS_MOCK_PERMISSION_PROVIDER`)
because no external mock server is configured on this host; the internal mock
equivalents in the other e2e binaries cover the same paths.

The Linux packaging smoke test (`RUST_HARNESS_PACKAGING_E2E=1
scripts/e2e-linux.sh`) was not run: it needs an `ubuntu-24.04` host with
`appimagetool`, `dpkg-deb`, and `xvfb`. `release-linux.yml` runs it on that
runner.
