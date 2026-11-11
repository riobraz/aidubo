# M11-01 — Multiple projects and location routing

| | |
| --- | --- |
| Status | done |
| Wave | 1 |
| Size | L |
| Depends on | T0, E0 |
| Parallel with | M9-*, M10-*, M13-* |
| Owns | `crates/protocol/src/project.rs`, `crates/core/src/projects.rs`, `crates/engine/src/features/projects.rs`, `crates/views/src/project_switcher.rs`, `crates/app/src/persistence.rs` |
| Shared touch | `protocol/src/lib.rs`, `engine/src/{lib,supervisor}.rs`, `core/src/sessions.rs`, `views/src/workspace.rs`, `app/src/main.rs` |

## 1. Goal

Several projects are open in one window. The sidebar groups sessions by
project, creating a session uses the selected project's directory, and every
request carries the right `location` so a project never sees another's
sessions. The app remembers the project list and the last active project.

This is the only node that changes the engine's location model. Do it in the
three slices below, in order, each with its own commit.

## 2. Verified contract

| Endpoint | Shape |
| --- | --- |
| `GET /api/project` | query `archived` (`true\|false`) → `[Project]` |
| `PATCH /api/project/{projectID}` | body `Project` fields (name, icon, commands) → `Project` |

`Project` = `{id, canonical, vcs, name, icon, commands, time, sandboxes}`;
`vcs` is a short string (`"git"`), `commands.start` is the worktree startup
script. Events: `project.updated`. Sessions carry `projectID` and
`location.directory`.

## 3. Slices

### Slice 1 — project list and selection

- `ProjectStore { projects, active: Option<ProjectId> }` with `replace`,
  `upsert`, `select`, `active_canonical`.
- `EngineCommand::{ListProjects, SelectProject{id}}`; the supervisor keeps the
  active canonical directory and scopes `ListSessions`/`CreateSession` with it.
- Sidebar shows a project switcher; the active project's sessions are listed.

### Slice 2 — per-command location

- Every command that reads or writes project state gains an explicit
  `directory: String` (sessions, transcript, prompt, catalogs, files, vcs,
  permissions). No implicit "current project" in the engine.
- The supervisor canonicalizes once at the boundary and rejects a
  non-canonical path with a clear message (macOS `/tmp` → `/private/tmp`).
- The view passes the active project's canonical path.

### Slice 3 — persistence

- `crates/app/src/persistence.rs` stores `{projects: [canonical], active}` in
  the app-data dir; a missing path on launch is dropped with a notice.
- Never write into the project directory.

## 4. Protocol

```rust
pub struct ProjectId(pub String);

pub struct Project {
    pub id: ProjectId, pub canonical: String, pub vcs: Option<String>,
    pub name: Option<String>, pub icon: Option<String>,
    pub commands: Option<ProjectCommands>, pub time: Option<ProjectTime>,
    pub sandboxes: Vec<String>,
}
```

Fixture: a project with only `id` and `canonical` (all optionals missing), and
one with an unknown `vcs` value.

## 5. Core

`ProjectStore` rules:

- The active project survives a list refresh if its id is still present;
  otherwise the first project becomes active and the view shows a notice.
- Sessions are grouped by `projectID`; a session without one falls into
  "Unknown project" rather than disappearing.
- Switching projects preserves per-project drafts and transcript scroll
  offsets (keyed by project + session).

## 6. UI

Project switcher at the top of the sidebar: active project name, canonical
path on hover, a menu to add an existing folder, and per-project session
groups. Adding a folder only records the path; the server creates the project
entry on the first request. Keyboard: Cmd/Ctrl+Shift+P opens the switcher.

## 7. Non-goals

Creating a project from a template, remote projects, sandboxes, worktrees
(M11-02), multi-run (M11-03).

## 8. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| protocol | `project_parses_with_only_required_fields` | optional tolerance |
| core | `project_active_survives_refresh` | stable selection |
| core | `project_sessions_group_by_project` | grouping |
| core | `project_drafts_are_per_project` | no draft bleed |
| engine | `list_sessions_scopes_by_directory` | location query |
| engine | `create_session_pins_directory` | pinned location |
| app | `project_list_round_trips_with_active`, `missing_project_is_dropped_with_a_notice` | persistence |

E2E (`tests/M11-01.rs`, no model gate): two temp dirs, `ListProjects`, add
both, create a session in each, assert `GET /api/session?directory=A` returns
only A's session and vice versa; restart the engine and assert both projects
still list their sessions. Durable assertion: sessions are scoped on the
server, not filtered locally.

## 9. Acceptance

- [x] Two projects are usable at once without cross-listing sessions.
- [x] A non-canonical path is rejected with an actionable message.
- [x] Project list and active project survive an app restart.
- [x] No command relies on a hidden current directory.

## 10. Risks

This node touches every command signature. Land the slices separately and keep
the old single-project behavior working after slice 1 (one project, same
behavior) before widening. `ARCHITECTURE.md` §5 and §6.2 must be updated in the
same commit as slice 2.
