//! Pure VT parser and terminal grid.
//!
//! No IO, no GPUI, no clocks: feed bytes, read a grid. The input is a shell's
//! output, so every sequence is handled defensively and anything unknown is
//! ignored rather than panicking.
//!
//! Supported: C0 control characters, CSI cursor/erase/insert/delete/scroll
//! operations, SGR colours and attributes, the alternate screen (`?1049`),
//! bracketed paste (`?2004`), and OSC 0/2 window titles. Everything else is
//! consumed and discarded.

use std::collections::VecDeque;

/// Maximum scrollback rows retained.
pub const SCROLLBACK_LIMIT: usize = 10_000;
/// Largest accepted grid width.
pub const MAX_COLS: u16 = 512;
/// Largest accepted grid height.
pub const MAX_ROWS: u16 = 256;

/// A colour as the terminal expresses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// The rendering attributes of a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Attrs {
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub reverse: bool,
    pub fg: Color,
    pub bg: Color,
}

impl Attrs {
    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// One character cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub attrs: Attrs,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: Color::Default,
            bg: Color::Default,
            attrs: Attrs::default(),
        }
    }
}

/// A horizontal cursor position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cursor {
    pub x: u16,
    pub y: u16,
}

/// A row of cells.
pub type Row = Vec<Cell>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    Escape,
    Csi,
    Osc,
}

/// The terminal grid plus its parser state.
#[derive(Debug, Clone)]
pub struct Grid {
    cols: u16,
    rows: u16,
    cells: Vec<Row>,
    cursor: Cursor,
    saved_cursor: (Cursor, Attrs),
    attrs: Attrs,
    scroll_top: u16,
    scroll_bottom: u16,
    wrap_pending: bool,
    state: State,
    params: Vec<u16>,
    param_private: Option<u8>,
    osc: Vec<u8>,
    utf8: Vec<u8>,
    alt: Option<Box<SavedScreen>>,
    title: Option<String>,
    bracketed_paste: bool,
    cursor_visible: bool,
    dirty: Vec<bool>,
    scrollback: VecDeque<Row>,
}

#[derive(Debug, Clone)]
struct SavedScreen {
    cells: Vec<Row>,
    cursor: Cursor,
    attrs: Attrs,
    scroll_top: u16,
    scroll_bottom: u16,
}

impl Grid {
    /// A blank grid with a cursor in the top-left cell.
    pub fn new(cols: u16, rows: u16) -> Self {
        let cols = cols.clamp(1, MAX_COLS);
        let rows = rows.clamp(1, MAX_ROWS);
        Self {
            cols,
            rows,
            cells: vec![vec![Cell::default(); cols as usize]; rows as usize],
            cursor: Cursor::default(),
            saved_cursor: (Cursor::default(), Attrs::default()),
            attrs: Attrs::default(),
            scroll_top: 0,
            scroll_bottom: rows - 1,
            wrap_pending: false,
            state: State::Ground,
            params: Vec::new(),
            param_private: None,
            osc: Vec::new(),
            utf8: Vec::new(),
            alt: None,
            title: None,
            bracketed_paste: false,
            cursor_visible: true,
            dirty: vec![true; rows as usize],
            scrollback: VecDeque::new(),
        }
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn bracketed_paste(&self) -> bool {
        self.bracketed_paste
    }

    pub fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// Rows currently on screen, top to bottom.
    pub fn visible(&self) -> &[Row] {
        &self.cells
    }

    /// Scrollback rows plus the screen, oldest first.
    pub fn row_from_history(&self, index: usize) -> Option<&Row> {
        match index.checked_sub(self.scrollback.len()) {
            Some(screen) => self.cells.get(screen),
            None => self.scrollback.get(index),
        }
    }

    /// One screen row.
    pub fn row(&self, index: u16) -> Option<&Row> {
        self.cells.get(index as usize)
    }

    /// One cell, or `None` when out of bounds.
    pub fn cell(&self, x: u16, y: u16) -> Option<&Cell> {
        self.cells.get(y as usize)?.get(x as usize)
    }

    /// Rows changed since the last call, then clears the markers.
    pub fn take_dirty(&mut self) -> Vec<u16> {
        let mut dirty = Vec::new();
        for (index, flag) in self.dirty.iter_mut().enumerate() {
            if *flag {
                *flag = false;
                dirty.push(index as u16);
            }
        }
        dirty
    }

    /// Resize the screen, keeping the top-left content and clamping the cursor.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let cols = cols.clamp(1, MAX_COLS);
        let rows = rows.clamp(1, MAX_ROWS);
        if cols == self.cols && rows == self.rows {
            return;
        }

        let mut next = vec![vec![Cell::default(); cols as usize]; rows as usize];
        for (y, row) in self.cells.iter().enumerate().take(rows as usize) {
            for (x, cell) in row.iter().enumerate().take(cols as usize) {
                next[y][x] = *cell;
            }
        }
        self.cells = next;
        self.cols = cols;
        self.rows = rows;
        self.cursor.x = self.cursor.x.min(cols - 1);
        self.cursor.y = self.cursor.y.min(rows - 1);
        self.scroll_top = 0;
        self.scroll_bottom = rows - 1;
        self.wrap_pending = false;
        self.dirty = vec![true; rows as usize];
    }

