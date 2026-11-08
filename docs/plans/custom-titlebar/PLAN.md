# Custom title bar instead of the OS window chrome

## Goal

Replace the default macOS title bar with an in-app top bar, so the window
chrome matches the application's visual language while preserving native window
behavior (drag, double-click zoom, traffic lights, resize, fullscreen).

## Context

- `crates/app/src/main.rs:26-36` builds `WindowOptions` with
  `..Default::default()`, which yields a normal server-side title bar
  (`WindowOptions::default()` sets `titlebar: Some(TitlebarOptions::default())`
  with `appears_transparent: false`).
- `crates/views/src/workspace.rs:913-924` renders the root as a horizontal
  `div` with sidebar and main panes; there is no top-level row for a title bar.
- The pinned UI stack already provides the needed primitives:
  - `gpui-component` 0.9.0 has a first-party `TitleBar` component
    (`src/title_bar.rs`) with `TitleBar::window_options()`,
    `TitleBar::title_bar_options()`, drag handling, macOS double-click zoom, and
    Linux/Windows window controls.
  - `gpui-pre` 0.3.11 (`WindowOptions`, `TitlebarOptions`,
    `WindowDecorations`) exposes `appears_transparent`,
    `traffic_light_position`, `app_owns_titlebar_drag`,
    `Window::start_window_move`, `Window::titlebar_double_click`,
    `Window::zoom_window`, `Window::minimize_window`, and
    `Window::set_traffic_light_position`.
- `harness-ui::Theme` already derives from the gpui-component theme
  (`crates/ui/src/lib.rs:27-46`), so the title bar can use the same tokens.

Verified in the pinned sources; no new dependency is required.

## Scope

- macOS first: transparent native title bar with the native traffic lights
  retained and repositioned, plus an in-app 34 px top bar.
- Keep the window resizable, minimizable, and user-movable.
- Put app-level context in the bar (project name and/or session title), not a
  second copy of the sidebar.
- Keep keyboard shortcuts and focus behavior unchanged.

## Non-goals

- Reimplementing macOS traffic lights; the native buttons stay.
- A full tab bar, breadcrumb bar, or sidebar redesign.
- Windows support (out of scope for the project).
- Custom window corner radii or a fully borderless window.
- Theming redesign; only tokens needed by the bar are added.

## Design

### 1. Window options

Use the component's canonical options as the base, preserving our bounds:

```rust
use gpui_kit::component::TitleBar;

let options = WindowOptions {
    window_bounds: Some(WindowBounds::centered(size(px(1120.), px(720.)), cx)),
    ..TitleBar::window_options()
};
```

`TitleBar::window_options()` sets:

- `titlebar.appears_transparent = true` → macOS `FullSizeContentView`,
  transparent title bar, hidden title text;
- `traffic_light_position = point(px(9.), px(9.))`;
- `app_owns_titlebar_drag = true` → the app moves the window itself through
  `start_window_move`, so AppKit does not also drag from the title bar or delay
  clicks while disambiguating double clicks.

Keep calling `window.set_window_title("Rust Harness")` for Mission Control,
the Window menu, and accessibility. The title stays hidden visually.

Do not set `WindowDecorations::Client` on macOS: native traffic lights are
retained by design. Linux client-side decorations are a later step.

### 2. Layout

Wrap the existing root in a vertical flex:

```text
div.size_full.flex.flex_col
├── TitleBar                     // fixed 34 px, flex_shrink_0
└── div.flex_1.flex.flex_row
    ├── sidebar                  // existing render_sidebar
    └── main                     // existing render_main
```

The bar spans the full window width and the sidebar starts below it. The
component applies macOS left padding (80 px) so its content never overlaps the
traffic lights.

Title bar content (first iteration):

- left/center: project directory basename, or the app title then the project;
- right: the engine status dot and label (reusing `harness_ui::status_dot`).

Avoid interactive widgets in the drag region in the first iteration. If a
button is added later, verify that pressing it does not initiate a window move
and that it stops event propagation.

### 3. Theme

Start with the `TitleBar` defaults (`title_bar` gradient + `title_bar_border`
from the gpui-component theme), which already track light/dark mode. If the
result clashes with `harness-ui::Theme`, override with `.bg(theme.surface)` and
`.border_color(theme.border)` (the component applies `refine_style` after its
defaults). Add `title_bar`/`title_bar_border` tokens to `harness-ui` only if the
override is repeated.

### 4. Behavior

| Action | Mechanism |
| --- | --- |
| Drag | `TitleBar` calls `window.start_window_move()` after a left-button drag (`gpui-component/src/title_bar.rs:355-375`). |
| Double-click | macOS: `window.titlebar_double_click()`, honoring the system zoom/minimize preference. Linux: `window.zoom_window()`. |
| Traffic lights | Native, repositioned by GPUI to `(9, 9)`; restored around fullscreen. |
| Fullscreen | Component adds leading padding when `window.is_fullscreen()`. |
| Resize | Window style mask is unchanged (`is_resizable: true`). |

