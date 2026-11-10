# M14-03 — Terminal dock

| | |
| --- | --- |
| Status | blocked (M14-01, M14-02) |
| Wave | 2 |
| Size | L |
| Depends on | M14-01, M14-02 |
| Parallel with | M10-03, M11-02, M12-01 |
| Owns | `crates/views/src/terminal.rs`, `crates/views/src/terminal_keys.rs` |
| Shared touch | `views/src/workspace.rs`, `engine/src/{lib,supervisor}.rs` |

## 1. Goal

A dockable terminal in the project directory: type, see output, resize,
scroll back, copy and paste, detach and reattach, and see the exit code.
Closing the app or the dock terminates the PTY it created.

## 2. Design

- The dock owns `PtySession { id, grid: harness_term::Grid, viewport_top,
  follow: bool }`.
- `PtyOutput` events are applied to the grid on the UI thread and coalesced
  into one `cx.notify()` per frame.
- Render only visible rows: `grid.visible()` → styled runs → elements. A
  terminal is a text grid; never parse markdown.
- Highlight/cursor: draw the cursor cell with an inverted background.
- Resize: the dock sends `ResizePty` with debounce 120 ms; the local grid is
  resized optimistically, the server confirms by no-op.
- Scrollback: wheel scrolls; new output follows only when `follow` is true
  (set when the user is at the bottom).
- Detach/attach: closing the dock hides it but keeps the PTY; reopening
  reattaches with the last cursor. Quitting the app deletes every PTY the app
  created.

## 3. Input encoding (`terminal_keys.rs`, pure)

| Key | Bytes |
| --- | --- |
| printable | UTF-8 |
| Enter | `\r` |
| Backspace | `\x7f` |
| Tab / Shift+Tab | `\t` / `\x1b[Z` |
| Esc | `\x1b` |
| Up/Down/Right/Left | `\x1b[A/B/C/D` |
| Home/End | `\x1b[H` / `\x1b[F` |
| PgUp/PgDn | `\x1b[5~` / `\x1b[6~` |
| Ctrl+letter | `letter & 0x1f` |
| Alt+key | `\x1b` + key |
| Paste | bracketed paste when enabled (`\x1b[200~…\x1b[201~`), else raw |

IME: composition is committed as UTF-8 on `Enter`/space; no key event is sent
while composing.

## 4. Tests

| Crate | Test | Proves |
| --- | --- | --- |
| `harness-views` | `keys_encode_control_and_arrows` | table above |
| `harness-views` | `keys_paste_uses_bracketed_mode` | paste guard |
| `harness-views` | `terminal_renders_visible_rows_only` | element count |
| `harness-views` | `terminal_follows_only_at_bottom` | scroll rule |
| `harness-core` | `terminal_batch_bounds_events` | ≤ 16 ms / 64 KB |
| `harness-views` | `terminal_resize_is_debounced` | 120 ms |

E2E (`tests/M14-03.rs`, gate `RUST_HARNESS_PTY_E2E`): create a terminal, run
`printf 'ready\n'`, assert the grid contains `ready`; resize to 120×40 and
assert no error; run `yes | head -c 10000000`, assert the app receives it in
batched events and the test drains it within 5 s; delete the terminal and
assert the process is gone. Durable assertion: the OS process and a clean
app shutdown.

## 5. Acceptance

- [ ] 10 MB of output does not freeze the window (measured, recorded).
- [ ] Copy/paste and resize work; no dropped keys during a burst.
- [ ] Detach/reattach resumes at the right cursor.
- [ ] Quit leaves no PTY process (`pgrep` empty).

## 6. Non-goals

Tabs of terminals, splits, search, hyperlinks, images, mouse reporting.

## 7. Risks

Rendering cost and orphan processes. Both have explicit gates above; a failure
blocks the node, not the wave.
