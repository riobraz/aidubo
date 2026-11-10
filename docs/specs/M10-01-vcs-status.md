# M10-01 — Git status, base, and branch list

| | |
| --- | --- |
| Status | in-progress |
| Wave | 1 |
| Size | M |
| Depends on | T0 |
| Parallel with | M9-*, M10-02, M13-* |
| Owns | `crates/protocol/src/vcs.rs`, `crates/core/src/vcs.rs`, `crates/engine/src/features/vcs.rs`, `crates/views/src/changes_panel.rs` |
| Shared touch | `protocol/src/lib.rs`, `engine/src/{lib,supervisor}.rs`, `views/src/workspace.rs` |

## 1. Goal

A Changes panel shows the working tree: every modified, added, deleted, and
renamed file with insertion/deletion counts, the current branch, and the base
ref used for comparison. It refreshes when files change, without blocking the
UI.

## 2. Verified contract

| Endpoint | Shape |
| --- | --- |
| `GET /api/vcs/status` | query `location[directory]` → `{location, data: [Vcs.FileStatus]}` |
| `GET /api/vcs/base` | query `location[directory]` → `{location, data: Vcs.Base \| null}` |
| `GET /api/vcs/branch` | query `location[directory]`, `search`, `limit` → `{location, data: [string]}` |
| `POST /api/vcs/init` | 204 |

`Vcs.FileStatus` = `{file, additions, deletions, status}`.
`Vcs.Base` = `{name, ref, source: "reflog" | "default"}`.
Errors: 404 `LocationNotFound`, 503 `ServiceUnavailable`.
Events: `vcs.branch.updated`, `filesystem.changed`.

`/api/vcs/branch` is the **branch list**, not the current branch. The current
branch comes from `vcs.branch.updated` and from `Vcs.Base.name`; the list feeds
the worktree picker in M11-02.

## 3. Protocol

```rust
pub struct ChangedFile { pub file: String, pub additions: u64, pub deletions: u64,
                         pub status: String }

pub struct VcsBase { pub name: String, pub reference: String, pub source: String }

pub enum VcsChange { Added, Modified, Deleted, Renamed, Untracked, Conflicted, Unknown(String) }
```

`VcsChange::from_wire(&str)` maps known words and keeps the rest in
`Unknown`, so a new server status never hides a file.

## 4. Engine

Commands `LoadVcsStatus{request_id}`, `LoadVcsBase{request_id}`,
`ListBranches{request_id, search, limit}`, `InitRepo`. Events
`VcsStatusLoaded{request_id, result}`, `VcsBaseLoaded{request_id, result}`,
`BranchesLoaded{request_id, result}`, `RepoInitialized`.

Debounce: `filesystem.changed` schedules a status reload at most once per
250 ms; the supervisor coalesces bursts and drops stale request ids.

Resnapshot: reload status and base on `StreamConnected` and when the panel
opens.

## 5. Core

`VcsStore`: `files`, `base`, `branch`, `loading`, `error`, `last_refresh`.

Rules:

- A snapshot replaces the file list; no merge, because deletions must vanish.
- `vcs.branch.updated` updates only the branch label.
- Files are sorted by status group, then path; grouping is computed once per
  snapshot.
- `InitRepo` failure surfaces as a short message; a non-git directory shows
  "Not a git repository" with an Initialize action.

## 6. UI

Right panel, "Changes" tab: branch chip, base name, groups (Changes / Staged /
Untracked when the server distinguishes them), per-file counts, click selects
(M10-03 renders the patch). Empty state: "No changes". Keyboard: Cmd/Ctrl+Shift+G
focuses the panel, Up/Down moves the selection.

## 7. Non-goals

Staging, committing, pushing, stashing, history browsing, PRs. Those are not in
the OpenCode API and need an ADR before any code.

## 8. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| protocol | `vcs_status_parses_counts` | numbers, not strings |
| protocol | `vcs_change_unknown_status_is_kept` | forward compatibility |
| core | `vcs_snapshot_replaces_removed_files` | deletions vanish |
| core | `vcs_debounce_collapses_bursts` | one refresh per 250 ms |
| core | `vcs_branch_event_updates_label_only` | no status reload |

E2E (`tests/M10-01.rs`, no model gate): create a temp git repo with one
committed file, modify it, start the engine, `LoadVcsStatus`, assert the file
appears with `status == "modified"` and non-zero counts; `LoadVcsBase` returns
a ref or null; run `InitRepo` in a second non-git directory and assert
`.git` exists on disk. Durable assertion: `git status --porcelain` matches the
panel's file set.

## 9. Acceptance

- [ ] A modified file appears with correct counts.
- [ ] A deleted file disappears from the list on the next refresh.
- [ ] A non-git directory offers Initialize instead of an error loop.
- [ ] No refresh storm while a build writes many files.

## 10. Risks

`filesystem.changed` can fire hundreds of times per second during a build.
The debounce is mandatory; the e2e test asserts the collapse.