### 5. Linux (follow-up step, not required for macOS acceptance)

To replace server decorations too, set
`window_decorations: Some(WindowDecorations::Client)`, call
`window.set_client_inset(TITLE_BAR_HEIGHT)`, and verify that
`window.window_decorations()` reports `Decorations::Client` before relying on
the component's custom controls. X11 without a compositor and some Wayland
compositors may fall back to server decorations; keep that fallback working.

### 6. Packaging

No Info.plist change is expected (`FullSizeContentView` is a runtime style
mask). Re-verify the custom bar in the packaged
`dist/Rust Harness.app` from `make bundle`, not only in `cargo run`, because
window chrome and activation differ between the two.

## Steps

- [x] 1. Switch `WindowOptions` in `crates/app/src/main.rs` to
      `..TitleBar::window_options()`, keeping `window_bounds` and
      `set_window_title`.
- [x] 2. Restructure `WorkspaceView::render` into a column with `TitleBar` above
      the existing sidebar/main row; move the current row into a child `div`.
- [x] 3. Add title bar content (project name and engine status) using
      `harness_ui::Theme`/`status_dot`; keep it non-interactive for now.
- [x] 4. Add `#[gpui::test]` coverage that renders the workspace and asserts the
      title bar element is present and the window options are client-drag
      compatible (no panics, `test_support` id present).
- [ ] 5. Run the manual macOS checklist (see Test plan), including the packaged
      app. Automated checks and both smoke runs are done; interactive items
      (drag, double-click, traffic lights, fullscreen) still need a human.
- [ ] 6. (Optional follow-up) Linux client-side decorations and control
      fallback.
- [x] 7. Update `docs/ARCHITECTURE.md` §6.3 (AppShell) and the README status if
      the visible shell changes; update `STATUS.md` at every step.

## Test plan

Automated:

```sh
make check
cargo test --workspace
```

Unit/UI test: render `WorkspaceView` in a GPUI test context, assert the
`title-bar` test-support id exists, and assert no panic when the title bar
receives a click/drag event (the test platform's `start_window_move` is a
no-op).

Manual (macOS):

- Drag from empty title bar space moves the window; dragging from a button does
  not.
- Double-click the title bar zooms or minimizes according to System Settings →
  Desktop & Dock → "Double-click a window's title bar to".
- Traffic lights are visible, vertically centered at 9 px, and not overlapped by
  content.
- Close, minimize, and zoom buttons work; green button enters fullscreen and
  returns with traffic lights restored.
- Window can still be resized from every edge/corner.
- Light and dark appearance both render correctly.
- Title bar is correct on a secondary display and after display changes.
- Mission Control and the Window menu show "Rust Harness".
- `make bundle` app behaves the same as `cargo run`.

## Risks

| Risk | Mitigation |
| --- | --- |
| Pinned `gpui-pre`/`gpui-component` snapshot API drift | Use only the APIs verified in the pinned sources; upgrade UI kit and title bar together. |
| Traffic lights misaligned across displays/scale | Use the component's `traffic_light_position`; verify on Retina and external displays. |
| AppKit double-handles title bar drag/double-click | Keep `app_owns_titlebar_drag: true`; do not move the bar into the OS drag region. |
| Fullscreen transition leaves traffic lights hidden or mispositioned | GPUI restores frames around fullscreen; test enter/exit explicitly. |
| Interactive widgets inside the drag region | Keep the first iteration non-interactive; if needed, stop propagation and verify drag semantics. |
| Theme mismatch between `harness-ui` and component theme | Bar uses component tokens by default; override with `harness-ui` tokens only if needed. |
| Linux compositor falls back to server decorations | Gate custom controls on `Decorations::Client`; defer Linux to a follow-up. |

## References

- [`docs/ARCHITECTURE.md`](../../ARCHITECTURE.md) §6.3 (views) and §6.4 (UI kit)
- [`AGENTS.md`](../../../AGENTS.md) — architecture and testing rules
- `crates/app/src/main.rs`
- `crates/views/src/workspace.rs`
- `crates/ui/src/lib.rs`
- `gpui-component` 0.9.0: `src/title_bar.rs` (`TitleBar`,
  `TitleBar::window_options`, `TitleBar::title_bar_options`,
  `TITLE_BAR_HEIGHT`)
- `gpui-pre` 0.3.11: `src/platform.rs` (`WindowOptions`, `TitlebarOptions`,
  `WindowDecorations`, `WindowControls`), `src/window.rs`
  (`start_window_move`, `titlebar_double_click`, `zoom_window`,
  `minimize_window`, `set_traffic_light_position`)
