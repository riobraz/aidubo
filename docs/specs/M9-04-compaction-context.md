# M9-04 — Compaction and the context meter

| | |
| --- | --- |
| Status | ready |
| Wave | 1 |
| Size | M |
| Depends on | T0 |
| Parallel with | M9-01, M9-02, M9-03, M9-05, M9-06 |
| Owns | `crates/protocol/src/context.rs`, `crates/core/src/context.rs`, `crates/engine/src/features/context.rs`, `crates/views/src/context_meter.rs` |
| Shared touch | `protocol/src/lib.rs`, `engine/src/{lib,supervisor}.rs`, `views/src/workspace.rs` |

## 1. Goal

The user can compact a long session without losing it, and can see how full the
context is (tokens and share of the window) in the status bar. The meter is
computed from server usage, never estimated from markdown length.

## 2. Verified contract

| Endpoint | Shape |
| --- | --- |
| `POST /api/session/{sessionID}/compact` | body `{id: Message.ID \| null, delivery: "steer" \| "queue" \| null}` → `{data: Session.Inbox.Compaction}` |
| `GET /api/session/{sessionID}/context` | `{data: [Session.Message.Info]}` |

Events: `session.compaction.started`, `session.compaction.delta`,
`session.compaction.ended`, `session.compaction.failed`, `session.context`,
`session.usage.updated`. `Session.Info.tokens` is
`{input, output, reasoning, cache{read, write}}`; `cost` is a number in USD.

## 3. Protocol

```rust
pub struct CompactRequest { pub id: Option<String>, pub delivery: Option<InboxDelivery> }

pub enum CompactionState { Idle, Running { started: i64 }, Failed { message: String } }

pub struct ContextUsage {
    pub input: u64, pub output: u64, pub reasoning: u64,
    pub cache_read: u64, pub cache_write: u64, pub cost_usd: f64,
    pub window: Option<u64>,
}
```

`ContextUsage::share()` returns `Option<f64>`; `None` when `window` is unknown.
Never render `0%` for unknown.

## 4. Engine

Commands `Compact{session_id, request}`, `LoadContext{session_id}`. Events
`CompactionStarted{session_id}`, `CompactionDelta{session_id, text}`,
`CompactionFinished{session_id}`, `CompactionFailed{session_id, message}`,
`ContextLoaded{session_id, messages}`, `UsageUpdated{session_id, usage}`.

`session.usage.updated` carries the usage payload; map it once and reuse it for
both the session list and the open transcript.

Resnapshot: on `StreamConnected`, reload context and the session snapshot; a
stale meter must be cleared, not frozen.

## 5. Core

`ContextStore`: per-session `ContextUsage`, a compaction state, and the
`window` learned from the model catalog (`ModelInfo` may carry it; otherwise
`None`). Rules:

- A session snapshot replaces usage; usage events upsert.
- `compaction.started` sets `Running`; `ended`/`failed` clear it.
- Cost accumulates only from the server's totals, never by summing deltas.
- After a reconnect with no new usage, mark `stale: true` and render the last
  value with a dimmed style; clear on the next update.

## 6. UI

Status bar shows `12.4k / 200k · 6% · $0.42`; clicking opens a popover with
input/output/reasoning/cache and the compaction action. While compacting, the
composer shows "Compacting…" and Stop stays available. Keyboard:
Cmd/Ctrl+Shift+K compacts the open session after a confirmation.

## 7. Non-goals

Auto-compaction heuristics, context editing, per-message token estimates.

## 8. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| protocol | `context_usage_parses_nested_cache` | cache read/write |
| protocol | `compact_body_omits_null_fields` | body shape |
| core | `usage_unknown_window_is_not_zero` | `share()` is `None` |
| core | `usage_snapshot_replaces_events` | snapshot wins |
| core | `compaction_failure_clears_running` | state machine |
| core | `usage_marked_stale_after_reconnect` | stale flag |

E2E (`tests/M9-04.rs`, gate `RUST_HARNESS_MOCK_SCRIPTED_PROVIDER`): run one
turn, assert the session snapshot reports `tokens.input > 0`; compact, assert
`session.compaction.started` and `.ended` arrive and the session still accepts
a prompt afterwards; `GET /context` returns an array. Durable assertion: the
session remains usable and its message list still exists.

## 9. Acceptance

- [ ] Meter uses server usage only; no local token estimation.
- [ ] Unknown window renders "unknown", not 0%.
- [ ] Compaction failure is visible and does not wedge the composer.
- [ ] Reconnect clears stale values on the next update.

## 10. Risks

Compaction spends model tokens and can take minutes. The confirmation must say
so; the e2e test uses the scripted mock, never a real provider.
