# M6 — Polish

## Goal

Make the MVP feel finished: assistant text renders as markdown, the composer is
a real multiline input with usable shortcuts, the color scheme follows the
system and can be toggled, and a dropped event stream is visible to the user
instead of silently freezing the transcript.

## Context

- `docs/ARCHITECTURE.md` §5.3 (event handling rules, reconnect), §5.4 (engine
  protocol additions), §6.3 (views), §12 milestone table.
- ADR 0001 — OpenCode remains the engine; the app only renders server state.
- M5 left `TextView` unused and `harness-ui` tokens hardcoded per call site.

## Scope / non-goals

In scope:

- Markdown rendering for assistant text parts.
- Multiline composer with Enter-to-send and Shift+Enter newline.
- Keyboard shortcuts: new session, focus composer, toggle theme, dismiss
  overlays.
- Theme tokens derived from gpui-component; follow the OS appearance at
  startup; light/dark toggle from the sidebar and by shortcut.
- Reconnect UX: engine event for a dropped stream, banner while disconnected,
  resnapshot on reopen.

Not in scope:

- Persisting the theme override across launches.
- Collapsible tool output, virtualized transcript, packaging.
- System-appearance live following after startup (toggle is explicit).

## Design

- `harness-ui::Theme` becomes a view of the active `gpui-component` theme
  (`Theme::current(cx)`), exposing `bg`, `surface`, `border`, `text`, `muted`,
  `accent`, and status colors. No component hardcodes colors.
- `harness-engine` adds `EngineEvent::StreamDisconnected`. `pump_events`
  announces a drop once per outage and resets on a successful subscribe, so the
  view is not flickered by fast retries. `StreamConnected` already existed and
  is emitted on every successful open.
- `WorkspaceView` keeps `stream_connected`; while the engine is ready and the
  stream is down it renders a non-blocking banner above the transcript. On
  `StreamConnected` it treats the stream as authoritative again by calling
  `follow_selection()` (transcript + pending permissions).
- Only `StreamEvent`s from the server clear `permission_in_flight`; transport
  events are handled separately, so a reconnect cannot leave buttons disabled.
- The composer becomes `Entity<TextareaState>` with `submit_on_enter(true)`.
  An `InputEvent::PressEnter { shift: false, .. }` subscription sends the
  prompt, preserving the M3 send path. Shift+Enter inserts a newline.
- Assistant `Content::Text` parts render through `TextView::markdown` with an
  ID derived from the message and part ordinal, so streaming parts update in
  place instead of duplicating. Empty text renders a muted ellipsis.
- Actions (`NewSession`, `ToggleTheme`, `FocusComposer`, `CancelOverlays`) are
  declared in `harness-views` and re-exported. The composition root binds them
  in the `Workspace` key context, binding `cmd-*` and `ctrl-*` separately
  because static keystroke parsing cannot resolve `secondary-`.

## Steps

- [x] Derive `harness-ui` tokens from gpui-component and add `Theme::current`.
- [x] Emit `StreamDisconnected` once per outage from the stream pump.
- [x] Track stream connectivity in `WorkspaceView`; show the reconnect banner.
- [x] Reload transcript and permissions on `StreamConnected`.
- [x] Replace the single-line composer with a multiline textarea.
- [x] Render assistant text as markdown with per-part element IDs.
- [x] Add view actions, sidebar theme toggle, and key bindings.
- [x] Follow the system appearance at startup.
- [x] Update `README.md`, `ARCHITECTURE.md`, and `CONTRIBUTING.md` if needed.

## Test plan

- `cargo test --workspace` — existing unit and sidecar tests must stay green;
  the permission tests in `crates/engine/tests/sidecar.rs` exercise the stream
  reconnect path indirectly.
- `cargo fmt --all -- --check` and
  `cargo clippy --workspace --all-targets -- -D warnings`.
- Smoke run: `RUST_HARNESS_AUTOQUIT_MS=5000 cargo run -p rust-harness --
  /tmp/rh-m6`; assert readiness, `event stream connected`, a clean
  `event stream disconnected` at quit, and no sidecar left listening.
- Manual checks: markdown headings/code, Enter sends and Shift+Enter adds a
  line, each shortcut fires, theme toggle restyles widgets too, banner appears
  when the sidecar is killed and clears on reconnect.

## Risks

- gpui-component API drift (TextView/Textarea/Theme) between minor versions.
- Keyboard shortcuts landing in the wrong context if the root context is
  missing; resolved by `.key_context("Workspace")` on the root element.
- Binding Esc globally can shadow overlay-specific handling; acceptable for
  the MVP because overlays are plain conditionals in the workspace.
- Markdown re-parsing cost while a long message streams; the transcript still
  re-renders per notify, so this stays a known MVP limit.
