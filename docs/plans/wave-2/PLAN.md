# Plan — Wave 2 execution

Status: draft · 2026-11-12

## Goal

Execute the remaining nodes of wave 2 — worktrees (M11-02), goals (M12-01), and
the terminal dock (M14-03) — then close gate `G2`. M10-03, the fourth wave-2
node, landed during wave 1; wave 3 is only M11-03, which waits on M11-02.

## Context

- [`docs/specs/README.md`](../../specs/README.md) — loop protocol, lanes, waves,
  gates, and the shared-file protocol.
- [`docs/specs/graph.json`](../../specs/graph.json) — the machine-readable graph;
  `python3 scripts/spec-graph.py --ready` is the source of truth for readiness.
- Node specs: [`M11-02`](../../specs/M11-02-worktrees.md),
  [`M12-01`](../../specs/M12-01-goals.md),
  [`M14-03`](../../specs/M14-03-terminal-view.md).
- [`docs/plans/wave-1/PLAN.md`](../wave-1/PLAN.md) and
  [`STATUS.md`](../wave-1/STATUS.md) — wave 1 closed gate `G1` and merged to
  `main` as `297b2eb` (PR #2).
- [`docs/ARCHITECTURE.md`](../../ARCHITECTURE.md) — crate boundaries, event
  model, and testing rules that all three nodes must keep.

## Scope / non-goals

In scope: the three specs as written, including their protocol types, pure
reducers, engine commands/events, views, unit tests, and e2e gates.

Out of scope:

- M10-03 (done) and M11-03 (wave 3, blocked until M11-02 closes).
- Worktree branch merge-back, push, submodules, and worktree-specific config
  (M11-02 non-goals).
- Multi-session goals, goal templates, automatic model routing, and server-side
  goal persistence (M12-01 non-goals).
- Terminal tabs, splits, search, hyperlinks, images, and mouse reporting
  (M14-03 non-goals).

## Design

The three nodes are independent in the dependency graph but share
`crates/views/src/workspace.rs`, `crates/engine/src/lib.rs`, and
`crates/engine/src/supervisor.rs`. Land them one at a time and keep every
shared-file edit minimal and tagged `// spec:<id>`, so the next node rebases
onto a small diff.

### M11-02 — Worktrees

- Protocol: `Worktree { directory, strategy? }`, `CreateWorktreeRequest`,
  `RemoveWorktreeRequest`; `GET`/`POST`/`DELETE /api/worktree`, `POST
  /api/worktree/refresh`, `POST /api/session/{id}/move`.
- Core: `WorktreeStore` per project with `list`/`upsert`/`remove`, plus an
  app-local `created_by_app` registry persisted with the project list. On
  launch, a missing registry path is dropped; an existing path absent from the
  server list renders as an orphan with Remove. The base ref is validated
  locally; `force` is only sent after the user confirms a dirty worktree.
  Verified against the live API: the server creates a **detached** worktree and
  does not create a branch, so the form offers an optional base ref
  (empty means `HEAD`).
- Engine: `ListWorktrees`, `CreateWorktree`, `RemoveWorktree`,
  `RefreshWorktrees`, `MoveSession`; reload on `StreamConnected`, on
  `worktree.*` events, and when the picker opens.
- View: project-header worktree chip and picker (existing worktrees, new form,
  per-row Remove); session list grouped by worktree; removal confirmation shows
  the absolute path and how many sessions will be deleted.

### M12-01 — Goals

- Core: pure `GoalMachine::on_idle(usage) -> GoalAction` enforcing
  `max_continuations` (10), `token_budget` (200k), and `max_wall_clock`
  (30 min); invalid auditor JSON becomes `Block`, a failed turn becomes
  `Error`, and a relaunch marks a running goal `Interrupted` without resuming.
- Engine: a goal is a supervised loop over `ExecutionFinished`; the auditor is
  a separate session created once per goal, given only the objective and the
  last assistant reply, using `goal_auditor_model` or the cheapest model from
  `/api/model`. Commands `StartGoal`, `StopGoal`, `ResumeGoal`; events
  `GoalStarted`, `GoalAudited`, `GoalContinued`, `GoalStopped`.
- Persistence: goal state is app-local (`crates/app/src/goal_store.rs`);
  nothing resumes silently after a relaunch.
- View: goal bar above the transcript with objective, state, `n/10`
  continuations, token spend, Stop, and the stop reason.

### M14-03 — Terminal dock

- The dock owns `PtySession { id, grid: harness_term::Grid, viewport_top,
  follow }`; `PtyOutput` is applied on the UI thread and coalesced into one
  `cx.notify()` per frame. Only visible rows render, as styled text runs.
- Pure key encoding (`terminal_keys.rs`) covers the spec table; IME
  composition never emits keys.
- Resize sends `ResizePty` with a 120 ms debounce and resizes the local grid
  optimistically; scrollback follows only when `follow` is true.
- Closing the dock hides it and keeps the PTY; quitting the app terminates
  every PTY the app created.

## Steps

- [x] M11-02: claim the spec (`Status: in progress`), then add protocol types
      and fixtures, `WorktreeStore` and its tests, engine commands/events and
      the adapter, the picker and grouped session list, then run the node
      gates and the e2e.
- [x] M11-02: close the spec (`Status: done`), record the commit in its
      `STATUS.md`, and update the spec table.
- [ ] M12-01: claim, add `GoalMachine` and its six unit tests before any IO,
      then the engine loop, app-local persistence, goal bar, node gates, and
      the scripted-provider e2e.
- [ ] M12-01: close the spec and update the status table.
- [ ] M14-03: claim, add the pure key-encoding tests and grid batching test,
      then the dock view, resize debounce, detach/attach, node gates, and the
      PTY e2e.
- [ ] M14-03: close the spec and update the status table.
- [ ] G2: run every workspace gate, every e2e gate the wave touched,
      `python3 scripts/spec-graph.py --validate`, and the manual checks below.
- [ ] Close the wave: update this plan's `STATUS.md`, the README status, and
      merge the wave-2 branch through a PR after CI passes.

Recommended landing order: M11-02, then M12-01, then M14-03. M11-02 owns the
largest shared-file diff and unblocks wave 3; M12-01 and M14-03 can move in
parallel once it lands if their shared touches do not overlap.

## Test plan

### Unit tests

Each node runs the test table in its spec. The minimum per node:

```sh
cargo test -p harness-protocol
cargo test -p harness-core
cargo test -p harness-engine
cargo test -p harness-views
```

### E2E gates

| Node | Test | Gate |
| --- | --- | --- |
| M11-02 | `tests/M11-02.rs` | none; temp git repo, verifies disk and `git worktree list` |
| M12-01 | `tests/M12-01.rs` | `RUST_HARNESS_MOCK_SCRIPTED_PROVIDER` |
| M14-03 | `tests/M14-03.rs` | `RUST_HARNESS_PTY_E2E` |

Every e2e asserts a durable effect (filesystem/git, server sessions, OS
process) and a clean shutdown, and skips with a clear message when its
environment is unavailable.

### Workspace gates after every node

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

### Manual checks

- Create a worktree, open a session in it, restart the app, confirm the
  worktree is still listed; remove it and confirm the path is gone.
- Run a goal with a mock provider; confirm each cap stops the loop visibly and
  that a relaunch shows `Interrupted`, not a resumed turn.
- Run `yes | head -c 10000000` in the terminal dock and confirm the window
  stays responsive; quit and confirm `pgrep` finds no PTY child.

## Risks

- **Shared-file rebases.** All three nodes touch `workspace.rs`,
  `engine/src/lib.rs`, and `supervisor.rs`. Land sequentially, keep
  `// spec:<id>` edits minimal, and rerun the workspace suite after each merge
  of a node.
- **Destructive worktree removal.** The confirmation must show the absolute
  path and session count; the e2e must use its own temp repository and never a
  developer's checkout.
- **Runaway goal cost.** Caps live in `harness-core` (pure and tested), not the
  UI; the auditor defaults to the cheapest model and sees only the objective
  and the last reply.
- **Terminal rendering and orphan processes.** Render visible rows only,
  batch output at ≤ 16 ms / 64 KB, and terminate PTYs on dock close and app
  quit. The 10 MB test measures that the window does not freeze.
- **API drift.** Verify the worktree and session-move endpoints against the
  live `/openapi.json` for the pinned OpenCode version before implementing the
  adapter, and keep version-specific routing in one engine adapter.
