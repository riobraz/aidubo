# M16-03 — Session export, import, and stats

| | |
| --- | --- |
| Status | ready |
| Wave | 1 |
| Size | M |
| Depends on | T0 |
| Parallel with | M16-01, M16-02, M16-04 |
| Owns | `crates/protocol/src/transfer.rs`, `crates/core/src/transfer.rs`, `crates/engine/src/features/transfer.rs`, `crates/views/src/transfer.rs` |
| Shared touch | `protocol/src/lib.rs`, `engine/src/{lib,supervisor}.rs`, `views/src/workspace.rs` |

## 1. Goal

A session can be exported to a file and imported into a fresh project with its
messages intact. An optional sanitize pass strips secrets before the file
leaves the machine. A stats view summarizes sessions, prompts, tokens, and cost
for a date range.

## 2. Verified contract

| Endpoint | Shape |
| --- | --- |
| `GET /api/experimental/session/{sessionID}/export` | query `sanitize` (`true\|false`) → `{data: SessionTransfer.Data}` |
| `POST /api/experimental/session/import` | body `{info, messages, location}` → `{data: Session.Info}` |
| `GET /api/experimental/session/stats` | query `from`, `to`, `project`, `timezone`, `tools` (`none\|summary\|detail`) → `{data: SessionStats.Info}` |

`SessionTransfer.Data` = `{info: Session.Info, messages: [Session.Message.Info]}`.
`SessionStats.Info` = `{range, sessions, subagents, prompts, steps, tokens,
cost, tools, activeDays, streak, activity, models}`.

## 3. Protocol

```rust
pub struct SessionTransfer { pub info: Session, pub messages: Vec<SessionMessage> }

pub struct SessionStats { pub sessions: u64, pub prompts: u64, pub steps: u64,
                          pub tokens: TokenUsage, pub cost: f64,
                          pub active_days: u64, pub streak: u64 }
```

Fixture: an export with one user and one assistant message; a stats payload
with `tools: "none"` (no `tools` object).

## 4. Engine

Commands `ExportSession{session_id, sanitize}`, `ImportSession{path}`,
`LoadStats{range}`. Events `SessionExported{path, bytes}`,
`SessionImported{session}`, `StatsLoaded{stats}`, plus `*Error` variants.

Export writes to a user-chosen path through the app crate (the engine returns
bytes, the app writes the file) so `harness-engine` stays free of dialogs.

Resnapshot: after import, refresh the session list and select the imported
session.

## 5. Core

`TransferStore`: last export path, last import result, stats cache keyed by
range.

Rules:

- Import validates the envelope before sending: `info.id` must be non-empty
  and `messages.len()` must equal the file's count; a mismatch is refused
  locally with a clear message.
- Import never overwrites an existing session; the server returns 409 and the
  app shows "Session already exists".
- Sanitize defaults to **true** for export from the UI.

## 6. UI

Session menu: "Export…", "Import…". Import opens a file dialog and shows a
preview (title, message count, project). Stats: a modal with the date range,
totals, and a per-model breakdown; no charts required.

## 7. Non-goals

Cross-app formats, attachments over a size cap, cloud sync, editing an export.

## 8. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| protocol | `transfer_parses_info_and_messages` | envelope |
| protocol | `stats_without_tools_parses` | optional object |
| core | `import_rejects_message_count_mismatch` | local guard |
| core | `import_conflict_shows_message` | 409 handling |
| core | `stats_cache_keyed_by_range` | no stale range |

E2E (`tests/M16-03.rs`, gate `RUST_HARNESS_MOCK_SCRIPTED_PROVIDER`): run one
turn, export with `sanitize: true` to a temp file, assert the JSON contains no
`sk-`, `Bearer `, or `Authorization` substring and has two messages; import it
into a second empty project, assert the returned session's message count
matches and its title is preserved. Durable assertion: `GET /message` on the
imported session.

## 9. Acceptance

- [ ] Export/import round-trips messages in order.
- [ ] Sanitize is on by default and its effect is visible in the file.
- [ ] A conflicting import is refused without side effects.
- [ ] Stats render for an empty range without dividing by zero.

## 10. Risks

Exports can contain prompt content. The UI says so before writing the file and
defaults to sanitize; the e2e test asserts the redaction.
