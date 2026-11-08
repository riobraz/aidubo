# Contributing to Rust Harness

Thanks for taking the time to contribute.

## Requirements

- Rust stable with the 2024 edition (workspace MSRV: 1.90)
- macOS or Linux — GPUI does not officially support Windows
- OpenCode v2 (`opencode`) for the sidecar integration tests

## Build and run

```sh
cargo build --workspace
cargo run -p rust-harness -- /path/to/project
make bundle   # dist/Rust Harness.app with the bundled OpenCode sidecar
```

The first build compiles GPUI and takes a while. See the README for the
environment variables that configure the project directory, sidecar binary,
and logging.

## Tests

```sh
cargo test --workspace
```

Covers protocol parsing, the pure state reducers in `harness-core`, and the SSE
event parser. The sidecar integration tests in `crates/engine/tests/sidecar.rs`
launch a real `opencode serve`:

- Set `RUST_HARNESS_SKIP_E2E=1` to skip the session lifecycle test when
  `opencode` is not installed.
- The streaming test runs only when `RUST_HARNESS_MOCK_PROVIDER` points at an
  OpenAI-compatible mock endpoint.
- The tool test runs only when `RUST_HARNESS_MOCK_TOOL_PROVIDER` points at a
  mock that streams reasoning and calls a tool.
- The permission tests run only when `RUST_HARNESS_MOCK_PERMISSION_PROVIDER`
  points at a mock that calls a shell tool; the project config asks before
  running it.

Before opening a pull request, also run:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Architecture rules

The crate boundaries are load-bearing; keep them intact.

- `harness-protocol` — wire types only. No GPUI, no IO. Unknown server
  variants must deserialize to a catch-all instead of failing.
- `harness-core` — pure state machines and reducers. No GPUI, no IO. This is
  where most correctness lives, so add unit tests with the behavior.
- `harness-engine` — owns the tokio runtime, the HTTP client, and the SSE
  stream. It must never touch GPUI entities; communicate through the event and
  command channels.
- `harness-views` — rendering and input handling. No network or disk access;
  read `harness-core` state and dispatch engine commands.
- `harness-app` — the composition root; only this crate wires the others
  together.
- `harness-ui` — theme tokens and shared visual primitives.

Treat the OpenCode API as an external contract: keep version-specific routing
in one adapter, tolerate unknown events, and prefer upserts by ID over blind
appends.

## Commits and pull requests

- Use the imperative present tense in the subject line (e.g. "Add tool-call
  streaming"), keep it concise, and explain the *why* in the body.
- Keep changes focused; unrelated refactors belong in a separate commit.
- Reference the milestone or architecture section when a change affects scope.
- Do not commit credentials, session data, or anything under `target/`.

## Scope

The MVP scope and explicit non-goals are defined in
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). Changes that add a non-goal
(diff review, editor, terminal, multi-window, and so on) should be discussed in
an issue first.

## License

By contributing, you agree that your contributions are licensed under the
[MIT License](LICENSE).
