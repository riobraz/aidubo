# M8 — Rich composer

## Goal

A prompt can carry project files, an agent, a skill, and a slash command, and
the user can switch a session's model and agent from the composer without
editing OpenCode configuration. Typing `@`, `/`, `$`, or `#` opens a picker
backed by the sidecar (or by a local registry for snippets); selecting an item
produces a well-formed wire request instead of literal text.

## Context

- [`docs/ROADMAP.md`](../../ROADMAP.md) — M8 definition, verified endpoint list,
  acceptance criteria, and risks. The roadmap explicitly requires re-verifying
  the live `/openapi.json` before implementing this milestone.
- [`docs/ARCHITECTURE.md`](../../ARCHITECTURE.md) §5.4 (protocol types), §6.2
  (entity graph: `ComposerState` owns draft, agent, model), §6.3 (Composer and
  StatusBar), §9 (security).
- [`docs/adr/0001-use-opencode-as-agent-engine.md`](../../adr/0001-use-opencode-as-agent-engine.md)
  — OpenCode owns session state; the client is a view over it.
- Current code:
  - `crates/protocol/src/lib.rs:316` — `PromptRequest { text }` is the only
    prompt shape today.
  - `crates/engine/src/lib.rs:42-77` — `EngineCommand` has no file, catalog,
    model, or agent commands.
  - `crates/engine/src/api.rs:127` — `prompt()` posts `{ "text": ... }`.
  - `crates/views/src/workspace.rs:43,70-81,380-400,1238-1281` — the composer
    is a plain `TextareaState`; Enter sends `value().trim()`; the composer bar
    renders an 84 px textarea plus Send/Stop.
  - `crates/core/src/lib.rs` — pure state only; this is where `PromptDraft` and
    the picker reducers belong.
- Verified in the pinned UI stack (`gpui-base` 0.9.0,
  `gpui-pre` 0.3.11):
  - `TextareaState::selected_range()` returns **byte** offsets into the rope;
    `text()`/`value()` expose the text and `set_selected_range()` takes bytes.
  - `InputEvent` has `Change`, `PressEnter { shift, .. }`, `Focus`, `Blur`.
  - `InputEvent::Change` is emitted per edit; `set_value()` does **not** emit
    `Change`.
  - `Div::capture_key_down` exists, so the composer container can intercept
    Enter/Up/Down/Esc/Tab while a picker is open before the textarea handles
    them.

No new crate or runtime dependency is required. A new ADR is not required:
this milestone uses the existing loopback sidecar and adds no new trust
boundary. If scope grows to local attachments, image upload, or shell
execution, stop and write an ADR first.

## Scope

- `@` file picker backed by `GET /api/fs/find`; selecting a file inserts an
  `@path` mention and attaches the file with `uri` + `mention` offsets.
