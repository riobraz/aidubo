# M9-03 — Stage, preview, and commit a revert

| | |
| --- | --- |
| Status | ready |
| Wave | 1 |
| Size | M |
| Depends on | T0 |
| Parallel with | M9-01, M9-02, M9-04 … M9-06 |
| Owns | `crates/protocol/src/revert.rs`, `crates/core/src/revert.rs`, `crates/engine/src/features/revert.rs`, `crates/views/src/revert.rs` |
| Shared touch | `protocol/src/lib.rs`, `engine/src/{lib,supervisor}.rs`, `core/src/transcript.rs`, `views/src/workspace.rs` |

## 1. Goal

The user can roll a session back to a chosen message. Staging is a preview:
the app shows which messages disappear and which files change, and nothing is
written until commit. A staged revert can be cleared.

## 2. Verified contract

| Endpoint | Shape |
| --- | --- |
| `POST /api/session/{sessionID}/revert/stage` | body `{messageID, files: bool \| null}` → `{data: Session.Revert}` |
| `POST /api/session/{sessionID}/revert/commit` | 204 |
| `DELETE /api/session/{sessionID}/revert` | 204 |

`Session.Revert` = `{messageID, partID, snapshot, files: [FileDiff.Info]}`.
Errors: 409 `SessionBusy` (stage/commit while a turn runs), 404
`MessageNotFound`. Events: `session.revert.staged`, `session.revert.committed`,
`session.revert.cleared`.

`files: false` reverts the transcript only; `true` also restores files.
Default in the UI is `false`, with an explicit checkbox to include files.

## 3. Protocol

```rust
pub struct RevertStageRequest { pub message_id: String, pub files: Option<bool> }

pub struct SessionRevert {
    pub message_id: String,
    pub part_id: Option<String>,
    pub snapshot: Option<String>,
    pub files: Vec<FileDiff>,
}
```

`FileDiff` is introduced here and reused by M10-02; if M10-02 lands first,
import it instead of duplicating. Fixture: a revert with files, one with an
empty `files` array, one missing `snapshot`.

## 4. Engine

Commands `StageRevert{session_id, message_id, files}`, `CommitRevert{
session_id}`, `ClearRevert{session_id}`. Events `RevertStaged{revert}`,
`RevertCommitted{session_id, message_id}`, `RevertCleared{session_id}`.

Resnapshot: after commit the transcript is reloaded from the server; the app
never truncates locally as the source of truth. A 409 maps to "Stop the
running turn before reverting".

## 5. Core

`RevertStore` per session: `stage(revert)`, `commit()`, `clear()`, and
`affected_messages(&TranscriptStore, message_id) -> Vec<Entry>`.

Rules:

- `staged` replaces any previous staged revert (the server keeps one).
- `committed` clears the staged state and marks the transcript for reload.
- `cleared` is idempotent.
- A commit without a stage is ignored (possible after reconnect).

## 6. UI

The message overflow menu gains "Revert to here…". Choosing it stages and opens
a panel above the composer: "N messages will be removed", the file list with
additions/deletions, a checkbox "Also restore files", and **Revert** /
**Cancel**. The panel blocks sending until resolved. Keyboard: Esc cancels,
Cmd/Ctrl+Enter commits.

## 7. Non-goals

Undo after commit, partial revert of a single part, reverting across forks.

## 8. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| protocol | `revert_stage_parses_with_and_without_files` | optional fields |
| core | `revert_stage_replaces_previous` | one staged revert |
| core | `revert_commit_without_stage_is_ignored` | reconnect safety |
| core | `revert_affected_messages_counts_from_boundary` | preview math |
| engine | `revert_stage_serializes_files_false` | explicit false is sent |

E2E (`tests/M9-03.rs`, gate `RUST_HARNESS_MOCK_SCRIPTED_PROVIDER`): two turns,
stage at the second user message with `files: false`, assert `files` is empty
and the message count is unchanged; commit; assert `GET /message` returns only
the first turn; stage again and clear; assert the transcript is intact.
Durable assertion: server message counts before and after commit.

## 9. Acceptance

- [ ] Stage never changes the transcript.
- [ ] Commit removes exactly the messages from the boundary on.
- [ ] `files: true` lists the affected files before commit.
- [ ] 409 during a running turn is shown and nothing is committed.

## 10. Risks

Revert can rewrite files on disk. The UI must say so before commit, and the
e2e test must use `files: false` except for one explicit file-restore case in
M10-02's fixtures.
