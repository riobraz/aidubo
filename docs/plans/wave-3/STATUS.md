# Status — Wave 3 execution

Status: in progress (implementation and `G3` done; PR and merge pending)

Last updated: 2026-11-12

## Completed

- 2026-11-12 — Plan opened. Wave 2 is closed and merged to `main` as
  `5daf7c5` (PR #5): M14-03 shipped the terminal dock and gate `G2` passed
  every local gate and e2e. `python3 scripts/spec-graph.py --validate` reports
  `ok: 33 nodes, 4 waves`; M11-03 was re-marked `in-progress` from `blocked`
  because its only dependency, M11-02, is done.
- 2026-11-12 — M11-03 core model (`crates/core/src/runs.rs`) with the four spec
  tests; `discard_group` now returns `Vec<(run_id, RunCleanup)>` so every
  removed run is reported.
- 2026-11-12 — Engine executor (`crates/engine/src/features/runs.rs`): a
  supervised `RunGroupRuntime`, one `RunTask` per model, a five-permit
  semaphore, sequential steps per run and parallel runs, worktree reuse on
  "already exists", `ExecutionFinished` correlation by session id, and cleanup
  for failed or cancelled runs. Commands `StartRunGroup`, `DiscardRun`,
  `KeepRun`, `DiscardGroup` are wired through `lib.rs` and the supervisor;
  events `RunGroupStarted`, `RunStateChanged` (with `session_id` and
  `worktree`), and `RunGroupFinished` reach the view.
- 2026-11-12 — App-local persistence (`crates/app/src/run_store.rs`): one JSON
  file keyed by canonical project directory, restored for the startup project,
  live runs restored as `Interrupted`, empty stores remove the file. Wired
  through `crates/app/src/main.rs` with a save thread.
- 2026-11-12 — Compare view (`crates/views/src/run_compare.rs` and
  `workspace.rs`): pure layout/label/tail helpers with five tests; panes for
  columns and narrow-window tabs; per-run model, badge, worktree, tail, Stop,
  Keep this; footer with Discard the rest and Open in sidebar; sidebar Compare
  button; Escape closes the pane; groups persist on every meaningful change and
  are saved before a project switch.
- 2026-11-12 — `crates/engine/tests/M11-03.rs`: three runs from one prompt in a
  temp git repository, all `Done`, three sessions and three worktrees on disk,
  discard one removes only its worktree and session, discarding the group
  removes the rest while the root session survives, clean shutdown asserted.
- 2026-11-12 — `G3` passed: fmt, clippy, the workspace tests, the M11-03 e2e
  with `RUST_HARNESS_MOCK_SCRIPTED_PROVIDER=1`, and `--validate`.
- 2026-11-12 — Spec closed (`status: done`, acceptance checked) and the spec
  table and graph updated; `G3` is `done` in `graph.json`.

## In progress / next

- Commit the wave, open a PR, and squash-merge to `main` without waiting for
  Actions (they are not running). Then close this plan and update the README.

## Blockers

- None.

## Verification

```sh
cargo fmt --all -- --check
# FMT_OK

cargo clippy --workspace --all-targets -- -D warnings
# Finished `dev` profile ... (no warnings)

cargo test --workspace
# 39 suites, all `test result: ok`; 0 failed

RUST_HARNESS_MOCK_SCRIPTED_PROVIDER=1 cargo test -p harness-engine --test M11-03
# test three_models_run_in_parallel_and_cleans_up_each_run ... ok
# test result: ok. 1 passed; 0 failed

python3 scripts/spec-graph.py --validate
# ok: 33 nodes, 4 waves
```
