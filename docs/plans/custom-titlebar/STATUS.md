# Status — custom title bar

- **Status:** done
- **Last updated:** 2026-11-08

## Completed

- 2026-11-08 — Inspected the current window setup (`crates/app/src/main.rs`) and
  the workspace root layout (`crates/views/src/workspace.rs`).
- 2026-11-08 — Verified the available API in the pinned crates:
  `gpui-component` 0.9.0 `TitleBar` (`window_options`,
  `title_bar_options`, drag/double-click handling) and `gpui-pre` 0.3.11
  `WindowOptions`/`TitlebarOptions`/`WindowDecorations` and window control
  methods. No new dependency is needed.
- 2026-11-08 — Step 1: `WindowOptions` now starts from
  `..TitleBar::window_options()`; bounds and the window title are unchanged.
- 2026-11-08 — Step 2: the workspace root is a column with the title bar above
  a `flex_1` row holding the sidebar and main pane; the `Workspace` key context
  and its actions stay on the outer element.
- 2026-11-08 — Step 3: the bar shows "Rust Harness" and the project directory
  name on the left and the engine status dot/label on the right. It is
  non-interactive, so dragging and double-click stay with `TitleBar`.
- 2026-11-08 — Step 4: `title_bar_is_rendered_above_the_workspace` renders the
  workspace in a test window and asserts the bar starts at y = 0, is 34 px tall,
  ends above the body, and survives a click on the drag region.
  `gpui-kit/test-support` is a dev-dependency of `harness-views`.

## In progress / next

None for the macOS acceptance criteria. The optional Linux client-side
decorations step was not implemented. The interactive part of the manual
checklist (drag, double-click, traffic lights, fullscreen, resizing) still
needs a human at the keyboard; automated checks and both app launches pass.

## Blockers

- None. Linux client-side decorations are left to `TitleBar`'s built-in
  handling and were not exercised.

## Verification

- `cargo fmt --all -- --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo test --workspace` — 24 core, 12 engine unit, 5 sidecar, 11 protocol,
  and 1 views GPUI test; all pass.
- `RUST_HARNESS_AUTOQUIT_MS=6000 cargo run -p rust-harness -- /tmp` — window
  opened, sidecar became ready, event stream connected, auto-quit exit 0.
- `make bundle` — `dist/Rust Harness.app` built and ad-hoc signed;
  `codesign --verify --deep --strict` passes.
- Bundled app smoke run — located `Contents/Resources/opencode`, readiness
  probe answered, event stream connected, exit 0.
- Note: enabling GPUI's `test-support` feature makes Cargo resolve GPUI's
  optional `gpui-fast`/proptest graph into `Cargo.lock` (it is not built);
  `taffy` and `unicode-properties` were re-resolved to versions that satisfy
  both GPUI backends.
- Docs updated: `ARCHITECTURE.md` §6.3 and README.
