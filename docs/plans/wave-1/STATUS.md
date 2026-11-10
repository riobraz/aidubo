# Status — Wave 1 execution

Status: in progress

Last updated: 2026-11-10

## Completed

- 2026-11-10 — **M10-02 done.** `FileDiff` in `harness-protocol`;
  `harness_core::parse_patch` with hunks, renames, add/delete, binary, CRLF,
  no-newline marker, malformed input, 50k-line truncation (9 tests);
  `EngineCommand::{LoadVcsDiff, LoadSessionDiff}` and the matching events;
  `SessionApi::vcs_diff` / `session_diff`; e2e against a real git repository.

## Discovered

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

## In progress / next

- M10 is complete (M10-01, M10-02, M10-03).
- M9-05: form card UI + e2e, then M9-03, M9-04, M9-06, integrations,
  automation, PTY, distribution and projects.
- Gate G1 has not run; the branch `wave-1` is not ready to merge.

## Blockers

None technical. Remaining work is volume: 20 of 21 nodes.

## Verification

Per node. M10-02:

```sh
cargo test -p harness-core           45 passed
cargo test -p harness-protocol       16 passed
cargo test -p harness-engine --test M10-02   1 passed
cargo test -p harness-engine --test M10-01   2 passed
cargo test --workspace                       121 passed
```
