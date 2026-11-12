# Plan — Wave 3 execution

Status: in progress · 2026-11-12

## Goal

Ship M11-03 — one prompt, N models, N isolated worktrees, compared side by
side, with one keeper and discard for the rest — then close gate `G3`. Wave 2
is closed and merged to `main` as `5daf7c5` (PR #5), so M11-03 is the only node
left in the graph.

## Context

- [`docs/specs/M11-03-multi-run.md`](../../specs/M11-03-multi-run.md) — the
  node contract: model, rules, commands, events, UI, tests, acceptance.
- [`docs/ARCHITECTURE.md`](../../ARCHITECTURE.md) §4 (crate layout), §5.3
  (event model), §5.4 (protocol types), §7 (concurrency), §8 (app-local
  persistence), §11 (testing).
- [`docs/specs/M11-02-worktrees.md`](../../specs/M11-02-worktrees.md) and
  [`M12-01-goals.md`](../../specs/M12-01-goals.md) — the worktree commands and
  the supervised-task pattern this node reuses.
- [`docs/plans/wave-2/PLAN.md`](../wave-2/PLAN.md) — landing conventions and
  the shared-file protocol.

## Scope / non-goals

In scope: the pure run-group model and its four tests, the engine executor and
its concurrency cap, app-local persistence of groups across relaunch, the
compare view, and the three-run e2e.

Out of scope (from the spec): automatic scoring, merging two runs, more than
five concurrent runs, cross-project groups, and any new server endpoint.

## Design

### Core (`crates/core/src/runs.rs`)

- `RunGroup { id, prompt, created, runs: Vec<Run>, kept: Option<String> }`,
  `Run { id, model: ModelRef, session_id, worktree, state }`, and
  `RunState { Pending, Starting, Running, Done, Failed { message },
  Discarded }`.
- `RunGroups` (app-wide, per project) with:
  - `start(group) -> Result<(), String>` rejecting a duplicate run id and more
    than five runs.
  - `set_state(group_id, run_id, state)` — the single mutation path; ignore
    unknown ids so a late event cannot panic.
  - `keep(group_id, run_id)` — exclusive winner, only for a group member.
  - `discard_run(group_id, run_id)` — marks `Discarded` and returns the
    worktree/session that must be cleaned up, if they exist.
  - `discard_group(group_id)` — keeps the `kept` run, returns cleanup targets
    for every other run that has one.
  - `relaunch(groups)` — any `Running`/`Starting` run becomes `Failed` with
    "Interrupted by relaunch"; the group is never resumed.
- All cleanup decisions come from the model, so the engine only executes them.

### Engine (`crates/engine/src/features/runs.rs`)

- A supervised `RunGroupRuntime` fed by its own unbounded command channel, like
  the goal runtime, so the engine loop never blocks on HTTP.
- `StartRunGroup { group }`, `DiscardRun { group_id, run_id }`,
  `KeepRun { group_id, run_id }`, `DiscardGroup { group_id }`.
- Per run, sequentially: `POST /api/worktree` (name derived from the group and
  run; an "already exists" reply is treated as reuse), `POST /api/session`
  with `location.directory` set to the worktree, `POST
  /api/session/{id}/model`, `POST /api/session/{id}/prompt`, then wait for that
  session's `ExecutionFinished` from the stream.
- Runs execute concurrently behind a `tokio::sync::Semaphore` of five; a
  failed run emits `Failed` and never cancels the others.
- `KeepRun` records the winner locally; `DiscardRun`/`DiscardGroup` delete the
  run's session and worktree (force = true, because the run's checkout is
  throwaway) and emit the state changes.
- Events: `RunGroupStarted { group }`, `RunStateChanged { group_id, run_id,
  state }`, `RunGroupFinished { group_id }`.
- Resnapshot: on `StreamConnected` the supervisor asks the runtime to verify
  each live run's session; a missing session marks the run `Failed`.

### App persistence (`crates/app/src/run_store.rs`)

- Mirror `goal_store.rs`: one JSON file under app data, written on every
  meaningful change, read at startup and handed to the view; a running group
  is restored as interrupted.

### View (`crates/views/src/run_compare.rs` + `workspace.rs`)

- A compare pane with one column per run: model, state badge, transcript tail,
  Stop, and **Keep this**. Footer: **Discard the rest** and **Open in sidebar**
  (selects the kept run's session and switches back to the conversation).
- Two or three columns at normal width; more scroll horizontally; narrow
  windows fall back to tabs.
- The pane never invents state: it renders `RunGroups` and dispatches engine
  commands.

## Steps

- [ ] M11-03: claim the spec and open this plan.
- [ ] Add `harness_core::runs` and the four core tests.
- [ ] Add protocol types if the spec needs them and the engine executor with
      the concurrency-cap test.
- [ ] Add app-local group persistence and relaunch interruption.
- [ ] Add the compare view and wire it into the workspace.
- [ ] Add `crates/engine/tests/M11-03.rs` and run the node gates.
- [ ] Close the spec, update the spec table and graph.
- [ ] G3: workspace gates, the wave's e2e gates, `--validate`, manual checks.
- [ ] Close the wave: `STATUS.md`, README, PR and merge.

## Test plan

```sh
cargo test -p harness-core runs
cargo test -p harness-views
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUST_HARNESS_MOCK_SCRIPTED_PROVIDER=1 cargo test -p harness-engine --test M11-03
```

The e2e creates three runs from one prompt in a temp git repository, waits
for three `Done` states, and asserts three sessions plus three worktrees on
disk. It then discards one run and asserts that worktree is gone from disk and
from `GET /api/worktree` while the other two remain, and finally discards the
group and asserts no worktree is left. Clean shutdown is asserted.

## Risks

- **Cost and disk.** Five runs multiply model cost and checkout size; the cap
  is enforced in the engine and the confirmation names each model and path.
- **Partial failure.** Worktree creation can succeed before session creation
  fails; cleanup runs for whatever exists and the failure message stays
  user-safe.
- **Server idempotency.** A reused worktree directory must be recognized from
  the server's error reply, never assumed.
- **GPUI columns.** The compare pane touches `workspace.rs`; keep the diff
  behind one `// spec:M11-03` render function to limit rebase pain.
