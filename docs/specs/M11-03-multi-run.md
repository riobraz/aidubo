# M11-03 — Multi-run and comparison

| | |
| --- | --- |
| Status | done |
| Wave | 3 |
| Size | L |
| Depends on | M11-02 |
| Parallel with | — |
| Owns | `crates/core/src/runs.rs`, `crates/engine/src/features/runs.rs`, `crates/views/src/run_compare.rs` |
| Shared touch | `engine/src/{lib,supervisor}.rs`, `views/src/workspace.rs` |

## 1. Goal

One prompt, N models, N isolated worktrees. Each run streams independently; a
failure in one does not cancel the others. The user compares the transcripts
side by side, keeps one result, and discards the rest with their worktrees.

## 2. Contract

No new server endpoint. A run is composed from:

- `POST /api/worktree` (M11-02) — one worktree per run.
- `POST /api/session` with `location.directory` — one session per run.
- `POST /api/session/{id}/model` — the run's model.
- `POST /api/session/{id}/prompt` — the shared prompt.
- `DELETE /api/worktree` — discard.

## 3. Core model

```rust
pub struct RunGroup {
    pub id: String,
    pub prompt: String,
    pub created: i64,
    pub runs: Vec<Run>,
    pub kept: Option<String>,
}

pub struct Run {
    pub id: String,
    pub model: ModelRef,
    pub session_id: Option<String>,
    pub worktree: Option<String>,
    pub state: RunState,
}

pub enum RunState { Pending, Starting, Running, Done, Failed { message: String }, Discarded }
```

Rules:

- The group is `app-local`; it never claims server state it cannot verify.
- A run that fails before its session exists is `Failed` with a message and
  no worktree; cleanup still runs for the ones that exist.
- `kept` is exclusive; keeping one run does not auto-discard the others, but
  the UI offers "Discard the rest".
- Deleting a group with `kept = Some` keeps that run's worktree and session.
- A relaunch with a group in `Running` marks it `Interrupted`, never resumes
  silently.

## 4. Engine

Commands `StartRunGroup{group}`, `DiscardRun{group_id, run_id, cleanup}`,
`KeepRun{group_id, run_id}`, `DiscardGroup{group_id, runs: Vec<RunDiscard>}`.
The discard commands carry the resources the core model says each run still
holds; the runtime merges them with its own handles so an event that arrives
late cannot leak a checkout. `RunGroups::discard_group` returns
`Vec<(run_id, RunCleanup)>` so each deleted run can be reported.

Events `RunGroupStarted{group}`, `RunStateChanged{group_id, run_id, state,
session_id, worktree}`, `RunGroupFinished{group_id}`. The extra session and
worktree fields are an additive extension: the view renders and cleans up
without a second round trip, and a client that only reads `state` is
unaffected.

The supervisor executes the steps sequentially per run and in parallel across
runs, capped at 5 concurrent runs. Every step is idempotent: a worktree that
already exists is reused, a session that exists is not recreated.

Resnapshot: on `StreamConnected`, re-check each run's session existence and
mark missing ones `Failed`.

## 5. UI

A "Compare" workspace: one column per run (2–3 visible; more scrolls), each
with model name, state badge, transcript tail, and Stop. Footer actions:
**Keep this** (per column), **Discard the rest**, **Open in sidebar**.
Narrow windows fall back to tabs. The sidebar's **Compare** button reopens the
newest group; **New comparison** returns to the form. Groups persist per
project in the app-data directory and a live run is restored as interrupted.

## 6. Non-goals

Automatic scoring of runs, merging two runs, more than five concurrent runs,
cross-project groups.

## 7. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| core | `run_group_partial_failure_keeps_others` | isolation |
| core | `run_group_keep_is_exclusive` | single winner |
| core | `run_group_relaunch_marks_interrupted` | no silent resume |
| core | `run_group_discard_cleans_existing_only` | no error on missing |
| engine | `run_group_caps_concurrency_at_five` | cap |
| engine | `worktree_names_are_path_safe_and_bounded` | safe checkout name |
| engine | `existing_worktree_errors_are_recognized` | idempotent reuse |
| views | `run_compare_falls_back_to_tabs_when_narrow` | responsive layout |
| views | `run_compare_state_labels_and_tones_are_stable` | stable badges |
| views | `run_compare_tail_keeps_the_end_and_respects_boundaries` | bounded tail |
| views | `run_compare_streaming_tail_is_bounded` | bounded storage |
| views | `run_compare_group_requires_distinct_models_and_a_prompt` | form rules |
| app | `run_groups_round_trip_per_project` | app-local persistence |
| app | `invalid_files_restore_as_empty` | tolerant restore |

E2E (`tests/M11-03.rs`, gate `RUST_HARNESS_MOCK_SCRIPTED_PROVIDER`): three runs
with the same prompt, wait for all `Done`, assert three sessions and three
worktrees on disk; discard one run and assert its worktree is gone while the
other two still exist; then discard the group and assert no run worktree
remains and the root session survives. Durable assertion: filesystem +
`GET /api/worktree`, plus a clean shutdown on the sidecar port.

## 8. Acceptance

- [x] Three runs start from one prompt and finish independently.
- [x] One failure leaves the others running and usable.
- [x] Discard removes only the chosen worktree and its session.
- [x] No orphan worktree after discarding a whole group.

## 9. Risks

Cost and disk: five runs multiply both. The confirmation shows the model and
the worktree path per run; the cap is enforced in the engine, not the UI.