    /// Feed raw bytes. Partial escape sequences and split UTF-8 are preserved
    /// across calls.
    pub fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            match self.state {
                State::Ground => self.ground(byte),
                State::Escape => self.escape(byte),
                State::Csi => self.csi(byte),
                State::Osc => self.osc(byte),
            }
        }
    }

    fn ground(&mut self, byte: u8) {
        match byte {
            0x1b => {
                self.state = State::Escape;
                self.utf8.clear();
            }
            0x0a => self.line_feed(),
            0x0d => {
                self.cursor.x = 0;
                self.wrap_pending = false;
            }
            0x08 => {
                self.cursor.x = self.cursor.x.saturating_sub(1);
                self.wrap_pending = false;
            }
            0x09 => {
                let next = ((self.cursor.x / 8) + 1) * 8;
                self.cursor.x = next.min(self.cols - 1);
                self.wrap_pending = false;
            }
            0x07 => {}
            0x00..=0x1f => {}
            _ => self.byte(byte),
        }
    }

    fn byte(&mut self, byte: u8) {
        if byte < 0x80 {
            self.utf8.clear();
            self.put_char(byte as char);
            return;
        }
        self.utf8.push(byte);
        match std::str::from_utf8(&self.utf8) {
            Ok(text) => {
                let mut chars = text.chars().collect::<Vec<_>>();
                self.utf8.clear();
                for ch in chars.drain(..) {
                    self.put_char(ch);
                }
            }
            Err(error) => {
                if error.error_len().is_some() {
                    // Invalid sequence: drop the offending byte and continue.
                    self.utf8.remove(0);
                }
            }
        }
    }

    fn escape(&mut self, byte: u8) {
        match byte {
            b'[' => {
                self.state = State::Csi;
                self.params.clear();
                self.param_private = None;
            }
            b']' => {
                self.state = State::Osc;
                self.osc.clear();
            }
            b'(' | b')' | b'*' | b'+' => {
                // Charset designation: the next byte names the set.
                self.state = State::Escape;
            }
            b'7' => {
                self.saved_cursor = (self.cursor, self.attrs);
                self.state = State::Ground;
            }
            b'8' => {
                let (cursor, attrs) = self.saved_cursor;
                self.cursor = cursor;
                self.attrs = attrs;
                self.state = State::Ground;
            }
            b'M' => {
                self.reverse_index();
                self.state = State::Ground;
            }
            b'D' => {
                self.line_feed();
                self.state = State::Ground;
            }
            b'c' => {
                self.reset();
                self.state = State::Ground;
            }
            _ => self.state = State::Ground,
        }
    }

    fn csi(&mut self, byte: u8) {
        match byte {
            b'?' => self.param_private = Some(b'?'),
            b'0'..=b'9' => {
                if self.params.is_empty() {
                    self.params.push(0);
                }
                let index = self.params.len() - 1;
                let value = self.params[index] as u32 * 10 + (byte - b'0') as u32;
                self.params[index] = value.min(u16::MAX as u32) as u16;
            }
            b';' => self.params.push(0),
            0x20..=0x2f => {}
            0x40..=0x7e => {
                self.finish_csi(byte);
                self.state = State::Ground;
                self.params.clear();
                self.param_private = None;
            }
            0x1b => {
                self.state = State::Escape;
                self.params.clear();
                self.param_private = None;
            }
            _ => {}
        }
    }

    fn osc(&mut self, byte: u8) {
        match byte {
            0x07 => {
                self.finish_osc();
                self.state = State::Ground;
            }
            0x1b => {
                // ST is ESC \ ; finish on the ESC and swallow the backslash.
                self.finish_osc();
                self.state = State::Escape;
            }
            _ => {
                if self.osc.len() < 4096 {
                    self.osc.push(byte);
                }
            }
        }
    }

    fn finish_osc(&mut self) {
        let text = String::from_utf8_lossy(&self.osc).into_owned();
        self.osc.clear();
        let mut parts = text.splitn(2, ';');
        let kind = parts.next().unwrap_or_default();
        let value = parts.next().unwrap_or_default();
        if matches!(kind, "0" | "2") {
            self.title = Some(value.to_string());
        }
    }

    fn param(&self, index: usize, default: u16) -> u16 {
        match self.params.get(index) {
            Some(0) | None => default,
            Some(value) => *value,
        }
    }

    fn finish_csi(&mut self, final_byte: u8) {
        match final_byte {
            b'm' => self.sgr(),
            b'A' => self.cursor.y = self.cursor.y.saturating_sub(self.param(0, 1)),
            b'B' => {
                self.cursor.y = (self.cursor.y + self.param(0, 1)).min(self.scroll_bottom);
            }
            b'C' => {
                self.cursor.x = (self.cursor.x + self.param(0, 1)).min(self.cols - 1);
                self.wrap_pending = false;
            }
            b'D' => {
                self.cursor.x = self.cursor.x.saturating_sub(self.param(0, 1));
                self.wrap_pending = false;
            }
            b'E' => {
                let count = self.param(0, 1);
                self.cursor.x = 0;
                self.cursor.y = (self.cursor.y + count).min(self.scroll_bottom);
            }
            b'F' => {
                let count = self.param(0, 1);
                self.cursor.x = 0;
                self.cursor.y = self.cursor.y.saturating_sub(count);
            }
            b'G' => {
                self.cursor.x = self.param(0, 1).saturating_sub(1).min(self.cols - 1);
            }
            b'H' | b'f' => {
                let row = self.param(0, 1).saturating_sub(1);
                let col = self.param(1, 1).saturating_sub(1);
                self.cursor.y = row.min(self.rows - 1);
                self.cursor.x = col.min(self.cols - 1);
                self.wrap_pending = false;
            }
            b'J' => self.erase_display(self.param(0, 0)),
            b'K' => self.erase_line(self.param(0, 0)),
            b'L' => self.insert_lines(self.param(0, 1)),
            b'M' => self.delete_lines(self.param(0, 1)),
            b'P' => self.delete_chars(self.param(0, 1)),
            b'@' => self.insert_chars(self.param(0, 1)),
            b'S' => {
                for _ in 0..self.param(0, 1) {
                    self.scroll_up();
                }
            }
            b'T' => {
                for _ in 0..self.param(0, 1) {
                    self.scroll_down();
                }
            }
            b'r' => {
                let top = self.param(0, 1).saturating_sub(1);
                let bottom = self.param(1, self.rows).saturating_sub(1);
                if top < bottom && bottom < self.rows {
                    self.scroll_top = top;
                    self.scroll_bottom = bottom;
                    self.cursor.x = 0;
                    self.cursor.y = top;
                }
            }
            b'h' | b'l' => self.set_mode(final_byte == b'h'),
            b's' => self.saved_cursor = (self.cursor, self.attrs),
            b'u' => {
                let (cursor, attrs) = self.saved_cursor;
                self.cursor = cursor;
                self.attrs = attrs;
            }
            _ => {}
        }
    }

    fn set_mode(&mut self, enabled: bool) {
        if self.param_private != Some(b'?') {
            return;
        }
        for index in 0..self.params.len() {
            match self.params[index] {
                25 => self.cursor_visible = enabled,
                2004 => self.bracketed_paste = enabled,
                1049 => {
                    if enabled {
                        self.enter_alt();
                    } else {
                        self.exit_alt();
                    }
                }
                _ => {}
            }
        }
    }

    fn enter_alt(&mut self) {
        if self.alt.is_some() {
            return;
        }
        let saved = SavedScreen {
            cells: self.cells.clone(),
            cursor: self.cursor,
            attrs: self.attrs,
            scroll_top: self.scroll_top,
            scroll_bottom: self.scroll_bottom,
        };
        self.alt = Some(Box::new(saved));
        self.cells = vec![vec![Cell::default(); self.cols as usize]; self.rows as usize];
        self.cursor = Cursor::default();
        self.scroll_top = 0;
        self.scroll_bottom = self.rows - 1;
        self.wrap_pending = false;
        self.dirty = vec![true; self.rows as usize];
    }

    fn exit_alt(&mut self) {
        let Some(saved) = self.alt.take() else {
            return;
        };
        self.cells = saved.cells;
        self.cursor = saved.cursor;
        self.attrs = saved.attrs;
        self.scroll_top = saved.scroll_top;
        self.scroll_bottom = saved.scroll_bottom;
        self.wrap_pending = false;
        self.dirty = vec![true; self.rows as usize];
    }

    fn sgr(&mut self) {
        if self.params.is_empty() {
            self.attrs.reset();
            return;
        }
        let mut index = 0;
        while index < self.params.len() {
            let code = self.params[index];
            match code {
                0 => self.attrs.reset(),
                1 => self.attrs.bold = true,
                2 => self.attrs.dim = true,
                3 => self.attrs.italic = true,
                4 => self.attrs.underline = true,
                7 => self.attrs.reverse = true,
                22 => {
                    self.attrs.bold = false;
                    self.attrs.dim = false;
                }
                23 => self.attrs.italic = false,
                24 => self.attrs.underline = false,
                27 => self.attrs.reverse = false,
                30..=37 => self.set_fg(Color::Indexed((code - 30) as u8)),
                39 => self.set_fg(Color::Default),
                40..=47 => self.set_bg(Color::Indexed((code - 40) as u8)),
                49 => self.set_bg(Color::Default),
                90..=97 => self.set_fg(Color::Indexed((code - 90 + 8) as u8)),
                100..=107 => self.set_bg(Color::Indexed((code - 100 + 8) as u8)),
                38 | 48 => {
                    let foreground = code == 38;
                    if let Some((color, consumed)) = self.extended_color(index) {
                        if foreground {
                            self.set_fg(color);
                        } else {
                            self.set_bg(color);
                        }
                        index += consumed;
                    }
                }
                _ => {}
            }
            index += 1;
        }
    }

    fn extended_color(&self, index: usize) -> Option<(Color, usize)> {
        match self.params.get(index + 1).copied()? {
            5 => {
                let value = *self.params.get(index + 2)?;
                Some((Color::Indexed(value as u8), 2))
            }
            2 => {
                let r = *self.params.get(index + 2)?;
                let g = *self.params.get(index + 3)?;
                let b = *self.params.get(index + 4)?;
                Some((Color::Rgb(r as u8, g as u8, b as u8), 4))
            }
            _ => None,
        }
    }

    fn set_fg(&mut self, color: Color) {
        self.attrs.fg = color;
    }

    fn set_bg(&mut self, color: Color) {
        self.attrs.bg = color;
    }

    fn put_char(&mut self, ch: char) {
        let width = char_width(ch);
        if width == 0 {
            return;
        }
        if self.wrap_pending || (width == 2 && self.cursor.x + 1 >= self.cols) {
            self.cursor.x = 0;
            self.line_feed();
        }
        if self.cursor.y >= self.rows {
            self.cursor.y = self.rows - 1;
        }
        let y = self.cursor.y as usize;
        let x = self.cursor.x as usize;
        let attrs = self.attrs;
        let cell = Cell {
            ch,
            fg: attrs.fg,
            bg: attrs.bg,
            attrs,
        };
        if let Some(row) = self.cells.get_mut(y) {
            if let Some(slot) = row.get_mut(x) {
                *slot = cell;
            }
            if width == 2
                && let Some(slot) = row.get_mut(x + 1)
            {
                *slot = Cell { ch: ' ', ..cell };
            }
            if let Some(flag) = self.dirty.get_mut(y) {
                *flag = true;
            }
        }

        let advance = self.cursor.x as usize + width;
        if advance >= self.cols as usize {
            self.cursor.x = self.cols - 1;
            self.wrap_pending = true;
        } else {
            self.cursor.x = advance as u16;
        }
    }

    fn line_feed(&mut self) {
        self.wrap_pending = false;
        if self.cursor.y == self.scroll_bottom {
            self.scroll_up();
        } else if self.cursor.y + 1 < self.rows {
            self.cursor.y += 1;
        } else {
            self.cursor.y = self.rows - 1;
        }
    }

    fn reverse_index(&mut self) {
        if self.cursor.y == self.scroll_top {
            self.scroll_down();
        } else {
            self.cursor.y = self.cursor.y.saturating_sub(1);
        }
    }

    fn scroll_up(&mut self) {
        let top = self.scroll_top as usize;
        let bottom = self.scroll_bottom as usize;
        if top >= bottom || bottom >= self.rows as usize {
            return;
        }
        if top == 0 && self.scroll_bottom == self.rows - 1 && self.alt.is_none() {
            let removed = self.cells.remove(0);
            self.scrollback.push_back(removed);
            // Recycle the row the scrollback cap drops instead of allocating a
            // fresh one; a full-screen burst scrolls once per line.
            let recycled = if self.scrollback.len() > SCROLLBACK_LIMIT {
                self.scrollback.pop_front()
            } else {
                None
            };
            self.cells.push(self.blank_row(recycled));
        } else {
            self.cells.remove(top);
            self.cells
                .insert(bottom, vec![Cell::default(); self.cols as usize]);
        }
        for flag in self.dirty.iter_mut() {
            *flag = true;
        }
    }

    fn blank_row(&self, recycled: Option<Row>) -> Row {
        match recycled {
            Some(mut row) => {
                row.resize(self.cols as usize, Cell::default());
                row.fill(Cell::default());
                row
            }
            None => vec![Cell::default(); self.cols as usize],
        }
    }

    fn scroll_down(&mut self) {
        let top = self.scroll_top as usize;
        let bottom = self.scroll_bottom as usize;
        if top >= bottom || bottom >= self.rows as usize {
            return;
        }
        self.cells.remove(bottom);
        self.cells
            .insert(top, vec![Cell::default(); self.cols as usize]);
        for flag in self.dirty.iter_mut() {
            *flag = true;
        }
    }

    fn erase_line(&mut self, mode: u16) {
        let y = self.cursor.y as usize;
        let x = self.cursor.x as usize;
        let cols = self.cols as usize;
        let Some(row) = self.cells.get_mut(y) else {
            return;
        };
        let (start, end) = match mode {
            0 => (x, cols),
            1 => (0, (x + 1).min(cols)),
            2 => (0, cols),
            _ => return,
        };
        for slot in &mut row[start..end] {
            *slot = Cell::default();
        }
        self.dirty[y] = true;
    }

    fn erase_display(&mut self, mode: u16) {
        match mode {
            0 => {
                self.erase_line(0);
                for y in (self.cursor.y as usize + 1)..self.rows as usize {
                    self.cells[y] = vec![Cell::default(); self.cols as usize];
                    self.dirty[y] = true;
                }
            }
            1 => {
                self.erase_line(1);
                for y in 0..self.cursor.y as usize {
                    self.cells[y] = vec![Cell::default(); self.cols as usize];
                    self.dirty[y] = true;
                }
            }
            2 | 3 => {
                for y in 0..self.rows as usize {
                    self.cells[y] = vec![Cell::default(); self.cols as usize];
                    self.dirty[y] = true;
                }
                if mode == 3 {
                    self.scrollback.clear();
                }
            }
            _ => {}
        }
    }

    fn insert_lines(&mut self, count: u16) {
        let top = self.cursor.y as usize;
        let bottom = self.scroll_bottom as usize;
        for _ in 0..count {
            if top > bottom || bottom >= self.rows as usize {
                break;
            }
            self.cells.remove(bottom);
            self.cells
                .insert(top, vec![Cell::default(); self.cols as usize]);
            for flag in self.dirty.iter_mut() {
                *flag = true;
            }
        }
    }

    fn delete_lines(&mut self, count: u16) {
        let top = self.cursor.y as usize;
        let bottom = self.scroll_bottom as usize;
        for _ in 0..count {
            if top > bottom || bottom >= self.rows as usize {
                break;
            }
            self.cells.remove(top);
            self.cells
                .insert(bottom, vec![Cell::default(); self.cols as usize]);
            for flag in self.dirty.iter_mut() {
                *flag = true;
            }
        }
    }

    fn insert_chars(&mut self, count: u16) {
        let y = self.cursor.y as usize;
        let x = self.cursor.x as usize;
        let Some(row) = self.cells.get_mut(y) else {
            return;
        };
        let count = (count as usize).min(row.len().saturating_sub(x));
        for _ in 0..count {
            row.insert(x, Cell::default());
            row.truncate(self.cols as usize);
        }
        self.dirty[y] = true;
    }

    fn delete_chars(&mut self, count: u16) {
        let y = self.cursor.y as usize;
        let x = self.cursor.x as usize;
        let Some(row) = self.cells.get_mut(y) else {
            return;
        };
        let count = (count as usize).min(row.len().saturating_sub(x));
        for _ in 0..count {
            row.remove(x);
            row.push(Cell::default());
        }
        self.dirty[y] = true;
    }

    fn reset(&mut self) {
        self.attrs.reset();
        self.cursor = Cursor::default();
        self.scroll_top = 0;
        self.scroll_bottom = self.rows - 1;
        self.wrap_pending = false;
        self.bracketed_paste = false;
        self.cursor_visible = true;
        self.cells = vec![vec![Cell::default(); self.cols as usize]; self.rows as usize];
        self.dirty = vec![true; self.rows as usize];
        self.state = State::Ground;
        self.params.clear();
        self.param_private = None;
        self.osc.clear();
        self.utf8.clear();
    }
}

