//! The terminal dock's view model: a PTY id, its grid, and the scroll state.
//!
//! Nothing here touches GPUI or IO, so the scroll and batching rules can be
//! tested without a window.

use harness_term::{Attrs, Color, Grid, Row, char_width};

/// How long resize requests settle before reaching the server.
pub const TERMINAL_RESIZE_DEBOUNCE_MS: i64 = 120;

/// One attached PTY.
#[derive(Debug, Clone)]
pub struct PtySession {
    pub id: String,
    pub grid: Grid,
    /// Index of the first visible history row; `scrollback_len` means the
    /// live screen is shown.
    pub viewport_top: usize,
    /// Whether new output keeps the viewport at the bottom.
    pub follow: bool,
}

impl PtySession {
    pub fn new(id: String, cols: u16, rows: u16) -> Self {
        Self {
            id,
            grid: Grid::new(cols, rows),
            viewport_top: 0,
            follow: true,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.grid.feed(bytes);
        if self.follow {
            self.viewport_top = self.grid.scrollback_len();
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.grid.resize(cols, rows);
        self.clamp_viewport();
    }

    /// Scroll toward older output; stops following new output.
    pub fn scroll_up(&mut self, lines: usize) {
        self.follow = false;
        self.viewport_top = self.viewport_top.saturating_sub(lines);
    }

    /// Scroll toward newer output; at the bottom, follow new output again.
    pub fn scroll_down(&mut self, lines: usize) {
        self.viewport_top = (self.viewport_top + lines).min(self.grid.scrollback_len());
        self.follow = self.viewport_top == self.grid.scrollback_len();
    }

    fn clamp_viewport(&mut self) {
        self.viewport_top = self.viewport_top.min(self.grid.scrollback_len());
    }

    /// The rows on screen, top to bottom. Never more than `grid.rows()`.
    pub fn rows(&self) -> impl Iterator<Item = &Row> {
        let top = self.viewport_top.min(self.grid.scrollback_len());
        (0..self.grid.rows() as usize)
            .filter_map(move |offset| self.grid.row_from_history(top + offset))
    }

    /// The cursor cell when the live screen is visible.
    pub fn cursor(&self) -> Option<(u16, u16)> {
        if self.follow && self.grid.cursor_visible() {
            let cursor = self.grid.cursor();
            Some((cursor.x, cursor.y))
        } else {
            None
        }
    }

    /// The visible screen as plain text, for the Copy button.
    pub fn visible_text(&self) -> String {
        self.rows()
            .map(|row| {
                row.iter()
                    .map(|cell| cell.ch)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// One run of cells that share a style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub fg: Color,
    pub bg: Color,
    pub attrs: Attrs,
    pub cursor: bool,
}

/// Group a row into styled runs, skipping the placeholder cell of a wide
/// character. The cursor cell, when in this row, is always its own run.
pub fn row_runs(row: &Row, cursor: Option<u16>) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let mut skip_wide = false;
    for (x, cell) in row.iter().enumerate() {
        if skip_wide {
            skip_wide = false;
            continue;
        }
        let is_cursor = cursor == Some(x as u16);
        let style = (cell.fg, cell.bg, cell.attrs, is_cursor);
        if let Some(last) = runs.last_mut()
            && (last.fg, last.bg, last.attrs, last.cursor) == style
        {
            last.text.push(cell.ch);
        } else {
            runs.push(Run {
                text: cell.ch.to_string(),
                fg: cell.fg,
                bg: cell.bg,
                attrs: cell.attrs,
                cursor: is_cursor,
            });
        }
        skip_wide = char_width(cell.ch) == 2;
    }
    runs
}

/// A pending resize that waits for the user to stop dragging.
#[derive(Debug, Default)]
pub struct TerminalResize {
    pending: Option<(u16, u16)>,
    requested_ms: i64,
}

impl TerminalResize {
    pub fn request(&mut self, now_ms: i64, cols: u16, rows: u16) {
        self.pending = Some((cols, rows));
        self.requested_ms = now_ms;
    }

    pub fn due_ms(&self) -> Option<i64> {
        self.pending
            .map(|_| self.requested_ms + TERMINAL_RESIZE_DEBOUNCE_MS)
    }

    /// Milliseconds until the pending resize is due, if any.
    pub fn wait_ms(&self, now_ms: i64) -> Option<i64> {
        self.due_ms().map(|due| (due - now_ms).max(0))
    }

    /// Take the size once the debounce has elapsed.
    pub fn take_if_due(&mut self, now_ms: i64) -> Option<(u16, u16)> {
        let due = self.due_ms()?;
        if now_ms < due {
            return None;
        }
        self.pending.take()
    }
}

/// The xterm 256-colour palette entry, for rendering runs.
pub fn indexed_rgb(index: u8) -> (u8, u8, u8) {
    const BASE: [(u8, u8, u8); 16] = [
        (30, 30, 30),
        (204, 85, 85),
        (80, 200, 120),
        (220, 180, 80),
        (85, 140, 220),
        (180, 100, 200),
        (80, 190, 200),
        (210, 210, 210),
        (110, 110, 110),
        (240, 110, 110),
        (120, 230, 150),
        (240, 220, 120),
        (120, 170, 250),
        (220, 140, 240),
        (120, 220, 230),
        (245, 245, 245),
    ];
    match index {
        0..=15 => BASE[index as usize],
        16..=231 => {
            let value = index - 16;
            let steps = [0_u8, 95, 135, 175, 215, 255];
            (
                steps[(value / 36) as usize],
                steps[((value % 36) / 6) as usize],
                steps[(value % 6) as usize],
            )
        }
        _ => {
            let level = 8 + (index - 232) * 10;
            (level, level, level)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(cols: u16, rows: u16) -> PtySession {
        PtySession::new("pty_1".into(), cols, rows)
    }

    fn row_text(row: &Row) -> String {
        row.iter()
            .map(|cell| cell.ch)
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn terminal_renders_visible_rows_only() {
        let mut session = session(10, 3);
        for line in 1..=20 {
            session.feed(format!("line{line}\r\n").as_bytes());
        }
        assert!(session.grid.scrollback_len() > 0);
        assert!(session.follow);
        assert_eq!(session.rows().count(), 3);

        let last: Vec<String> = session.rows().map(row_text).collect();
        assert_eq!(last, vec!["line19", "line20", ""]);

        session.scroll_up(2);
        assert_eq!(session.rows().count(), 3);
        let older: Vec<String> = session.rows().map(row_text).collect();
        assert_eq!(older, vec!["line17", "line18", "line19"]);
        assert_eq!(row_runs(&session.grid.visible()[0], None).len(), 1);
    }

    #[test]
    fn terminal_follows_only_at_bottom() {
        let mut session = session(10, 2);
        session.feed(b"one\r\ntwo\r\n");
        assert!(session.follow);

        session.scroll_up(1);
        assert!(!session.follow);
        let before = session.viewport_top;
        session.feed(b"three\r\n");
        assert_eq!(session.viewport_top, before);
        assert!(!session.follow);

        session.scroll_down(100);
        assert!(session.follow);
        session.feed(b"four\r\n");
        assert_eq!(session.viewport_top, session.grid.scrollback_len());
        let text: Vec<String> = session.rows().map(row_text).collect();
        assert_eq!(text.last().map(String::as_str), Some(""));
        assert_eq!(session.cursor(), Some((0, 1)));
    }

    #[test]
    fn terminal_resize_is_debounced() {
        let mut resize = TerminalResize::default();
        resize.request(1_000, 100, 30);
        assert_eq!(resize.due_ms(), Some(1_120));
        assert_eq!(resize.take_if_due(1_119), None);
        assert_eq!(resize.take_if_due(1_120), Some((100, 30)));
        assert_eq!(resize.due_ms(), None);

        resize.request(2_000, 80, 24);
        resize.request(2_050, 90, 25);
        assert_eq!(resize.due_ms(), Some(2_170));
        assert_eq!(resize.take_if_due(2_160), None);
        assert_eq!(resize.take_if_due(2_170), Some((90, 25)));
    }

    #[test]
    fn runs_group_by_style_and_mark_the_cursor() {
        let mut grid = Grid::new(6, 1);
        grid.feed(b"ab\x1b[31mcd\x1b[0m");
        let runs = row_runs(&grid.visible()[0], Some(1));
        assert_eq!(runs.len(), 4);
        assert_eq!(runs[0].text, "a");
        assert!(runs[1].cursor);
        assert_eq!(runs[1].text, "b");
        assert_eq!(runs[2].fg, Color::Indexed(1));
        assert_eq!(runs[2].text, "cd");
        assert_eq!(runs[3].text, "  ");
    }

    #[test]
    fn indexed_palette_covers_cube_and_grayscale() {
        assert_eq!(indexed_rgb(1), (204, 85, 85));
        assert_eq!(indexed_rgb(16), (0, 0, 0));
        assert_eq!(indexed_rgb(231), (255, 255, 255));
        assert_eq!(indexed_rgb(232), (8, 8, 8));
    }
}
