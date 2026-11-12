# M12-01 — Goals: keep working until verified

| | |
| --- | --- |
| Status | done |
| Wave | 2 |
| Size | L |
| Depends on | M9-06, T0 |
| Parallel with | M10-03, M11-02, M14-03 |
| Owns | `crates/core/src/goal.rs`, `crates/engine/src/features/goal.rs`, `crates/app/src/goal_store.rs` |
| Shared touch | `engine/src/{lib,supervisor}.rs`, `views/src/workspace.rs`, `protocol/src/lib.rs` |

## 1. Goal

A session can be started "as a goal": when a turn goes idle, an auditor
decides whether the objective is met. If not, the session continues with the
auditor's next step. The loop stops on success, a blocker, a continuation cap,
or a token budget. Nothing resumes silently after a relaunch.

There is no server goal endpoint. This is an app-side supervised loop.

## 2. Design

```
main session  --prompt--> turn --> idle
                                   |
                                   v
                        auditor session (cheap model)
                        input: objective + last reply + budget
                        output: {"decision":"continue|finish|block",
                                 "reason":"...", "next":"..."}
                                   |
              continue ------------+--------> prompt main session
              finish/block -----------------> stop
```

Rules:

- The auditor runs in a **separate** session, created once per goal, with a
  model chosen from settings (`goal_auditor_model`), defaulting to the cheapest
  available model from `/api/model`.
- The auditor sees the objective and the latest assistant reply only. Never
  the whole transcript, never tool output.
- The answer must parse as strict JSON; anything else is `block` with reason
  "auditor returned invalid JSON".
- Caps are mandatory: `max_continuations` (default 10), `token_budget`
  (default 200k), `max_wall_clock` (default 30 min). Any cap hit stops the
  goal with `Stopped::Cap`.
- A turn failure stops the goal (`Stopped::Error`); it never retries blindly.
- Goal state is persisted app-locally. On launch, a goal that was running is
  marked `Interrupted` and offers Resume; it never auto-resumes.

## 3. Core

```rust
pub struct Goal { pub id: String, pub session_id: String, pub objective: String,
                  pub state: GoalState, pub continuations: u32, pub tokens_used: u64,
                  pub started_at: i64, pub auditor_session_id: Option<String> }

pub enum GoalState { Running, Stopped(Stopped), Interrupted }
pub enum Stopped { Done { reason: String }, Blocked { reason: String },
                   Cap { which: CapKind }, Error { message: String }, User }
pub enum AuditDecision { Continue { next: String }, Finish { reason: String },
                         Block { reason: String } }
```

`GoalMachine::on_idle(&mut self, usage) -> GoalAction` is pure and returns
`AskAuditor`, `Prompt { text }`, or `Stop(Stopped)`. It enforces every cap.

## 4. Engine

Commands `StartGoal{session_id, objective, limits}`, `StopGoal{goal_id}`,
`ResumeGoal{goal, limits}`. Events `GoalStarted`, `GoalAudited{decision}`,
`GoalContinued{goal}`, `GoalStopped{goal, reason}`.

The supervisor watches `ExecutionFinished` for the goal's session; it never
polls.

Implementation notes:

- `ResumeGoal` carries the restored `Goal` plus its limits instead of a bare
  id, because a relaunch restores the record from disk and the runtime has no
  other source for the objective or counters.
- Every goal event carries the full `Goal` snapshot so the view never has to
  reconstruct state from deltas.
- The auditor model comes from `RUST_HARNESS_GOAL_AUDITOR_MODEL`
  (`provider/model`), falling back to the cheapest enabled model in
  `/api/model` by input cost. `harness-protocol`'s `ModelInfo` gained a
  minimal `cost` list for this.
- A separate unbounded channel forwards only `ExecutionFinished` and
  `UsageUpdated` to the goal runtime. Replayed events are deduped by
  `event_id` before any turn counts.
- `GoalMachine` is authoritative for caps: it checks continuations, then
  tokens, then wall clock before every audit.

## 5. UI

A goal bar above the transcript: objective, state, `3/10` continuations, token
spend, and Stop. When stopped, it shows the auditor's reason. Starting a goal
asks for the objective and shows the caps with defaults.

## 6. Non-goals

Multi-session goals, goal templates, automatic model routing, server-side
persistence.

## 7. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| core | `goal_continues_until_finish` | happy path |
| core | `goal_stops_at_continuation_cap` | cap |
| core | `goal_stops_at_token_budget` | cap |
| core | `goal_stops_at_wall_clock` | cap |
| core | `goal_invalid_audit_json_blocks` | safe default |
| core | `goal_turn_failure_stops` | no blind retry |
| core | `goal_relaunch_is_interrupted` | no silent resume |
| core | `goal_resume_restarts_the_wall_clock` | resume semantics |
| core | `audit_parse_accepts_the_three_decisions` | strict JSON |
| core | `last_reply_ignores_tool_output` | data minimisation |
| engine | `auditor_prompt_contains_only_objective_and_reply` | data minimisation |
| engine | `goal_auditor_prefers_the_cheapest_enabled_model` | default routing |
| engine | `goal_auditor_falls_back_to_the_catalog_when_none_are_enabled` | fallback |
| engine | `goal_auditor_token_total_counts_every_bucket` | usage accounting |
| app | `goal_record_round_trips_and_can_be_cleared` | persistence |
| app | `auditor_model_parses_an_optional_provider` | config parsing |

E2E (`tests/M12-01.rs`, gate `RUST_HARNESS_MOCK_SCRIPTED_PROVIDER`): script
`[main "not done", auditor "continue", main "done", auditor "finish"]`; start
a goal, assert exactly two main turns and one auditor session, assert
`GoalStopped(Done)`, and assert the auditor session contains no tool output.
Durable assertion: `GET /api/session` shows the goal session and the auditor
session with the expected message counts.

The mock provider answers OpenCode's session-title completion (system prompt
"You name conversations") with a fixed title and does not consume a scripted
turn; without this every scripted e2e desynchronizes by one turn per session.

## 8. Acceptance

- [x] A goal continues only after an explicit `continue` decision.
- [x] Every cap stops the loop and is visible.
- [x] Invalid auditor output never causes a new turn.
- [x] Relaunch never resumes automatically.

## 9. Risks

Runaway cost is the main risk. Caps are enforced in `harness-core` (pure,
tested), not in the UI. The auditor model is configurable and defaults to the
cheapest available.
