# M11-02 — Worktrees

| | |
| --- | --- |
| Status | done |
| Wave | 2 |
| Size | L |
| Depends on | M11-01 |
| Parallel with | M10-03, M12-01, M14-03 |
| Owns | `crates/protocol/src/worktree.rs`, `crates/core/src/worktrees.rs`, `crates/engine/src/features/worktrees.rs`, `crates/views/src/worktree_picker.rs` |
| Shared touch | `protocol/src/lib.rs`, `engine/src/{lib,supervisor}.rs`, `views/src/workspace.rs` |

## 1. Goal

From a project, the user can create an isolated worktree on a new branch, open
a session inside it, and remove it later with its sessions. The app never
leaves an orphan worktree it created without telling the user.

## 2. Verified contract

| Endpoint | Shape |
| --- | --- |
| `GET /api/worktree` | query `projectID` → `Worktree.List` = `[{directory, strategy?}]` |
| `POST /api/worktree` | body `Worktree.CreateInput` → `Worktree.Info` = `{directory}` |
| `DELETE /api/worktree` | body `Worktree.RemoveInput` |
| `POST /api/worktree/refresh` | 204 |
| `POST /api/session/{sessionID}/move` | body `{directory, delivery?}` |

`Worktree.CreateInput` = `{projectID, from, branch, directory, name}`.
`Worktree.RemoveInput` = `{projectID, directory, force}`.

Verified against a live server (opencode 2.2.7, `/openapi.json`):

- `branch` is an existing git ref, not a new branch name. Omitted means the
  `HEAD` of `from`.
- `from` is the project directory (an `AbsolutePath`), not a ref.
- The server's default strategy creates a **detached** worktree; it does not
  create a branch. The app therefore treats the worktree as an isolated
  checkout of the chosen base ref, not as a feature branch.
- Omitting `directory` places the worktree under the server's own worktree
  root; the app never guesses that path.

Errors: `WorktreeError`, `ProjectNotFound`.
Events: `worktree.created`, `worktree.updated`, `worktree.resolved`,
`session.moved`.

## 3. Protocol

```rust
pub struct Worktree { pub directory: String, pub strategy: Option<String> }

pub struct CreateWorktreeRequest {
    pub project_id: String, pub from: String, pub branch: Option<String>,
    pub directory: Option<String>, pub name: String,
}

pub struct RemoveWorktreeRequest {
    pub project_id: String, pub directory: String, pub force: bool,
}
```

Fixture: a list with `strategy` present and absent; a create response with only
`directory`; a create body with and without `branch`.

## 4. Engine

Commands `ListWorktrees{project_id}`, `CreateWorktree{request}`,
`RemoveWorktree{request}`, `RefreshWorktrees{project_id}`,
`MoveSession{session_id, directory}`.

Events `WorktreesLoaded{project_id, worktrees}`,
`WorktreeCreated{worktree}`, `WorktreeRemoved{directory}`,
`SessionMoved{session_id, directory}`.

Resnapshot: reload the list on `StreamConnected`, on `worktree.*` events, and
when the picker opens.

## 5. Core

`WorktreeStore` per project: `list`, `upsert`, `remove`, plus an app-local
`created_by_app: Vec<String>` registry persisted with the project list.

Rules:

- Removing a worktree also removes sessions whose `location.directory` is that
  path, after confirmation; the server owns the deletion of session data.
- On launch, a registry path that no longer exists is dropped silently; a path
  that exists but is not in the server list is shown as "orphan" with Remove.
- Branch names are validated locally (no spaces, no `..`, no leading `-`) and
  the server error is shown verbatim if it still rejects. The base ref is
  optional. The server lists the project itself in the worktree collection, so
  `apply_snapshot` filters the project directory out of the rows.
- `force: true` is only used after the user confirms a dirty worktree.

## 6. UI

A worktree chip in the project header opens a picker: existing worktrees, a
"New worktree" form (name, optional base ref), and per-row Remove. The session
list groups sessions by worktree under the project. Removing a worktree shows
the absolute path and how many sessions will be deleted.

## 7. Non-goals

Merging a worktree branch back, pushing it, submodules, worktree-specific
config.

## 8. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| protocol | `worktree_list_parses_strategy_optional` | optional field |
| core | `worktree_orphan_detected_on_launch` | registry vs server |
| core | `worktree_remove_drops_sessions` | session cleanup |
| core | `base_ref_validation` | local guard |
| core | `project_root_is_not_a_worktree_row` | server lists the project |
| engine | `create_worktree_body_shape` | wire shape |

E2E (`tests/M11-02.rs`, no model gate): temp git repo with one commit,
`CreateWorktree`, assert the directory exists on disk and
`git worktree list` includes it; create a session with that directory, assert
`GET /api/session?directory=<worktree>` returns it; `RemoveWorktree`, assert
the directory is gone and `git worktree list` no longer has it. Durable
assertion: the filesystem and git, not local events.

## 9. Acceptance

- [x] A created worktree is a real directory registered with git, checked out
  from the chosen base ref (the server's strategy detaches `HEAD`; it does not
  create a branch).
- [x] A session opened there is scoped to it.
- [x] Remove deletes the directory and its sessions, or explains why not.
- [x] An app-created worktree is never left behind without a notice.

## 10. Risks

Disk growth and destructive removal. The confirmation must show the path; the
e2e test must never touch a developer's repository.
