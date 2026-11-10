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

## In progress / next

- M10-01 panel: Changes list + branch chip, refresh on `filesystem.changed`.
- M10-03 renderer: rows from `flatten`/`visible_range`, unified layout.
- Then M9-02, M9-01, M9-05, M9-03, M9-04, M9-06, then integrations,
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
