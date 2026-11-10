# M8 — Rich composer — Status

## Status

`in progress` — implementation and automated verification are complete; the
manual macOS checklist has not been run in a real window.

## Last updated

2026-11-09

## Completed

- **Endpoint verification (step 1).** Started a throwaway `opencode serve` on
  `127.0.0.1` (OpenCode v2.2.6) with a temporary project and checked the live
  requests and responses:
  - `GET /api/fs/find` takes `location[directory]`, `query`, `type=file`, and
    `limit`; the response is `{ "data": [{ path, type }] }`.
  - `GET /api/command`, `/api/skill`, `/api/agent`, and `/api/model` take
    `location[directory]` and return `{ "data": [...] }`.
  - `POST /api/session/{id}/command` takes `{ name, text, files?, agents?,
    skills? }` and returns `204`.
  - `POST /api/session/{id}/model` takes `{ model: { id, providerID?, variant? } }`
    and `POST /api/session/{id}/agent` takes `{ agent }`; both return `204`.
  - `POST /api/session/{id}/prompt` echoes an `Session.Inbox.User`; mention
    offsets are **UTF-16 code units**, the range includes `@` and `text`
    excludes it, and the URI is `file:///` plus the absolute path.
  - The echoed inbox `files` omit the top-level `uri`; the location stays in
    `source`. `PromptFile.uri` is therefore `#[serde(default)]`.
- Protocol types in `harness-protocol`: `FileEntry`, `CommandInfo`, `SkillInfo`,
  `AgentInfo`, `ModelInfo`, `MentionRange`, `PromptFile`, `FileSource`,
  `CommandRequest`, `SetModelRequest`, `SetAgentRequest`, plus the optional
  `files`/`agents`/`skills` on `PromptRequest` (absent arrays keep the old
  `{ "text": ... }` body). Serde fixtures cover complete, minimal, unknown
  field, and unknown-variant payloads.
- `PromptDraft` in `harness-core`: trigger detection (`@`, `/`, `$`, `#` with
  the documented boundary rules), mention resynchronization by rescanning,
  UTF-16 conversion at the wire boundary, snippet expansion, chip removal,
  command splitting into `DraftSend::Command`, and `to_send()`.
- `PickerState`/`PickerEffect` in `harness-core`: request-ID filtering, stale
  result rejection, wrapping highlight, empty results, snippet source.
- Engine: `FindFiles`, `ListCatalog`, `RunCommand`, `SetSessionModel`,
  `SetSessionAgent`, `SendPrompt` commands; `FilesFound`, `CatalogLoaded`,
  `SessionModelSet`, `SessionAgentSet` events; API adapter methods and
  `Engine` wrappers. Stub-server tests assert method, path, query, and body for
  each endpoint and reject malformed catalogs.
- Workspace view: draft/picker/composer integration, debounced file search,
  catalog prefetch, chips, picker panel with loading/error/empty rows, model
  and agent badges with optimistic update and reconciliation, session-switch
  clearing, and command-vs-prompt sending.
- Picker keys are intercepted on the composer container (`capture_key_down`),
  so Up/Down/Esc/Tab work whether focus is in the textarea or the model/agent
  search input; Enter stays with the textarea/search subscriptions.
- `harness-ui::attachment_chip` shared primitive.
- GPUI tests: trigger-to-mention insertion, Enter-with-picker does not send,
  and picker navigation/choice/close.
- Sidecar e2e: sending a file mention plus an agent attachment, switching the
  session model and agent, verifying the echoed inbox payload and the durable
  `GET /api/session` state, and asserting the port is closed after shutdown.
- Docs: `docs/ARCHITECTURE.md` §5.4/§6.2/§6.3 and `README.md` status,
  shortcuts, and tests.

## In progress / next

- Manual macOS checklist from `PLAN.md` step 11: real window, mouse selection
  in the picker, IME/dead keys, light/dark themes, long paths, popover clipping,
  and the packaged app (`make bundle`). A human needs to run this.
- After the manual pass, mark this plan `done` and update the milestone table
  in `docs/ARCHITECTURE.md` and the README status from M8 to the next
  milestone.

## Blockers

- None for implementation. The manual checklist needs an interactive macOS
  session and, for sending, `RUST_HARNESS_MOCK_PROVIDER`.

## Verification

Run on 2026-11-09:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Results:

- `cargo fmt --all -- --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo test --workspace` — all pass: `harness-protocol` 14, `harness-core`
  36, `harness-engine` 17, sidecar 6, `harness-views` 4, other crates 0. The
  mock-gated sidecar tests skip with a clear message when their environment
  variables are unset.
- With the local OpenAI-compatible mock
  (`RUST_HARNESS_MOCK_PROVIDER=http://127.0.0.1:47899/v1`), all 6 sidecar tests
  pass, including `attachments_and_session_selection_reach_the_server` and
  `streams_assistant_text_from_a_mock_provider`.
- The live-probe project and mock server were throwaway loopback processes and
  are not part of the repository.
