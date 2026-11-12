# Status — Wave 2 execution

Status: in progress

Last updated: 2026-11-12

## Completed

- 2026-11-12 — **M12-01 goals is done** (`28c80a8`). `GoalMachine` in
  `harness-core` enforces three caps before every audit (10 continuations,
  200k tokens, 30 minutes) and is covered by ten pure tests. The engine runs
  the loop as a supervised tokio task fed only by `ExecutionFinished` and
  `UsageUpdated`, deduped by `event_id`; the auditor is a separate session
  created once per goal and sees only the objective and the last reply. The
  app persists the single live goal app-locally (`goal_store.rs`), restores a
  running goal as `Interrupted`, and never resumes it. The workspace renders
  a goal bar with caps, Stop, Resume, and Dismiss. The scripted mock provider
  now answers OpenCode's session-title completion without consuming a turn,
  which made the node e2e deterministic.

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

- M14-03 (terminal dock) is the last wave-2 node and the only ready one.
  Claim it, add the pure key-encoding and grid batching tests before the dock
  view, then resize debounce, detach/attach, node gates, and the PTY e2e.
- After M14-03, close wave 2 with gate `G2`.

## Blockers

- None for this wave.
- The Linux packaging smoke test remains owned by `release-linux.yml` and has
  not run on this macOS host; it does not block wave 2.

## Verification

```sh
python3 scripts/spec-graph.py --validate
# ok: 33 nodes, 4 waves

python3 scripts/spec-graph.py --ready
# 1 ready: M14-03 (wave 2, Terminal dock)

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# all pass (2026-11-12, after M12-01)

cargo test -p harness-engine --test M11-02
# 1 passed — temp git repo, real opencode serve, clean shutdown

RUST_HARNESS_MOCK_SCRIPTED_PROVIDER=1 cargo test -p harness-engine --test M12-01
# 1 passed — 2 main turns, 1 auditor session, GoalStopped(Done), clean shutdown
```

