# M9-01 — Inbox: queue and steer

| | |
| --- | --- |
| Status | ready |
| Wave | 1 |
| Size | M |
| Depends on | T0 |
| Parallel with | M9-02 … M9-06 |
| Owns | `crates/protocol/src/inbox.rs`, `crates/core/src/inbox.rs`, `crates/engine/src/features/inbox.rs`, `crates/views/src/inbox.rs` |
| Shared touch | `protocol/src/lib.rs`, `engine/src/{lib,supervisor}.rs`, `views/src/workspace.rs` |

## 1. Goal

While a turn is running, the user can type the next message and choose
**queue** (runs after the current turn) or **steer** (delivered into the
running turn). Queued items appear above the composer, can be removed, and can
switch between the two modes before delivery.

## 2. Verified contract (OpenCode v2.2.6, 2026-11-10)

| Endpoint | Shape |
| --- | --- |
| `GET /api/session/{sessionID}/inbox` | `{data: [Session.Inbox.Info]}` |
| `PATCH /api/session/{sessionID}/inbox/{inboxID}` | body `{delivery: "steer" \| "queue"}` |
| `DELETE /api/session/{sessionID}/inbox/{inboxID}` | 204 |
| `POST /api/session/{sessionID}/prompt` | body gains `delivery: "steer" \| "queue" \| null` |

`Session.Inbox.Info` is `User | Synthetic | Compaction | Move`. `User` carries
`id`, `sessionID`, `time`, `type`, `payload{text, files, agents, skills,
metadata}`, `delivery`. Events: `session.inbox.enqueued`,
`session.inbox.delivered`, `session.inbox.cancelled`,
`session.inbox.delivery.changed`.

## 3. Protocol

```rust
pub enum InboxDelivery { Steer, Queue }

pub enum InboxItem {
    User { id: String, session_id: String, created: i64,
           text: String, files: Vec<PromptFile>, delivery: InboxDelivery },
    Synthetic { id: String, created: i64, text: String },
    Compaction { id: String, created: i64, delivery: InboxDelivery },
    Move { id: String, created: i64, directory: String },
    Unknown { id: String, kind: String },
}
```

`Unknown` keeps the id so cancel still works; it renders as "unsupported item".
`PromptRequest` gains `delivery: Option<InboxDelivery>` with
`#[serde(skip_serializing_if = "Option::is_none")]`.

Fixtures: a `User`, a `Compaction`, and a fixture whose `type` is
`"future.thing"` that must parse as `Unknown`.

## 4. Engine

Commands: `ListInbox{session_id}`, `UpdateInboxDelivery{session_id, inbox_id,
delivery}`, `CancelInbox{session_id, inbox_id}`. `SendPrompt` passes the
delivery through.

Events: `InboxLoaded{session_id, items}`, `InboxMutationFailed{session_id,
inbox_id, message}`. Live updates come as `StreamEvent::Inbox*` and are applied
by the reducer, not by refetching per event.

Resnapshot: on `StreamConnected` and when a session is selected, the view
issues `ListInbox`.

## 5. Core

`InboxStore` per session: `replace`, `upsert`, `remove`, `set_delivery`.
Ordering by `created`, server order as tie-break. Rules:

- `enqueued` upserts; a repeat is a no-op.
- `delivered` removes the item and the message appears in the transcript
  through the normal path; do not insert it twice.
- `cancelled` removes; unknown id is a no-op.
- A snapshot replaces the list; live events after the snapshot win.

## 6. UI

A strip between transcript and composer shows "N queued" with an expandable
list: text preview, delivery badge, a queue/steer toggle, and a remove button.
While busy, Enter queues and Cmd/Ctrl+Enter steers; while idle, Enter sends
normally and the strip is hidden. Keyboard: the strip is reachable with
Up from the composer when it is empty.

## 7. Non-goals

Reordering the queue, editing a queued item's text, delivery of attachments
beyond what `PromptRequest` already carries.

## 8. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| protocol | `inbox_item_parses_user_and_compaction` | both variants round-trip |
| protocol | `inbox_unknown_type_parses_as_unknown` | forward compatibility |
| core | `inbox_enqueue_then_deliver_removes_item` | no duplicate row |
| core | `inbox_replayed_events_are_idempotent` | reconnect safety |
| core | `inbox_snapshot_wins_over_events` | snapshot semantics |
| core | `inbox_delivery_toggle_updates_item` | PATCH result applied |
| engine | `prompt_serializes_delivery_only_when_set` | body shape |

E2E (`crates/engine/tests/M9-01.rs`, gate `RUST_HARNESS_MOCK_SCRIPTED_PROVIDER`):
start a turn with a 400 ms-per-chunk script, send a second prompt with
`delivery: queue`, assert `ListInbox` returns it, cancel it, assert the server
inbox is empty; then steer a third prompt and assert
`session.inbox.delivered` arrives. Durable assertion: `GET /inbox` is empty
after cancel and the session has exactly two user messages.

## 9. Acceptance

- [ ] Queue and steer both reach the server with the right `delivery`.
- [ ] Cancel removes the item on the server, not only locally.
- [ ] A replayed `enqueued` event never duplicates an item.
- [ ] An unknown inbox type renders read-only and can still be cancelled.
- [ ] `cargo test --workspace` and the e2e file pass.

## 10. Risks

`session.inbox.delivered` may arrive before the transcript snapshot; the
reducer must tolerate a delivered id that was never seen. Steering semantics
depend on the server; do not simulate them locally.
