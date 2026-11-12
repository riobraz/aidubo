# M16-03 — Session export, import, and stats

| | |
| --- | --- |
| Status | done |
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
| `GET /api/experimental/session/stats` | query `from`, `to` (epoch **milliseconds**), `project`, `timezone`, `tools` (`none\|summary\|detail`) → `{data: SessionStats.Info}` |

`SessionTransfer.Data` = `{info: Session.Info, messages: [Session.Message.Info]}`.
`SessionStats.Info` = `{range, sessions, subagents, prompts, steps, tokens,
cost, tools, activeDays, streak, activity, models}`.

## 3. Protocol

```rust
pub struct SessionTransfer { pub info: serde_json::Value,
                             pub messages: Vec<serde_json::Value> }

pub struct StatsRange { pub from: i64, pub to: i64 }

pub struct SessionStats { pub range: StatsRange,
                          pub sessions: u64, pub subagents: u64, pub prompts: u64,
                          pub steps: u64, pub tokens: TokenUsage, pub cost: f64,
                          pub tools: Option<serde_json::Value>,
                          pub active_days: u64, pub streak: u64,
                          pub activity: Vec<StatsActivity>,
                          pub models: Vec<StatsModel> }
```

`info` and `messages` stay raw JSON, unlike the rest of the protocol: the
typed `Session`/`SessionMessage` types are deserialize-only and drop fields
they do not model, so an import would silently lose data. Both a bare
`{info, messages}` document and the endpoint's `{data: ...}` envelope parse.

Fixture: an export with one user and one assistant message; a stats payload
with `tools: "none"` (no `tools` object).

## 4. Engine

Commands `ExportSession{session_id, sanitize, path}`, `PreviewImport{path}`,
`ImportSession{path, copy}`, `LoadStats{request_id, range}`. Events
`SessionExported{path, bytes}`, `ImportPreviewed{path, transfer}`,
`SessionImported{session}`, `StatsLoaded{request_id, result}`, plus `*Error`
variants.

The engine writes the export file itself. The spec originally had the app
write it, but `harness-views` is IO-free and the app crate has no async file
access; the engine already owns disk access, and the file dialog only supplies
the path.

`import_copy` rewrites the session id and every message id. OpenCode checks
the session id first (409 `ConflictError` when it exists), but message ids are
a global namespace: a fresh session id with the original message ids fails
with a 500. The UI's copy toggle defaults on; turning it off preserves the ids
and therefore the server's conflict behaviour.

Resnapshot: after import, refresh the session list and select the imported
session.

## 5. Core

`TransferStore`: last export path, last import result, stats cache keyed by
range.

Rules:

- Import validates the envelope before sending: `info.id` must be non-empty
  and every message must be an object with a non-empty string id; a mismatch
  is refused locally with a clear message.
- Import never overwrites an existing session; a 409 or an
  `already exists` message becomes "Session already exists: <id>. The server
  will not overwrite it; import a copy with a new id."
- Sanitize defaults to **true** for export from the UI.

## 6. UI

Session menu: "Export…", "Import…". Import opens a file dialog and shows a
preview (title, message count, project). Stats: a modal with the date range,
totals, and a per-model breakdown; no charts required.

The stats range is a preset (7 days / 30 days / all time) rather than a date
picker: the endpoint takes epoch milliseconds, and ISO strings return a 500.

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

E2E (`tests/M16-03.rs`): run one turn, export with `sanitize: true` to a temp
file, assert the JSON contains no `sk-`, `Bearer `, or `Authorization`
substring and has two messages; import it into a second empty project, assert
the returned session's message count matches and its title is preserved.
Durable assertion: `GET /message` on the imported session.

Corrections from the implementation: the spec's
`RUST_HARNESS_MOCK_SCRIPTED_PROVIDER` gate does not exist; the test uses
`support::mock::MockProvider` directly, so no new environment variable is
needed. One turn stores **three** transcript messages in opencode v2.2.7 (user,
assistant, and the idle assistant record), so the test asserts `>= 2` and uses
the plain export's count as the authoritative round-trip size.

## 9. Acceptance

- [x] Export/import round-trips messages in order.
- [x] Sanitize is on by default and its effect is visible in the file.
- [x] A conflicting import is refused without side effects.
- [x] Stats render for an empty range without dividing by zero.

## 10. Implementation notes (2026-11-11)

Verified against opencode v2.2.7 with a mock provider:

- Export: `GET /api/experimental/session/{id}/export?sanitize=true|false`
  returns `{data: {info, messages}}`. Sanitize replaces the title with
  `[redacted:session-title:<id>]`, the directory with
  `/[redacted:session-directory:<id>]`, and metadata with a `redacted:` marker.
  A sanitized export therefore does **not** preserve the title: the
  title-preserving round trip in the e2e uses `sanitize: false` after the
  separate sanitize assertions.
- Stats: the range query is epoch milliseconds. ISO timestamps are rejected
  with a 500. `tools=none` omits the `tools` object, so it stays optional in
  the protocol. The payload is `range, sessions, subagents, prompts, steps,
  tokens, cost, activeDays, streak, activity, models`; `models[].model` is
  `{id, providerID, variant}`. Totals are global, not scoped to the open
  project.
- Import: `POST /api/experimental/session/import` with
  `{info, messages, location: {directory}}`. The same session id returns 409
  `ConflictError "Session already exists: <id>"`; a missing message `id`
  returns 400 `InvalidRequestError`; a fresh session id with the original
  message ids returns 500 because message ids are global. This is why copy
  mode rewrites every id.
- The e2e asserts the durable side: the imported session appears in the
  project's session list with exactly the exported message count, the
  conflicting import adds no session, and stats within an all-time range
  report the turn. Both sessions are deleted and both sidecars shut down.

## 11. Risks

Exports can contain prompt content. The UI says so before writing the file and
defaults to sanitize; the e2e test asserts the redaction.
