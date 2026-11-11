# M11-01 — Multiple projects and location routing

Status: in progress · 2026-11-11

## Goal

Several projects are open in one window. The sidebar groups sessions by
project, creating a session uses the selected project's directory, and every
request carries the right `location` so a project never sees another's
sessions. The app remembers the project list and the last active project.

## Context

- `docs/specs/M11-01-projects.md`
- `docs/ARCHITECTURE.md` §5 (engine commands) and §6.2 (project scope)
- `docs/specs/000-conventions.md`

## Scope / non-goals

In scope: project list and selection, explicit per-command directory, and
app-data persistence. Worktrees, templates, remote projects, sandboxes,
multi-run, and project renaming are out of scope.

## Design

Three slices, each with its own commit:

1. **Project list and selection.** `harness-protocol` gains `Project`,
   `ProjectId`, `ProjectCommands`, and `ProjectTime`; `harness-core` gains
   `ProjectStore`, `SessionGroup`/`group_sessions`, and `ProjectDrafts`.
   `harness-engine` gains `ListProjects`/`SelectProject`; the supervisor keeps
   the active canonical directory in `SessionApi` and scopes
   `ListSessions`/`CreateSession` with it. The sidebar renders a switcher above
   the session list.
2. **Per-command location.** Project-scoped commands carry an explicit
   `directory`. The supervisor canonicalizes once and rejects a non-canonical
   path. The view always passes the active canonical path.
3. **Persistence.** `crates/app/src/persistence.rs` stores canonical paths and
   the active id in the app-data directory; a missing path is dropped with a
   notice. Nothing is written into a project directory.

## Steps

- [x] Protocol project types and fixtures.
- [x] Core project store, grouping, and drafts with the three named tests.
- [x] Engine project listing and selection; four engine unit tests.
- [x] Sidebar project switcher and `Cmd/Ctrl+Shift+P`.
- [x] `tests/M11-01.rs`: two projects, crossed lists, restart durability.
- [ ] Slice 2: explicit directory on every project-scoped command.
- [ ] Slice 2: canonicalization rejection and `ARCHITECTURE.md` §5/§6.2.
- [ ] Slice 3: app-data persistence and missing-path notice.

## Test plan

| Crate | Test |
| --- | --- |
| protocol | `project_parses_with_only_required_fields` |
| core | `project_active_survives_refresh` |
| core | `project_sessions_group_by_project` |
| core | `project_drafts_are_per_project` |
| engine | `list_sessions_scopes_by_directory` |
| engine | `create_session_pins_directory` |
| engine e2e | `tests/M11-01.rs` |

Manual: switch between two folders, create a session in each, restart the app.
The build must also keep single-project behavior working.

## Risks

- The server's project registry is global; the sidebar only shows folders the
  user added, so unrelated projects do not appear.
- `Project.canonical` is a string; a non-UTF-8 path is lossy.
- Every command signature changes in slice 2; land it separately.
