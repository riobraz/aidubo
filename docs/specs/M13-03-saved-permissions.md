# M13-03 — Saved permission rules

| | |
| --- | --- |
| Status | done |
| Wave | 1 |
| Size | S |
| Depends on | T0 |
| Parallel with | M13-01, M13-02, M13-04 |
| Owns | `crates/protocol/src/permission_saved.rs`, `crates/core/src/saved_permissions.rs`, `crates/engine/src/features/saved_permissions.rs`, `crates/views/src/permissions_settings.rs` |
| Shared touch | `protocol/src/lib.rs`, `engine/src/{lib,supervisor}.rs`, `views/src/workspace.rs` |

## 1. Goal

"Always allow" answers create saved rules on the server. This node lists them
per project and lets the user revoke one, so the app can explain and undo the
blanket approvals it helped create. The app still never auto-approves a new
request.

## 2. Verified contract

| Endpoint | Shape |
| --- | --- |
| `GET /api/permission/saved` | query `projectID` → `{data: [PermissionSaved.Info]}` |
| `PATCH /api/permission/saved/{id}` | update a rule |
| `DELETE /api/permission/saved/{id}` | 204 |

`PermissionSaved.Info` = `{id, projectID, action, resource, time}`.

## 3. Protocol

```rust
pub struct SavedPermission { pub id: String, pub project_id: String,
                             pub action: String, pub resource: String }
```

Fixture: a rule with `time` missing; one with a very long `resource` (the UI
must truncate, not wrap the layout).

## 4. Engine

Commands `ListSavedPermissions{project_id}`, `DeleteSavedPermission{id}`.
Events `SavedPermissionsLoaded{project_id, rules}`,
`SavedPermissionDeleted{id}`, `SavedPermissionError{id, message}`.

Resnapshot: reload when the panel opens and after a `permission.replied` with
`reply == always` (the server has just created a rule).

## 5. Core

`SavedPermissionStore` keyed by id, sorted by `action` then `resource`.
`remove(id)` is idempotent.

## 6. UI

Settings → Permissions: rows of `action` + `resource` with a Revoke button.
Revoking asks for confirmation and explains: "Future matching requests will ask
again." Empty state: "No saved rules for this project."

## 7. Non-goals

Creating rules from the app, editing patterns (PATCH is exposed but not
required), global (non-project) rules.

## 8. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| protocol | `saved_permission_parses_without_time` | optional field |
| core | `saved_permission_remove_is_idempotent` | replay safety |
| core | `saved_permission_sorted_by_action` | stable order |
| engine | `delete_saved_permission_uses_id` | path shape |

E2E (`tests/M13-03.rs`, gate `RUST_HARNESS_MOCK_PERMISSION_PROVIDER`): run a
turn that asks for permission, reply `always`, wait for the turn to finish,
list saved rules and assert at least one exists; delete it; list again and
assert it is gone. Durable assertion: the server's saved-rule list.

## 9. Acceptance

- [ ] Rules created by "always" appear without a restart.
- [ ] Revoke removes the rule on the server.
- [ ] The app never creates a rule on its own.

## 10. Risks

A stale list after a rule changes elsewhere. The panel reloads on open and
after every `permission.replied`, which is enough for a single-user app.