- `@` agent results when the server lists agents (selecting one adds it to the
  prompt's `agents[]`).
- `/` command picker backed by `GET /api/command`; sending runs
  `POST /api/session/{id}/command` instead of a prompt.
- `$` skill picker backed by `GET /api/skill`; selecting attaches the skill to
  the prompt's `skills[]`.
- `#` snippet picker backed by an app-local, built-in registry; selecting
  expands text into the composer. No server call.
- Model and agent pickers backed by `GET /api/model`, `GET /api/model/default`,
  and `GET /api/agent`; choosing one calls `POST /api/session/{id}/model` or
  `POST /api/session/{id}/agent` and persists for that session.
- Draft chips above the textarea for attached files, agents, skills, and the
  selected command, each removable.
- Graceful degradation: picker errors and empty results leave typed text as
  plain text; nothing panics and no keystroke is lost.

## Non-goals

- Image paste, drag-and-drop attachments, and binary uploads. The roadmap does
  not verify an upload contract; deferred.
- Shell mode (`!`). It only returns if it can reuse the existing
  permission/confirmation path; otherwise it belongs to M14.
- User-defined or persisted snippets; M8 ships built-ins only.
- Command arguments UI beyond the literal text after the command name.
- Editing OpenCode configuration, provider credentials, or agent definitions.
- Inline rich-text chips inside the textarea. Mentions stay plain text; chips
  are a summary row above the input.
- Multi-cursor handling for triggers (the active cursor is used).
- Windows/Linux input specifics; macOS is the supported target.

## Design

### 1. Wire protocol (`harness-protocol`)

Add types for the newly used endpoints. Exact field names, query parameters,
and enum values must be confirmed against the live spec in step 1; the shapes
below are the intended contract, not a substitute for that check.

- File discovery: `FileEntry { path, kind?, ... }`, response
  `Envelope<Vec<FileEntry>>` for `GET /api/fs/find`.
- Catalog items: `CommandInfo`, `SkillInfo`, `AgentInfo`, `ModelInfo` (name it
  `ModelInfo` to avoid colliding with the existing `ModelRef`). Every
  non-essential field is `#[serde(default)]`.
- `PromptRequest` grows optional `files`, `agents`, `skills`:
  `#[serde(skip_serializing_if = "Option::is_none")]` so an old server still
  receives `{ "text": ... }` and the existing sidecar e2e behavior is
  unchanged.
- `PromptFile { uri, mention }` with `MentionRange { start, end }` (exclusive
  end). The wire unit is what the server expects; see §3.
- `CommandRequest { command, arguments }` for `POST /api/session/{id}/command`.
- `SetModelRequest` / `SetAgentRequest` for the session endpoints.
- Response parsing stays tolerant: unknown fields are ignored by serde and
  missing optional fields default. Unknown catalog kinds are skipped by the
  reducer rather than becoming an error.
- Fixtures under `#[cfg(test)]` cover: complete payload, minimal payload with
  missing optional fields, unknown extra fields, and an unknown enum/kind.

Do not log request bodies: prompts, paths, and command arguments are user
content. Existing redaction rules apply.

### 2. `PromptDraft` in `harness-core`

One pure type owns the composer's semantic state:

```text
PromptDraft
├── text: String                 // exactly what the textarea contains
├── cursor: usize                // byte offset
├── files: Vec<FileAttachment>   // mention text + uri, offsets derived
├── agents: Vec<AgentAttachment> // chips, no inline offsets
├── skills: Vec<SkillAttachment> // chips, no inline offsets
├── command: Option<CommandSelection>
├── trigger: Option<Trigger>     // active picker token
└── snippets: &'static [Snippet] // built-ins
```

- `edit(text, cursor)` is called on every `InputEvent::Change`; it stores the
  new text and cursor, re-derives the active trigger, and calls
  `sync_mentions()`.
- `sync_mentions()` rescans `text` left to right for each attachment's literal
  mention token, assigning byte ranges in document order. It never patches
  old offsets. Attachments whose token no longer exists are dropped. Duplicate
  tokens are matched sequentially.
- A mention token is `@` followed by the path, preceded by start-of-text or
  whitespace, and ended by whitespace/end-of-text. Paths containing spaces are
  only supported if the verified API accepts a quoting form; otherwise the
  inserted token uses the server-returned path and step 1 records the
  limitation.
- `Trigger { kind, start, query }` is derived from the cursor:
  - only when the selection is collapsed (`selected_range().is_empty()`);
  - the sigil is at text start or preceded by whitespace;
  - the query is the text between the sigil and the cursor, and stops at
    whitespace;
  - `/` is only a command trigger at the start of the input (after leading
    whitespace), so paths and prose containing `/` do not open a picker.
- Insertion methods return a plain edit for the view to apply, instead of
  mutating GPUI state:
  - `select_file(entry)` → replace `[trigger.start, cursor)` with `@path`, set
    cursor after it, add the attachment.
  - `select_agent` / `select_skill` → remove the trigger text and add a chip.
  - `select_command` → keep `/name ` in the text and record the selection;
    editing the first token away clears it.
  - `select_snippet` → replace `[trigger.start, cursor)` with the expansion.
- `remove_file(index)` removes its mention token from the text; chip removal
  for agents/skills only updates the arrays.
- `to_send()` returns `DraftSend::Prompt(PromptRequest)` or
  `DraftSend::Command(CommandRequest)`. A draft is sendable when it has
  non-empty trimmed text or at least one attached agent/skill.
- The textarea remains the source of truth for `text`; the draft never writes
  to it outside explicit selection/removal operations, which prevents feedback
  loops. `set_value()` does not emit `Change`, so the view calls `edit()` after
  applying an edit.

### 3. Mention offsets and IME safety

- Internal offsets are UTF-8 byte offsets, matching `TextareaState` and Rust
  string slicing. This is the only representation that can slice safely.
- OpenCode is a TypeScript server; JavaScript string indices are UTF-16 code
  units. Step 1 must establish which unit the prompt API expects. If it is
  UTF-16, `PromptDraft` converts at the wire boundary with a tested helper
  (`byte_to_utf16`) and fixtures include emoji and accented text. If it is
  byte offsets, the helper is a no-op and the same tests still guard the
  conversion point.
- Offsets are recomputed after every edit, including edits before an existing
  mention. Tests cover: insertion before/inside/after a mention, deletion of
  the mention, two identical mentions, astral-plane emoji before the mention,
  combining accents, and a cursor at a non-char boundary (must snap without
  panicking).
- During IME composition, `InputEvent::Change` can arrive before the
  composition is committed. The view only updates the draft and trigger from
  `Change`; it never rewrites the textarea value while a trigger is active
  unless the user explicitly selects an item. A manual IME check is required
  (Japanese or Chinese input, plus dead keys).

### 4. Picker reducer in `harness-core`

```text
PickerKind  { File, Agent, Command, Skill, Snippet }
PickerState { kind, query, items, highlighted }
PickerItem  { File(FileEntry), Agent(AgentInfo), Command(CommandInfo),
              Skill(SkillInfo), Snippet(Snippet) }
PickerEffect { None, Fetch { request_id, kind, query } }
```

- `open(kind, query)` returns a fetch effect for server-backed kinds and
  populates snippets immediately for `Snippet`.
- `update_query(query)` returns a fetch effect; the view debounces file
  fetches (~150 ms) and cancels by incrementing `request_id`.
- `apply_results(request_id, items)` ignores a response whose ID is not the
  current one (stale responses after fast typing or session switches).
- `move_highlight(delta)` wraps; `selected()` returns the highlighted item
  without mutating.
- Empty results do not panic: the popover shows "No matches" and Enter keeps
  the literal text.
- The reducer is generic over data, so it is unit-tested without GPUI.

### 5. Engine commands and events

Add to `EngineCommand`:

- `FindFiles { request_id, query, limit }`
- `ListCatalog { request_id, kind: CatalogKind }` where `CatalogKind` is
  `Commands | Skills | Agents | Models | DefaultModel`
- `RunCommand { session_id, request: CommandRequest }`
- `SetSessionModel { session_id, model: ModelRef }`
- `SetSessionAgent { session_id, agent: String }`
- `SendPrompt { session_id, request: PromptRequest }` replaces
  `{ text }` so callers cannot forget the new arrays.

Add engine events:

- `FilesFound { request_id, result: Result<Vec<FileEntry>, String> }`
- `CatalogLoaded { request_id, kind, result: Result<Vec<CatalogItem>, String> }`
- `SessionConfigured { session_id, model_or_agent }` success/failure events,
  or reuse the existing command-error event if one fits; the UI must never
  assume the server applied an optimistic change.

`Engine` gets thin wrappers (`find_files`, `list_catalog`, `run_command`,
`set_session_model`, `set_session_agent`). All requests go through the same
supervisor command loop and tokio runtime; no GPUI types cross it.

Cancellation and coalescing:

- Request IDs make stale results harmless; the UI also ignores responses for
  a picker that has since closed.
- The view debounces file queries and never issues one per keystroke. The
  engine does not need an abort handle for M8; if a huge query blocks, the
  per-request 15 s timeout and ID filtering bound the damage.
- No new logging of queries or paths.

### 6. Engine adapter (`harness-engine/src/api.rs`)

- `find_files(query, limit)`: `GET /api/fs/find` with the canonical project
  directory plus the verified query parameters.
- `commands()`, `skills()`, `agents()`, `models()`, `default_model()`: GET the
  verified endpoints and deserialize into `Envelope<Vec<...>>` or the actual
  wrapper shape.
- `command(session_id, request)`: `POST /api/session/{id}/command`.
- `set_model` / `set_agent`: `POST /api/session/{id}/model` and
  `POST /api/session/{id}/agent`; handle both a returned session and a
  `204 No Content` (the shape is confirmed in step 1).
- `prompt` now accepts the full `PromptRequest`.
- Version-specific routing stays in this adapter; the UI and core stay
  protocol-agnostic.

### 7. Views: composer integration

`WorkspaceView` changes:

- Replace the `Entity<TextareaState>`-only composer with
  `ComposerState { input: Entity<TextareaState>, draft: PromptDraft,
  picker: Option<PickerState>, catalog: CatalogCache, ... }`; pure state stays
  in `harness-core`, the entity stays in the view.
- The `InputEvent` subscription handles:
  - `Change` → `draft.edit(value, selected_range().end)`, recompute the
    trigger, open/update the picker, and schedule a debounced file fetch.
  - `PressEnter { shift: false }` → if a picker is open, select the
    highlighted item; otherwise `send_prompt`. `Shift+Enter` always inserts a
    newline.
- A `capture_key_down` handler on the composer container intercepts
  `Up`/`Down`/`Tab`/`Esc`/`Enter` while the picker is open, calls
  `cx.stop_propagation()`, and lets the picker consume the key. Esc closes the
  picker without clearing text.
- `send_prompt` builds `draft.to_send()`: `DraftSend::Command` calls
  `Engine::run_command`, `DraftSend::Prompt` calls `Engine::send_prompt`. Clear
  both the textarea and the draft only after the engine accepts the command.
- `apply(EngineEvent)` handles `FilesFound` and `CatalogLoaded` by checking the
  request ID and picker/session identity before updating state, then calls
  `cx.notify()` once.
- On session switch, clear the draft/picker (drafts are not persisted in M8)
  and refresh the model/agent badges from the selected session.
- On engine ready, prefetch commands, skills, agents, models, and the default
  model once; refresh on picker open if a list is empty or older than the
  session switch.

Composer layout:

```text
div.flex.flex_col
├── chips row (files, agents, skills, command)   // hidden when empty
├── div.relative
│   ├── Textarea
│   └── picker popover (absolute, bottom-full)    // max 8 rows, scroll
└── toolbar: model badge | agent badge | Send/Stop
```

The popover is rendered inside the composer's relative container so it is not
clipped by the transcript; it must not take focus from the textarea. Clicking
an item selects it and refocuses the textarea. Mouse-down selection is
preferred over click so the textarea does not blur first; verify in the GPUI
test and manually.

### 8. `harness-ui` components

- `attachment_chip(theme, label, icon, on_remove)` — one visual primitive for
  files, agents, and skills; the label is truncated in the middle for long
  paths.
- `picker_popover(theme, state, on_select)` — header with the trigger kind,
  query, up to 8 rows, highlighted row, empty/error/loading rows, and a footer
  hint (`↑↓ navigate · Enter select · Esc close`).
- `model_badge` / `agent_badge` — compact buttons that open the same popover.
- Components take callbacks/data and never call the engine. Add theme tokens
  only if the defaults do not fit; prefer existing `harness-ui::Theme` tokens.

### 9. Snippets

- Built-ins live in `harness-core` as a `const` table (`name`, `description`,
  `expansion`), for example "review" and "explain-errors". They are plain text
  expansions; no server call, no persistence, no file IO.
- `#` opens the picker, filtering by name/description. Selecting replaces the
  trigger with the expansion and places the cursor at the end.
- The set is deliberately small; user-defined snippets need a storage and
  settings design and are out of scope.

### 10. Model and agent selection

- The badges show the selected session's `model`/`agent` when present, else
  "Default".
- Picking a model calls `SetSessionModel`; picking an agent calls
  `SetSessionAgent`. The UI optimistically updates `SessionStore` for
  responsiveness, then reconciles from the response or the next
  `ListSessions`. On error, revert the badge and show the existing error area.
- `SessionStore` gains tested `set_model` / `set_agent` upserts by session ID.
- Models include provider and variant from `ModelRef`; the badge shows the
  model ID (and provider in the picker) without truncating incorrectly.

### 11. Error handling and degradation

- File/search failure: keep the literal text, close the picker, show a short
  non-secret error in the popover or composer error area.
- Missing catalog entries: disable only that picker; other triggers still
  work.
- Unknown server variants: serde defaults/ignored fields; unknown catalog
  kinds are skipped; no `unwrap()` outside tests.
- Empty prompt with only file mentions: sendable if files exist; an empty
  draft is not sendable.
- Command with no arguments: send `arguments: ""` only if the verified
  contract accepts empty; otherwise omit the field.

## Steps

- [x] 1. Start a throwaway loopback `opencode serve`, fetch the authenticated
      `/openapi.json`, and record the exact shapes for `fs/find`, `fs/list`,
      `command`, `skill`, `agent`, `model`, `model/default`, prompt
      `files/agents/skills/mention`, `/command`, `/model`, and `/agent`
      (method, query params, request body, response wrapper, status codes, and
      the offset unit). Write the findings and the OpenCode version into
      `STATUS.md`. Do not implement against the roadmap's summary alone.
- [x] 2. Add protocol types and serde fixtures, including minimal/unknown
      payloads. Keep the existing `{ "text": ... }` prompt serialization when
      the new arrays are absent.
- [x] 3. Implement `PromptDraft`, trigger parsing, mention resynchronization,
      UTF-16/byte conversion, and `to_send()` in `harness-core` with unit
      tests.
- [x] 4. Implement `PickerState`/`PickerEffect` reducers with request-ID and
      highlight tests.
- [x] 5. Add engine commands, events, adapter methods, and `Engine` wrappers;
      add stub-server tests for method/path/body and response decoding.
- [x] 6. Integrate the draft with `WorkspaceView`: change subscription,
      debounce, trigger lifecycle, key interception, send/command split, and
      session-switch clearing.
- [x] 7. Add `harness-ui` chips, popover, and model/agent badges; wire model
      and agent selection to the session endpoints with optimistic update plus
      reconciliation.
- [x] 8. Add built-in snippets and the `#` flow.
- [x] 9. Add error/empty/unknown-variant paths and the GPUI trigger-to-
      selection test; verify no text is lost on failure.
- [x] 10. Update `docs/ARCHITECTURE.md` §5.4/§6.2/§6.3, the README status/
      limitations, and `STATUS.md` at each step. If implementation reveals a
      new trust boundary, stop and write an ADR.
- [ ] 11. Run the full verification suite and the manual macOS checklist.
      The automated suite passes; the manual macOS checklist still needs a
      human on a real window.

## Test plan

Automated:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p harness-protocol -p harness-core
cargo test -p harness-engine -- --skip sidecar   # or RUST_HARNESS_SKIP_E2E=1
cargo test --workspace
```

- Protocol tests: every new response/request fixture; missing optional fields;
  unknown fields; unknown kinds; prompt serialization with and without
  `files/agents/skills`; mention range field names; command/model/agent bodies.
- Core draft tests: trigger detection at start/middle/after whitespace;
  `/` only at start; mention insertion and cursor placement; offsets after
  edits before/inside/after a mention; duplicate mentions; removal; emoji and
  accented text; UTF-16 conversion; `to_send` command vs prompt; empty-draft
  rules.
- Core picker tests: query change produces a fetch only for server kinds;
  stale request IDs are dropped; highlight wraps; empty results; selection
  does not mutate.
- Core session tests: `set_model`/`set_agent` upsert by ID and preserve
  selection.
- Engine tests: a local `tokio::net::TcpListener` stub asserts method, path,
  query, JSON body, and decodes a success response for each new endpoint; a
  malformed/unknown payload becomes an `Err`/empty state, not a panic. No new
  dev-dependency is required.
- GPUI test: render `WorkspaceView` with the existing nonexistent-binary
  engine, set the composer value to `"@src/ma"`, trigger `Change`, assert the
  picker opens with a file result, select it, and assert the textarea contains
  `@src/main.rs` and the draft has one file attachment. A second case asserts
  Enter with the picker open does not call `send_prompt`.
- Sidecar e2e (gated by `RUST_HARNESS_SKIP_E2E=1` on machines without the
  binary, never weakened): send a prompt containing a file mention and a
  skill/agent attachment with `RUST_HARNESS_MOCK_PROVIDER`, then verify the
  durable server-side message/inbox contains the expected `files[]`,
  `agents[]`, and `skills[]`; run a `/command` and verify the server accepted
  it; set model/agent and verify `GET /api/session/{id}` reflects it; assert
  clean shutdown with no port left listening.

Manual (macOS):

- `@` opens after whitespace, filters as you type, and never opens inside a
  word; Up/Down/Tab/Enter/Esc behave; clicking an item keeps/restores focus.
- `@` with no matches, a network error, and a very large project does not
  freeze typing; stale results never replace newer ones.
- A mention before and after other edits keeps the correct `files[]` offsets;
  send it and inspect the accepted server message (with the mock provider).
- `/` at the start lists commands; sending runs a command, not a prompt; `/`
  in prose does not open the picker.
- `$` attaches a skill chip; removing the chip removes it from the next send.
- `#` expands a snippet; no network request is made.
- Model and agent pickers list server values, update the badge, survive a
  session switch (from the server snapshot), and revert with a visible error
  when the request fails.
- Japanese/Chinese IME and dead keys: trigger detection and mention insertion
  do not corrupt composed text.
- Light and dark themes; long paths truncate; the popover is not clipped at
  the bottom of the window.
- Packaged app (`make bundle`) behaves the same as `cargo run`.

## Risks

| Risk | Mitigation |
| --- | --- |
| Roadmap endpoint shapes differ from the live spec | Step 1 is mandatory: verify `/openapi.json` and record the results before writing types; treat the live spec as the contract. |
| Mention offset unit is UTF-16 in JS but bytes in Rust | Convert in one tested boundary helper; fixtures include emoji/accents; verify against a real prompt in the e2e test. |
| Offset drift after edits/IME | Recompute by rescanning the text on every `Change`; never patch offsets; drop attachments whose token disappeared; never rewrite the textarea during composition. |
| Trigger detection fights the textarea's key handling | Use `capture_key_down` on the composer container and `stop_propagation`; assert Enter-with-picker in a GPUI test. |
| File search latency or request storms | Debounce (~150 ms), cap results, request IDs, and per-request timeout; ignore stale/closed-picker results. |
| Huge or binary result payloads | Only request the verified file list; never read file contents in M8; cap display length and degrade to plain text. |
| Model list is large | Virtualize or cap the popover to 8 visible rows with scrolling/filtering. |
| Paths with spaces/special characters | Verify the API's path encoding; if no quoting contract exists, record the limitation and keep the token literal rather than corrupting it. |
| Command syntax ambiguity (`/` in prose) | Commands only trigger at the start of the input; otherwise `/` stays literal. |
| Server upgrade adds/removes fields | Tolerant serde defaults, skip unknown kinds, ignore unknown events; no panic on missing optionals. |
| Optimistic model/agent update diverges from server | Reconcile from the response or the next session snapshot; revert and surface an error on failure. |
| Focus/click behavior in the popover | Prefer mouse-down selection, refocus the textarea after selection, and cover it in the GPUI test plus the manual checklist. |

## References

- [`docs/ROADMAP.md`](../../ROADMAP.md) — M8 and sequencing
- [`docs/ARCHITECTURE.md`](../../ARCHITECTURE.md) — §5.4, §6.2, §6.3, §9
- [`AGENTS.md`](../../../AGENTS.md) — architecture, testing, and security rules
- `crates/protocol/src/lib.rs`
- `crates/core/src/lib.rs`
- `crates/engine/src/lib.rs`, `crates/engine/src/api.rs`,
  `crates/engine/src/supervisor.rs`
- `crates/views/src/workspace.rs`
- `crates/ui/src/lib.rs`
- `gpui-base` 0.9.0: `src/input/base/state.rs` (`InputEvent`, `text`,
  `selected_range`, `set_selected_range`)
- `gpui-pre` 0.3.11: `src/elements/div.rs` (`capture_key_down`)
