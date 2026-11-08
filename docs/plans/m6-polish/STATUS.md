# M6 — Polish: status

- **Status:** done
- **Last updated:** 2026-11-08

## Completed

- [x] `harness-ui` tokens derived from gpui-component; `Theme::current(cx)`.
- [x] `EngineEvent::StreamDisconnected`; pump announces one drop per outage.
- [x] Workspace tracks stream connectivity, shows a reconnecting banner, and
      reloads transcript + pending permissions on `StreamConnected`.
- [x] Multiline composer (`TextareaState`), Enter sends, Shift+Enter newline.
- [x] Assistant text renders as markdown (`TextView::markdown`) with stable
      per-part IDs; empty text shows a muted ellipsis.
- [x] Actions + `Workspace` key context: Cmd/Ctrl+N, Cmd/Ctrl+L,
      Cmd/Ctrl+Shift+L, Esc. Sidebar Light/Dark button.
- [x] Startup follows the OS appearance via `Theme::sync_system_appearance`.
- [x] Docs updated: `README.md` status/shortcuts, `ARCHITECTURE.md` §5.3,
      §5.4, §6.3.

## In progress / next

Nothing outstanding for M6. Potential follow-ups: persist the theme override,
collapsible tool output, packaging.

## Blockers

None.

## Verification

- `cargo check --workspace --all-targets` — finished with no errors.
- `cargo fmt --all -- --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean (fixed
  pre-existing lints in `harness-core`, `locate.rs`, `supervisor.rs`,
  `workspace.rs`, and the sidecar test).
- `cargo test --workspace` — 24 core, 11 protocol, 9 engine unit, 5 sidecar
  (the gated provider tests skip or run according to their env vars); all pass.
- Smoke run `RUST_HARNESS_AUTOQUIT_MS=5000 cargo run -p rust-harness --
  /tmp/rh-m6`: readiness probe answered, `event stream connected` logged,
  window rendered, auto-quit, `event stream disconnected`, and `pgrep -fl
  "opencode serve"` found nothing afterwards.
- Manual theme/shortcut checks were not automated; the code paths were
  exercised by the smoke run only insofar as window rendering goes.
