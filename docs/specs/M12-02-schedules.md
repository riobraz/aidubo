# M12-02 — Scheduled prompts

| | |
| --- | --- |
| Status | done |
| Wave | 1 |
| Size | M |
| Depends on | T0 |
| Parallel with | M9-*, M12-03, M13-* |
| Owns | `crates/protocol/src/schedule.rs`, `crates/core/src/schedules.rs`, `crates/engine/src/features/schedules.rs`, `crates/views/src/schedules.rs` |
| Shared touch | `protocol/src/lib.rs`, `engine/src/{lib,supervisor}.rs`, `views/src/workspace.rs` |

## 1. Goal

The user can save a prompt to run later or on a recurring cadence in a
session. Schedules are a view over the server's schedules: the sidecar owns
the clock, the app only creates, pauses, resumes, and deletes them. The UI says
plainly that schedules do not fire while the sidecar is stopped.

## 2. Verified contract

| Endpoint | Shape |
| --- | --- |
| `GET /api/experimental/schedule` | query `sessionID` → `{data: [Schedule.Info]}` |
| `POST /api/experimental/schedule` | body `Schedule.CreatePayload` → `{data: Schedule.Info}` |
| `DELETE /api/experimental/schedule/{scheduleID}` | 204 |
| `POST /api/experimental/schedule/{scheduleID}/pause` | 204 |
| `POST /api/experimental/schedule/{scheduleID}/resume` | 204 |

`Schedule.CreatePayload` = `{sessionID, name, when, if, until, ends, prompt}`.
`when` is one of `{in: Duration}`, `{at: string}`, `{every: Duration}`,
`{cron: string}`, each with optional `tz`. `Duration` is a string such as
`"90 seconds"`. `if`/`until` are `{run: string, is?/matches?/changes?}` or
`{code: string}`. `ends` is `{after: Duration} | {at: string} | {times: int}`.
`Schedule.Info` adds `status` (`active|paused|done`), `nextDueAt`, `endsAt`,
`remainingRuns`, `runCount`, `lastFiredAt`, `lastOutcome`
(`fired|late|checked|expired|skipped|disabled|failed`), `lastOutput`.
Errors: 403 `Forbidden`, 404 `SessionNotFound`.

## 3. Protocol

```rust
pub enum ScheduleWhen { In { in_: String, tz: Option<String> },
                        At { at: String, tz: Option<String> },
                        Every { every: String, tz: Option<String> },
                        Cron { cron: String, tz: Option<String> } }

pub enum ScheduleStatus { Active, Paused, Done, Unknown(String) }
pub enum ScheduleOutcome { Fired, Late, Checked, Expired, Skipped, Disabled, Failed, Unknown(String) }
```

`in` is a Rust keyword; serialize with `#[serde(rename = "in")]` on the field
and keep the variant name `In`. Fixture: each `when` form, a schedule with
`lastOutput: ""`, and one with an unknown `status`.

## 4. Engine

Commands `ListSchedules{session_id}`, `CreateSchedule{request}`,
`PauseSchedule{id}`, `ResumeSchedule{id}`, `DeleteSchedule{id}`.
Events `SchedulesLoaded{session_id, schedules}`, `ScheduleCreated{schedule}`,
`ScheduleChanged{schedule}`, `ScheduleDeleted{id}`.

The engine never runs a clock. After create/pause/resume it reloads the list
so `nextDueAt` comes from the server.

Resnapshot: reload on `StreamConnected` and when the schedules panel opens.

## 5. Core

`ScheduleStore` keyed by id, ordered by `nextDueAt` then `created`.

Rules:

- A snapshot replaces; a create upserts.
- `status == done` rows stay visible for one session lifetime, then are
  filtered from the default view (a "Show finished" toggle reveals them).
- `lastOutcome == failed` renders the reason from `lastOutput` (plain text,
  truncated to 400 characters).
- Duration strings are validated locally with a small grammar
  (`<number> <second|minute|hour|day>s?`) before sending; the server remains
  authoritative for cron.

## 6. UI

Per-session "Schedules" panel: name, cadence in words, next run in relative
time, status badge, Pause/Resume, Delete. "New schedule" form: prompt, when
(relative, at, every, cron), optional end condition, and a warning banner:
"Schedules only run while Rust Harness is open."

## 7. Non-goals

Goals-as-schedules, shell conditions (`if.run`) in the UI, code conditions
(`if.code`), loop files, OS-level scheduling.

## 8. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| protocol | `schedule_parses_every_when_form` | four variants |
| protocol | `schedule_unknown_status_is_kept` | forward compatibility |
| core | `schedule_snapshot_replaces_and_sorts` | ordering |
| core | `schedule_duration_validation` | local grammar |
| core | `schedule_failed_outcome_is_truncated` | safe text |
| engine | `schedule_create_body_shape` | `in` serialization |

E2E (`tests/M12-02.rs`, no model gate): create a schedule with
`when: {in: "2 seconds"}` on a session, poll `ListSchedules` for up to 20 s and
assert `runCount > 0` or `lastFiredAt` is set; pause and assert `status ==
paused` and `runCount` stable over 3 s; resume; delete and assert the id is
gone. Durable assertion: the server's `Schedule.Info`, not a local timer.

## 9. Acceptance

- [ ] A relative schedule fires and records an outcome.
- [ ] Pause stops firing; resume restarts the cadence.
- [ ] Delete removes it from the server.
- [ ] The UI states the sidecar dependency.

## 10. Risks

The endpoint is experimental and its `when`/`if` schema is rich. Ship the four
`when` forms and no conditions; adding `if` later is additive.
