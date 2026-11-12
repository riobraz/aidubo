# Status — Wave 2 execution

Status: in progress

Last updated: 2026-11-12

## Completed

- 2026-11-12 — **M11-02 worktrees is done** (`609a2d5`). The live
  `/openapi.json` was checked before the adapter: `branch` is an existing base
  ref, `from` is the project directory, and the default strategy creates a
  detached worktree instead of a branch. The form and spec now say so.
  Protocol, `WorktreeStore`, engine commands/events, the sidebar panel and
  grouped session list, the persisted app-owned registry, and
  `crates/engine/tests/M11-02.rs` are in place.

- 2026-11-12 — Plan opened (`8c14987`). Wave 1 is closed and merged to `main`
  as `297b2eb` (PR #2). `python3 scripts/spec-graph.py --validate` reports
  `ok: 33 nodes, 4 waves`; `--ready` lists M11-02, M12-01, and M14-03. The
  per-spec headers for those three nodes said `blocked (<dependency>)` even
  though every dependency is done and the central status table already said
  `ready`, so the headers were corrected to `ready`.

## In progress / next

- M12-01 (goals) is next: claim it, add `GoalMachine` and its unit tests before
  any IO, then the engine loop, app-local persistence, the goal bar, node
  gates, and the scripted-provider e2e.
- Then M14-03 (terminal dock). M11-02 is closed, so M11-03 in wave 3 is
  unblocked for a later wave.

## Blockers

- None for this wave.
- The Linux packaging smoke test remains owned by `release-linux.yml` and has
  not run on this macOS host; it does not block wave 2.

## Verification

```sh
python3 scripts/spec-graph.py --validate
# ok: 33 nodes, 4 waves

python3 scripts/spec-graph.py --ready
# M12-01, M14-03 (M11-02 no longer ready; it is done)

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# all pass (2026-11-12)

cargo test -p harness-engine --test M11-02
# 1 passed — temp git repo, real opencode serve, clean shutdown
```

