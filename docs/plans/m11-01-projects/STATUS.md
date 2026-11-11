# M11-01 — Status

Status: in progress
Last updated: 2026-11-11

## Completed

- Slice 1 (protocol, core, engine, sidebar switcher): implementation complete,
  2026-11-11. Tests `project_parses_with_only_required_fields`,
  `project_active_survives_refresh`, `project_sessions_group_by_project`,
  `project_drafts_are_per_project`, `list_sessions_scopes_by_directory`,
  `create_session_pins_directory`, and `tests/M11-01.rs` pass.
- Verified the live contract while implementing: `GET /api/project` returns a
  bare array; the server materializes a project entry on the first
  session request for a directory.
- Slice 2 (explicit per-command directory): implementation complete,
  2026-11-11. `EngineCommand` is now `{ directory, kind }`; every `Engine`
  method names its directory explicitly and no engine state holds a current
  project. The supervisor canonicalizes once, rejects relative, missing, and
  non-canonical paths with `CommandFailed`, and `ARCHITECTURE.md` §5/§6.2
  describe the v2.2.7 routing.

## In progress / next

- Slice 3: app-data persistence.

## Blockers

None.

## Verification

Slice 1:

```text
cargo test -p harness-protocol -p harness-core -p harness-engine --lib  ok
cargo test -p harness-views                                          ok (13)
cargo test -p harness-engine --test M11-01                            ok (real opencode v2.2.7)
cargo test --workspace                                               ok
cargo fmt --all -- --check                                           ok
cargo clippy --workspace --all-targets -- -D warnings                ok
```

Slice 2:

```text
cargo test --workspace                                               ok
cargo fmt --all -- --check                                           ok
cargo clippy --workspace --all-targets -- -D warnings                ok
```

The canonicalization rule is covered by the supervisor unit test
`command_directories_must_be_canonical_and_exist` and by the symlink case in
`tests/M11-01.rs`.
