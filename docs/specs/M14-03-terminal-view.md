# M14-03 — Terminal dock

| | |
| --- | --- |
| Status | done |
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
| `harness-views` | `runs_group_by_style_and_mark_the_cursor` | styled runs and the cursor cell |
| `harness-views` | `indexed_palette_covers_cube_and_grayscale` | 256-colour palette |
| `harness-core` | `terminal_batch_bounds_events` | ≤ 16 ms / 64 KB |
| `harness-views` | `terminal_resize_is_debounced` | 120 ms |
| `harness-engine` | `terminal_dock_streams_resizes_and_terminates` | the e2e below |

E2E (`tests/M14-03.rs`, gate `RUST_HARNESS_PTY_E2E`): create a terminal, run
`printf '%s%s\n' rea dy` (the marker is split so the shell's echo of the
command cannot satisfy the search), assert the grid contains `ready`; resize
to 120×40 and assert the server confirms it; run
`yes | head -c 10000000`, assert every event is ≤ 64 KiB, the worst event
applies to the grid in under 100 ms, and the bytes arrive without loss;
delete the terminal and assert the process is gone; create a second terminal
and assert quitting the engine leaves no process behind. Durable assertions:
the OS process and a clean app shutdown.

## 5. Acceptance

- [x] 10 MB of output does not freeze the window (measured, recorded). In the
  e2e, 15,000,426 bytes (10 MB of `yes` plus CRLF and echo) arrived as 308
  events, largest 65,536 bytes; applying them to the grid took 3.96 s in total
  (debug, `opt-level = 1`) and 34.6 ms in the worst event. The sidecar's PTY
  pump paces the transfer: a raw WebSocket probe with no app code measured
  the same 10 MB taking 10.14 s, so the e2e asserts the 60 s hang guard and
  the per-event frame bound, not the wall clock.
- [x] Copy/paste and resize work; no dropped keys during a burst. Copy uses
  the visible screen; paste honours bracketed-paste mode; resize is
  debounced (120 ms) and confirmed by the server.
- [x] Detach/reattach resumes at the right cursor. Hiding the dock keeps the
  `PtySession` and its socket, so output keeps accumulating and the viewport
  is still at the bottom on reopen.
- [x] Quit leaves no PTY process (`pgrep` empty). The e2e creates a leftover
  `sleep 300` PTY, shuts the engine down, and asserts the pid is gone.

## 6. Implementation notes

- `PtySession` lives in `crates/views/src/terminal.rs` with `Run`/`row_runs`;
  `terminal_keys.rs` holds the pure key and paste encoders. The grid itself is
  `harness_term::Grid`, unchanged except for `row_from_history` and a scroll
  fix that recycles the row the scrollback cap evicts (without it, 10 MB of
  newlines spent ~10 s in `Grid::feed` in the debug profile).
- Output batching moved to `harness_core::TerminalBatch` so the engine pump
  and its test share one policy.
- The dock renders `PtySession::rows()` (history + live screen) as styled
  runs, always exactly `grid.rows()` lines, and draws the cursor by inverting
  its cell.
- Resize is optimistic locally (`PtySession::resize`) and debounced toward the
  server with a generation counter, so a stale timer cannot undo a newer size.
- Platform+C/V are handled by the view (clipboard) and never reach the PTY;
  every other key is encoded by `terminal_keys::encode_key`.
- The e2e gate is opt-in: without `RUST_HARNESS_PTY_E2E` the test prints why
  it skipped, as with `RUST_HARNESS_SKIP_E2E`.

## 7. Non-goals

Tabs of terminals, splits, search, hyperlinks, images, mouse reporting.

## 8. Risks

Rendering cost and orphan processes. Both have explicit gates above; a failure
blocks the node, not the wave.
