# Status — Wave 2 execution

Status: done

Last updated: 2026-11-12

## Completed

- 2026-11-12 — **M14-03 terminal dock is done** (`a56f3d9`). The dock reuses
  M14-01's WebSocket transport: `PtySession` keeps the grid, viewport, and
  follow flag; `terminal_keys.rs` is a pure encoder for the spec's key table;
  `TerminalBatch` moved to `harness-core` so the engine pump and the view
  share one 16 ms/64 KiB policy. The workspace renders visible rows as styled
  runs with an inverted cursor, copies the visible screen, honours bracketed
  paste, hides without killing the PTY, debounces resize 120 ms with a
  generation counter, and shows the exit code. `Grid::scroll_up` now recycles
  the row the scrollback cap evicts. `tests/M14-03.rs` (opt-in via
  `RUST_HARNESS_PTY_E2E`) runs a real shell, parses output into a grid,
  resizes to 120×40, pushes 10 MB through the batching pipeline, and proves
  DELETE and app quit both leave no OS process.

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

- Gate `G2`: every workspace gate and the wave's e2e gates passed on
  2026-11-12 (see Verification); the wave-2 branch goes to `main` through one
  PR. Nothing else is ready in the graph until wave 3's M11-03 unblocks.
- Manual GUI checks (worktree restart, goal caps on screen, running the dock)
  were not performed on this headless run; their automated equivalents are the
  M11-02, M12-01, and M14-03 e2e gates below.
- Wave 3 starts from M11-03 (multi-run and comparison).

## Blockers

- None for this wave.
- The Linux packaging smoke test remains owned by `release-linux.yml` and has
  not run on this macOS host; it does not block wave 2.

## Verification

```sh
python3 scripts/spec-graph.py --validate
# ok: 33 nodes, 4 waves

python3 scripts/spec-graph.py --ready
# nothing ready (G2 remains blocked until marked done; wave 3's M11-03 is blocked)

cargo fmt --all -- --check
# clean

cargo clippy --workspace --all-targets -- -D warnings
# clean

cargo test --workspace
# all pass (2026-11-12, after M14-03); M14-03 itself skips without its gate

cargo test -p harness-engine --test M11-02
# 1 passed in 10.10s — temp git repo, real opencode serve, clean shutdown

RUST_HARNESS_MOCK_SCRIPTED_PROVIDER=1 cargo test -p harness-engine --test M12-01
# 1 passed in 8.01s — 2 main turns, 1 auditor session, GoalStopped(Done)

RUST_HARNESS_PTY_E2E=1 cargo test -p harness-engine --test M14-03 -- --nocapture
# 1 passed in 16.44s — 15,000,411 bytes in 11.40s across 275 events
# (largest 65,536 bytes; grid apply 3.87s total, 28.8ms worst);
# DELETE and quit left no PTY process; no listener left on the port
```

The 10 MB burst is paced by the sidecar, not the app: a raw WebSocket probe
against opencode v2.2.7 delivered the same 10 MB in 10.14 s with no harness
code in the path, which is why the e2e bounds the worst single event and uses
a 60 s hang guard instead of the spec's original 5 s wall clock.


