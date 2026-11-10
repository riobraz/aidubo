# M9-02 — Fork a session at a message

| | |
| --- | --- |
| Status | ready |
| Wave | 1 |
| Size | S |
| Depends on | T0 |
| Parallel with | M9-01, M9-03 … M9-06 |
| Owns | `crates/protocol/src/fork.rs`, `crates/engine/src/features/fork.rs`, `crates/views/src/session_actions.rs` |
| Shared touch | `protocol/src/lib.rs`, `engine/src/{lib,supervisor}.rs`, `core/src/sessions.rs` |

## 1. Goal

From any message in the transcript, the user can create a new session that
contains the conversation up to (not including) that message. The new session
opens immediately and is listed in the sidebar.

## 2. Verified contract

| Endpoint | Shape |
| --- | --- |
| `POST /api/session/{sessionID}/fork` | body `{before: Session.Message.ID \| null}` → `{data: Session.Info}` |

Errors: 404 `MessageNotFound` / `SessionNotFound`. Event: `session.forked`.
`Session.Info` carries `fork: object` describing the origin; the app only
needs it for display and must tolerate its absence.

## 3. Protocol

```rust
pub struct ForkRequest { pub before: Option<String> }

pub struct SessionFork { pub session_id: Option<String>, pub message_id: Option<String> }
```

`Session` gains `fork: Option<SessionFork>` with `#[serde(default)]`.
Fixture: a session with `fork` present, one with it absent, one with unknown
extra fields.

## 4. Engine

Command `ForkSession{session_id, before: Option<String>}`. Event
`SessionForked{session: Session}` on success, `RequestFailed{request_id,
message}` on error. The supervisor does **not** select the new session; the
view does, so a fork started from a background panel does not steal focus.

Resnapshot: after a fork, the session list is refreshed (`ListSessions`) so the
new session appears even if the event is missed.

## 5. Core

`SessionStore::upsert` already handles insertion and ordering. Add:

```rust
pub fn select_newest(&mut self) -> Option<String>
```

so the view can open the forked session deterministically.

## 6. UI

Each user message and each assistant message gets a "Fork from here" action in
its overflow menu. A confirmation dialog states: "Creates a new session with
the conversation before this message." On success the new session is selected
and its transcript loaded.

## 7. Non-goals

Forking into another project or worktree (M11), merging a fork back, renaming
the fork automatically.

## 8. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| protocol | `session_fork_optional_and_unknown_fields` | missing `fork` parses |
| core | `upsert_forked_session_sorts_and_selects` | ordering + selection |
| engine | `fork_body_serializes_null_before` | `{"before":null}` for full copy |

E2E (`tests/M9-02.rs`, gate `RUST_HARNESS_MOCK_SCRIPTED_PROVIDER`): run two
turns, load the transcript, fork before the second user message, assert the
new session id differs, `GET /api/session/{new}/message` returns exactly the
first turn's messages, and the original still has all of them. Durable
assertion: message counts on the server.

## 9. Acceptance

- [ ] Fork before message N yields N-1 messages in the new session.
- [ ] Fork with `before: null` copies the whole transcript.
- [ ] The original session is untouched.
- [ ] A 404 shows a short message and leaves the UI usable.

## 10. Risks

`before` semantics could shift between OpenCode versions. The e2e test is the
contract; if the count is off by one, fix the spec and the code together, never
the assertion alone.
