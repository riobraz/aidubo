# E0 — Engine and core module seams

| | |
| --- | --- |
| Status | done |
| Wave | 0 |
| Size | M |
| Depends on | — |
| Parallel with | T0, M14-02 |
| Owns | `crates/engine/src/features/` (new), `crates/core/src/{sessions,transcript,permissions,composer}.rs` (new) |
| Shared touch | `crates/engine/src/{lib,api,supervisor,events}.rs`, `crates/core/src/lib.rs` |

## 1. Goal

Make every later node additive. Today `crates/engine/src/api.rs` (537 lines)
and `crates/core/src/lib.rs` (2,607 lines) are single files that every feature
would edit, so two agents working in parallel always conflict. After E0:

- one engine module per feature area, each under 250 lines;
- one core module per store, re-exported from `lib.rs`;
- a generic failure event so new commands do not invent their own error path;
- **no behavior change**; the existing suite is the regression test.

This is the only node that is allowed to move code without adding a feature.

## 2. Design

### Engine

```
crates/engine/src/
  lib.rs           Engine, EngineConfig, command/event enums (append-only)
  supervisor.rs    runtime, channel pump, SSE loop
  events.rs        wire event -> StreamEvent
  locate.rs
  api.rs           shared HTTP helpers: client, base URL, auth, error mapping
  features/
    mod.rs
    sessions.rs    list/create/rename/delete
    transcript.rs  message page -> SessionMessage
    prompt.rs      prompt, command, interrupt
    catalogs.rs    command/skill/agent/model
    files.rs       fs/find
    permissions.rs permission list/reply
```

`api.rs` keeps only `Client`, `get/post/patch/delete`, `ApiError`, and the
`Envelope` helpers. Feature modules take `&Client` and return `Result<T,
ApiError>`; the supervisor maps results to `EngineEvent`.

Add:

```rust
EngineEvent::RequestFailed { request_id: u64, message: String }
```

for commands that only need success/failure. Existing variants stay.

### Core

```
crates/core/src/
  lib.rs            re-exports only
  sessions.rs       SessionStore
  transcript.rs     TranscriptStore, Entry, AssistantEntry, ToolCall
  permissions.rs    PermissionQueue
  composer.rs       PromptDraft, PickerState, snippets
```

`pub use` keeps every existing path (`harness_core::SessionStore`, …) valid.

## 3. Non-goals

- No new endpoint, event, or UI.
- No change to the public API of `harness-core` or `harness-engine`.
- No dependency upgrade.

## 4. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| `harness-engine` | `api_error_maps_to_safe_message` | error text has no URL, token, or body |
| `harness-engine` | `feature_modules_keep_envelope_shapes` | `{data}`, `{location,data}`, bare array still parse |
| `harness-core` | `reexports_resolve_old_paths` | `harness_core::SessionStore` and friends still compile |
| `harness-core` | `transcript_module_has_no_io` | `cargo tree` check documented in STATUS: no tokio/reqwest |

E2E: the existing `crates/engine/tests/sidecar.rs` suite must pass unchanged.
That is the whole e2e requirement for E0.

## 5. Acceptance

- [ ] `api.rs` under 250 lines; every feature module under 250 lines.
- [ ] `harness-core/src/lib.rs` contains only `mod`/`pub use`.
- [ ] `cargo test --workspace` passes with no test renamed or deleted.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes.
- [ ] No `pub` item removed; `cargo doc` has no new warnings.

## 6. Risks

Moving code can hide a semantic change. Mitigation: the diff must be
move-only except for imports; review with `git diff -M --stat` and confirm no
logic lines changed.

## 7. Verification

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUST_HARNESS_SKIP_E2E=1 cargo test --workspace
```
