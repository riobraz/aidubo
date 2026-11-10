# M12-03 — OS notifications

| | |
| --- | --- |
| Status | ready |
| Wave | 1 |
| Size | S |
| Depends on | T0 |
| Parallel with | M9-*, M12-02, M13-* |
| Owns | `crates/app/src/notify.rs`, `crates/core/src/notify.rs` |
| Shared touch | `app/src/main.rs`, `views/src/workspace.rs` |

## 1. Goal

When a turn finishes, fails, asks a permission, or asks a question while the
window is not focused, the OS shows a notification. It never includes message
content by default, never duplicates after a reconnect, and degrades silently
when the user denies permission.

## 2. Design

```rust
pub enum NotifyKind { TurnDone, TurnFailed, PermissionAsked, QuestionAsked }

pub struct Notification { pub id: String, pub kind: NotifyKind,
                          pub session_title: String, pub body: String }

pub trait Notifier { fn notify(&self, n: &Notification) -> Result<(), String>; }
```

Backends:

- `MacNotifier` — runs `/usr/bin/osascript` with `-e` script and the title/body
  as **argv**, never interpolated into the script. No new dependency, no
  entitlement, no background agent.
- `FileNotifier` — appends one JSON line per notification to the path in
  `RUST_HARNESS_NOTIFY_SINK`; used by tests.
- `NullNotifier` — when notifications are disabled or denied.

A native `UNUserNotificationCenter` backend is a later upgrade and needs an
ADR (bundle identifier, entitlement, permission prompt).

## 3. Core

`NotifyPolicy`:

- Dedupe: keep the last 200 notification ids; a repeat is dropped. Ids are
  `"{session}:{kind}:{event_or_message_id}"`, so a reconnect replay is a
  no-op.
- Throttle: at most one notification per session per 2 s; a later event within
  the window replaces the pending one instead of queueing.
- Content: `body` defaults to "Turn finished" / "Turn failed" /
  "Needs permission" / "Has a question". Message text is opt-in per kind and
  truncated to 120 characters when enabled.
- Focus: suppressed when the window is focused **and** the session is the
  active one.

## 4. Wiring

The view pumps `EngineEvent`s and calls `NotifyPolicy::on_event(...)`, which
returns `Option<Notification>`. The app owns the `Notifier` and the policy, so
`harness-views` stays free of IO.

## 5. Settings

Per-kind toggles (four), a global "notifications" switch, and "include message
preview" (off by default). Stored app-locally. Denial of OS permission flips
the global switch off and shows a one-time inline notice.

## 6. Non-goals

Notification actions, inline replies, sound per kind, Windows/Linux backends
(Linux uses `notify-send` behind the same trait when built).

## 7. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| core | `notify_dedupes_replayed_event` | reconnect safety |
| core | `notify_throttles_per_session` | one per 2 s |
| core | `notify_preview_off_by_default` | no content leak |
| core | `notify_suppressed_when_focused` | focus rule |
| app | `osascript_receives_body_as_argv` | no injection |
| app | `file_notifier_appends_json_line` | test sink works |

E2E (`tests/M12-03.rs`, gate `RUST_HARNESS_MOCK_SCRIPTED_PROVIDER` +
`RUST_HARNESS_NOTIFY_SINK`): run one turn, assert the sink file contains
exactly one `TurnDone` line with the session title and no message text; force
a stream reconnect and assert the count stays one. The OS delivery itself is a
manual check recorded in `STATUS.md`.

## 8. Acceptance

- [ ] One notification per turn, never one per delta.
- [ ] Reconnect does not re-notify.
- [ ] No prompt or message content by default.
- [ ] Denied permission does not break the turn or log an error loop.

## 9. Risks

`osascript` is a subprocess with a small startup cost. It runs at most once
per event and never blocks the UI thread.
