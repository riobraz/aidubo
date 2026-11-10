# M14-02 — VT parser and grid model

| | |
| --- | --- |
| Status | done |
| Wave | 0 |
| Size | M |
| Depends on | — |
| Parallel with | E0, T0 |
| Owns | `crates/term/` (new crate: `harness-term`), `crates/term/src/*.rs` |
| Shared touch | `Cargo.toml` (workspace member) |

## 1. Goal

A pure, IO-free terminal model: feed bytes, get a grid of styled cells with a
cursor and scrollback. It must never panic on arbitrary input, because the
input is a shell's output.

Creating `harness-term` needs an ADR because it is a new crate: see
[ADR 0002](../adr/0002-pure-terminal-crate.md). It has **no dependencies**
(`char_width` handles the ranges the dock needs) and no GPUI.

## 2. Model

```rust
pub struct Grid { pub cols: u16, pub rows: u16, cells: Vec<Cell>, cursor: Cursor,
                  attrs: Attrs, scroll_region: (u16, u16), alt: Option<Box<Grid>>,
                  scrollback: VecDeque<Row>, title: Option<String>, dirty: Vec<bool> }

pub struct Cell { pub ch: char, pub fg: Color, pub bg: Color, pub attrs: CellAttrs }
pub enum Color { Default, Indexed(u8), Rgb(u8, u8, u8) }

impl Grid {
    pub fn new(cols: u16, rows: u16) -> Self;
    pub fn feed(&mut self, bytes: &[u8]);
    pub fn resize(&mut self, cols: u16, rows: u16);
    pub fn visible(&self) -> &[Row];
    pub fn row(&self, index: u16) -> Option<&Row>;
    pub fn take_dirty(&mut self) -> Vec<u16>;
}
```

Parser states: `Ground`, `Escape`, `Csi`, `Osc`, `Charset`. Partial sequences
are kept across `feed` calls, so a chunk boundary never drops a sequence.

Supported: C0 (`\n`, `\r`, `\t`, `\b`), CSI `m` (SGR 0–7, 22–27, 30–37,
40–47, 90–97, 100–107), `A B C D E F G H f`, `J K`, `L M P @`, `S T`, `r`,
`h/l` for `?25`, `?1049`, `?2004`, OSC 0/2 (title), and `\x1b[` with an
unknown final byte → ignored. UTF-8 is decoded incrementally; an incomplete
sequence is buffered.

Limits: scrollback 10,000 rows (ring), 512 columns, 256 rows. Beyond the cap,
drop the oldest row instead of growing.

## 3. Non-goals

Hyperlinks, sixel/kitty images, mouse reporting, ligatures, fonts, rendering.
This crate has no UI and no IO.

## 4. Tests

| Test | Proves |
| --- | --- |
| `vt_prints_and_wraps` | wrap at column |
| `vt_sgr_sets_and_resets_colors` | 16/256/truecolor |
| `vt_cursor_moves_and_clamps` | bounds |
| `vt_erase_display_and_line` | `J`/`K` modes |
| `vt_scroll_region_keeps_rows` | `r` + scroll |
| `vt_alt_screen_round_trip` | `?1049` restores |
| `vt_resize_keeps_top_rows` | resize semantics |
| `vt_unknown_sequence_is_ignored` | no panic |
| `vt_split_utf8_across_feeds` | incremental decode |
| `vt_scrollback_ring_caps_at_10000` | memory bound |

Fixtures in `crates/term/tests/streams/*.bin` are recorded from `script(1)` on
macOS; the tests feed them in 1-byte, 7-byte, and whole-buffer chunks and
assert identical grids.

E2E: none. This crate has no IO by design; the terminal view's e2e covers the
integration.

## 5. Acceptance

- [ ] Feeding random bytes for 1 MB never panics and never exceeds limits.
- [ ] Chunk boundaries never change the final grid.
- [ ] `cargo test -p harness-term` runs without a runtime or a server.
- [ ] `cargo tree -p harness-term` shows no tokio, reqwest, or gpui.

## 6. Risks

VT completeness is a rabbit hole. The supported set is the one terminals use
for an agent's shell output; anything else is ignored, which is safe.