/// Display width of a character in cells.
///
/// Deliberately small: combining marks take no cell, the CJK and Hangul blocks
/// take two, everything else takes one. A terminal that needs full Unicode
/// width tables can replace this function without touching the parser.
pub fn char_width(ch: char) -> usize {
    let code = ch as u32;
    if code == 0 {
        return 0;
    }
    if matches!(code,
        0x0300..=0x036f | 0x1ab0..=0x1aff | 0x1dc0..=0x1dff |
        0x20d0..=0x20ff | 0xfe20..=0xfe2f
    ) {
        return 0;
    }
    if matches!(code,
        0x1100..=0x115f | 0x2e80..=0x303e | 0x3041..=0x33ff |
        0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xa000..=0xa4cf |
        0xac00..=0xd7a3 | 0xf900..=0xfaff | 0xfe30..=0xfe6f |
        0xff00..=0xff60 | 0xffe0..=0xffe6 | 0x1f300..=0x1f64f |
        0x1f900..=0x1f9ff | 0x20000..=0x3fffd
    ) {
        return 2;
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(grid: &Grid) -> Vec<String> {
        grid.visible()
            .iter()
            .map(|row| row.iter().map(|cell| cell.ch).collect::<String>())
            .collect()
    }

    fn trimmed(grid: &Grid) -> Vec<String> {
        text(grid)
            .into_iter()
            .map(|line| line.trim_end().to_string())
            .collect()
    }

    #[test]
    fn vt_prints_and_wraps() {
        let mut grid = Grid::new(5, 3);
        grid.feed(b"abcdef");
        assert_eq!(trimmed(&grid), vec!["abcde", "f", ""]);
        assert_eq!(grid.cursor(), Cursor { x: 1, y: 1 });
    }

    #[test]
    fn vt_sgr_sets_and_resets_colors() {
        let mut grid = Grid::new(10, 2);
        grid.feed(b"\x1b[31mR\x1b[0mN");
        assert_eq!(grid.cell(0, 0).unwrap().fg, Color::Indexed(1));
        assert_eq!(grid.cell(1, 0).unwrap().fg, Color::Default);

        grid.feed(b"\x1b[38;5;200mX");
        assert_eq!(grid.cell(2, 0).unwrap().fg, Color::Indexed(200));

        grid.feed(b"\x1b[38;2;1;2;3mY");
        assert_eq!(grid.cell(3, 0).unwrap().fg, Color::Rgb(1, 2, 3));

        grid.feed(b"\x1b[1;4mZ");
        let cell = grid.cell(4, 0).unwrap();
        assert!(cell.attrs.bold);
        assert!(cell.attrs.underline);
    }

    #[test]
    fn vt_cursor_moves_and_clamps() {
        let mut grid = Grid::new(4, 3);
        grid.feed(b"\x1b[99;99H");
        assert_eq!(grid.cursor(), Cursor { x: 3, y: 2 });
        grid.feed(b"\x1b[1;1H");
        assert_eq!(grid.cursor(), Cursor { x: 0, y: 0 });
        grid.feed(b"\x1b[2A");
        assert_eq!(grid.cursor().y, 0);
        grid.feed(b"\x1b[9C");
        assert_eq!(grid.cursor().x, 3);
        grid.feed(b"\x1b[9D");
        assert_eq!(grid.cursor().x, 0);
    }

    #[test]
    fn vt_erase_display_and_line() {
        let mut grid = Grid::new(4, 2);
        grid.feed(b"abcd\r\nefgh");
        grid.feed(b"\x1b[1;3H\x1b[K");
        assert_eq!(trimmed(&grid), vec!["ab", "efgh"]);
        grid.feed(b"\x1b[2J");
        assert_eq!(trimmed(&grid), vec!["", ""]);
    }

    #[test]
    fn vt_scroll_region_keeps_rows() {
        let mut grid = Grid::new(4, 4);
        grid.feed(b"1\r\n2\r\n3\r\n4");
        grid.feed(b"\x1b[2;3r");
        grid.feed(b"\x1b[3;1H\nX");
        assert_eq!(trimmed(&grid), vec!["1", "3", "X", "4"]);
    }

    #[test]
    fn vt_alt_screen_round_trip() {
        let mut grid = Grid::new(6, 2);
        grid.feed(b"main");
        grid.feed(b"\x1b[?1049h");
        assert_eq!(trimmed(&grid), vec!["", ""]);
        grid.feed(b"alt");
        assert_eq!(trimmed(&grid)[0], "alt");
        grid.feed(b"\x1b[?1049l");
        assert_eq!(trimmed(&grid)[0], "main");
    }

    #[test]
    fn vt_resize_keeps_top_rows() {
        let mut grid = Grid::new(4, 3);
        grid.feed(b"a\r\nb\r\nc");
        grid.resize(2, 2);
        assert_eq!(trimmed(&grid), vec!["a", "b"]);
        grid.resize(6, 3);
        assert_eq!(trimmed(&grid), vec!["a", "b", ""]);
    }

    #[test]
    fn vt_unknown_sequence_is_ignored() {
        let mut grid = Grid::new(4, 2);
        grid.feed(b"a\x1b[999zb\x1b]0;title\x07c");
        assert_eq!(trimmed(&grid)[0], "abc");
        assert_eq!(grid.title(), Some("title"));
    }

    #[test]
    fn vt_split_utf8_across_feeds() {
        let bytes = "ação".as_bytes();
        let mut one = Grid::new(10, 1);
        one.feed(bytes);
        let mut many = Grid::new(10, 1);
        for byte in bytes {
            many.feed(&[*byte]);
        }
        assert_eq!(trimmed(&one), trimmed(&many));
        assert_eq!(trimmed(&one)[0], "ação");
    }

    #[test]
    fn vt_scrollback_ring_caps() {
        let mut grid = Grid::new(4, 2);
        for _ in 0..(SCROLLBACK_LIMIT + 5) {
            grid.feed(b"x\r\n");
        }
        assert_eq!(grid.scrollback_len(), SCROLLBACK_LIMIT);
    }

    #[test]
    fn vt_control_characters() {
        let mut grid = Grid::new(8, 2);
        grid.feed(b"ab\t");
        assert_eq!(grid.cursor().x, 7);
        grid.feed(b"\x08d");
        assert_eq!(grid.cell(6, 0).unwrap().ch, 'd');
        grid.feed(b"\r\nX");
        assert_eq!(grid.cell(0, 1).unwrap().ch, 'X');
    }

    #[test]
    fn vt_bracketed_paste_and_cursor_visibility() {
        let mut grid = Grid::new(4, 1);
        assert!(!grid.bracketed_paste());
        grid.feed(b"\x1b[?2004h\x1b[?25l");
        assert!(grid.bracketed_paste());
        assert!(!grid.cursor_visible());
        grid.feed(b"\x1b[?2004l\x1b[?25h");
        assert!(!grid.bracketed_paste());
        assert!(grid.cursor_visible());
    }

    #[test]
    fn vt_wide_chars_take_two_cells() {
        let mut grid = Grid::new(4, 1);
        grid.feed("日本".as_bytes());
        assert_eq!(grid.cell(0, 0).unwrap().ch, '日');
        assert_eq!(grid.cell(2, 0).unwrap().ch, '本');
        assert_eq!(grid.cursor().x, 3);
    }

    #[test]
    fn vt_feed_is_bounded_on_random_input() {
        let mut grid = Grid::new(80, 24);
        let bytes: Vec<u8> = (0..200_000u32).map(|n| (n % 256) as u8).collect();
        for chunk in bytes.chunks(97) {
            grid.feed(chunk);
        }
        assert_eq!(grid.cols(), 80);
        assert_eq!(grid.rows(), 24);
        assert!(grid.scrollback_len() <= SCROLLBACK_LIMIT);
    }
}
