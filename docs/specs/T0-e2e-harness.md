# T0 — Shared e2e harness and scripted mock provider

| | |
| --- | --- |
| Status | done |
| Wave | 0 |
| Size | M |
| Depends on | — |
| Parallel with | E0, M14-02 |
| Owns | `crates/engine/tests/support/mod.rs`, `crates/engine/tests/support/mock.rs`, `crates/engine/tests/fixtures/` |
| Shared touch | `crates/engine/tests/sidecar.rs` (imports only) |

## 1. Goal

Every later node adds one e2e file and one scenario. Without shared support,
each file would copy 100 lines of server boot, temp-dir, and event-wait code,
and the mock provider could only say one fixed thing.

After T0:

- `support::TestProject` — unique temp dir, canonicalized, removed on drop.
- `support::start_engine(&project)` — spawn, wait for `Ready`, return
  `(Engine, Receiver<EngineEvent>, port)`.
- `support::wait_for`, `support::wait_for_stream`, `support::expect_absent`.
- `support::shutdown(engine, port)` — shut down, assert the port is closed.
- `support::opencode()` — `locate_binary` or a clean skip.
- `support::MockProvider` — an OpenAI-compatible HTTP server written with
  `std::net::TcpListener` (no new dependency), driven by a JSON script.

## 2. Scripted mock

The mock serves `POST /v1/chat/completions` with `text/event-stream` and
`GET /v1/models`. The script is a JSON file whose path is in
`RUST_HARNESS_MOCK_SCRIPTED_PROVIDER_SCRIPT`; if unset, the mock uses the
existing single-turn text behavior so current tests keep working.

```json
{
  "turns": [
    { "chunks": ["Not done: ", "tests still fail."], "delay_ms": 40 },
    { "chunks": ["Done. All tests pass."], "tool": null },
    { "chunks": [], "tool": { "name": "bash", "input": { "command": "true" } } },
    { "chunks": ["Waiting."], "permission": { "action": "shell", "resources": ["true"] } }
  ]
}
```

Rules:

- The mock consumes one entry per request, in order; the last entry repeats.
- `delay_ms` is applied between chunks so a test can send while a turn runs.
- `tool` emits a tool-call delta followed by `finish_reason: "tool_calls"`.
- `permission` emits a tool call that OpenCode will gate behind `ask`.
- The server binds `127.0.0.1:0`, reports the URL through
  `MockProvider::base_url()`, and stops on drop.
- It never reads the project, never writes outside the temp dir.

The provider is registered through the existing env mechanism used by
`RUST_HARNESS_MOCK_PROVIDER` (an `opencode.json` in the temp project pointing
at the mock base URL).

## 3. Non-goals

- No real provider, no network access.
- No change to production crates.
- No new dev-dependency.

## 4. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| `harness-engine` (support) | `mock_serves_scripted_turns_in_order` | turn 2 text differs from turn 1 |
| `harness-engine` (T0) | `mock_emits_tool_call_chunks` | tool-call delta is well formed |
| `harness-engine` (T0) | `mock_lists_models` | the model list the client probes first |
| `harness-engine` (T0) | `engine_streams_from_the_scripted_provider` | a real sidecar streams text from the mock |
| `harness-engine` (support) | `mock_stops_and_releases_port` | drop closes the listener |
| `harness-engine` (support) | `temp_project_is_unique_and_removed` | two projects differ; drop removes |

E2E: `crates/engine/tests/sidecar.rs` passes after switching its helpers to
`support::*`, with every assertion preserved (`RUST_HARNESS_MOCK_PROVIDER`
still works). `crates/engine/tests/T0.rs` covers the mock and one real sidecar
round trip; it skips cleanly without `opencode`.

## 5. Acceptance

- [ ] `sidecar.rs` contains no `std::env::temp_dir()` or `wait_for` copy.
- [ ] The scripted mock boots a real sidecar and streams a turn
      (`engine_streams_from_the_scripted_provider`). Sending during a stream is
      M9-01's scenario, not this node's.
- [ ] Every helper returns a `Result` or a skip message; no `unwrap` in
      helpers except on a poisoned mutex.
- [ ] `cargo test -p harness-engine --test sidecar` passes with
      `RUST_HARNESS_MOCK_PROVIDER` set, and skips cleanly without it.

## 6. Risks

A hand-written HTTP server is easy to get subtly wrong. Mitigation: it is
test-only, it asserts its own framing in a unit test, and the first real
consumer (M9-01) validates it end to end.

## 7. Verification

```sh
cargo test -p harness-engine --test sidecar -- --nocapture
RUST_HARNESS_SKIP_E2E=1 cargo test -p harness-engine
```
