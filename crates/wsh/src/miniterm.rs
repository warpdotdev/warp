use vte::{Params, Perform};

use crate::cell::{Cell, CellAttr, CellFlags, Color};

pub struct MiniTerm {
    grid: Vec<Vec<Cell>>,
    cursor_row: usize,
    cursor_col: usize,
    cols: usize,
    rows: usize,
    current_attr: CellAttr,
    saved_cursor: Option<(usize, usize)>,
    scroll_top: usize,
    scroll_bottom: usize,
    scrolled_out: Vec<Vec<Cell>>,
    alt_grid: Option<Vec<Vec<Cell>>>,
    parser: Option<vte::Parser>,
}

impl MiniTerm {
    pub fn new(cols: u16, rows: u16) -> Self {
        let cols = cols as usize;
        let rows = rows as usize;
        Self {
            grid: blank_grid(cols, rows),
            cursor_row: 0,
            cursor_col: 0,
            cols,
            rows,
            current_attr: CellAttr::default(),
            saved_cursor: None,
            scroll_top: 0,
            scroll_bottom: rows,
            scrolled_out: Vec::new(),
            alt_grid: None,
            parser: Some(vte::Parser::new()),
        }
    }

    pub fn process_bytes(&mut self, bytes: &[u8]) {
        let mut parser = self.parser.take().unwrap_or_default();
        for &byte in bytes {
            parser.advance(self, byte);
        }
        self.parser = Some(parser);
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let new_cols = cols as usize;
        let new_rows = rows as usize;
        let mut new_grid = blank_grid(new_cols, new_rows);
        for (r, row) in self.grid.iter().enumerate() {
            if r >= new_rows {
                break;
            }
            for (c, cell) in row.iter().enumerate() {
                if c >= new_cols {
                    break;
                }
                new_grid[r][c] = *cell;
            }
        }
        self.grid = new_grid;
        self.cols = new_cols;
        self.rows = new_rows;
        self.cursor_row = self.cursor_row.min(new_rows.saturating_sub(1));
        self.cursor_col = self.cursor_col.min(new_cols.saturating_sub(1));
        self.scroll_top = 0;
        self.scroll_bottom = new_rows;
    }

    pub fn grid(&self) -> &[Vec<Cell>] {
        &self.grid
    }

    pub fn cursor_pos(&self) -> (usize, usize) {
        (self.cursor_row, self.cursor_col)
    }

    pub fn take_scrolled_out(&mut self) -> Vec<Vec<Cell>> {
        std::mem::take(&mut self.scrolled_out)
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn reset(&mut self) {
        self.grid = blank_grid(self.cols, self.rows);
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.current_attr = CellAttr::default();
        self.saved_cursor = None;
        self.scroll_top = 0;
        self.scroll_bottom = self.rows;
        self.scrolled_out.clear();
    }

    // -- scroll helpers --

    fn scroll_up(&mut self, n: usize) {
        let top = self.scroll_top;
        let bot = self.scroll_bottom;
        let n = n.min(bot - top);
        if n == 0 {
            return;
        }
        if top == 0 {
            self.scrolled_out
                .extend(self.grid[..n].iter().cloned());
        }
        for i in top..bot - n {
            self.grid[i] = self.grid[i + n].clone();
        }
        for i in bot - n..bot {
            self.grid[i] = blank_row(self.cols);
        }
    }

    fn scroll_down(&mut self, n: usize) {
        let top = self.scroll_top;
        let bot = self.scroll_bottom;
        let n = n.min(bot - top);
        if n == 0 {
            return;
        }
        for i in (top + n..bot).rev() {
            self.grid[i] = self.grid[i - n].clone();
        }
        for i in top..top + n {
            self.grid[i] = blank_row(self.cols);
        }
    }

    fn advance_cursor_down(&mut self) {
        if self.cursor_row == self.scroll_bottom - 1 {
            self.scroll_up(1);
        } else if self.cursor_row < self.rows - 1 {
            self.cursor_row += 1;
        }
    }
}

fn blank_row(cols: usize) -> Vec<Cell> {
    vec![Cell::default(); cols]
}

fn blank_grid(cols: usize, rows: usize) -> Vec<Vec<Cell>> {
    vec![blank_row(cols); rows]
}

// -- SGR helpers --

fn parse_sgr(params: &Params, attr: &mut CellAttr) {
    let collected: Vec<&[u16]> = params.iter().collect();
    if collected.is_empty() {
        attr.reset();
        return;
    }
    let mut i = 0;
    while i < collected.len() {
        let p = collected[i][0];
        match p {
            0 => *attr = CellAttr::default(),
            1 => attr.flags |= CellFlags::BOLD,
            2 => attr.flags |= CellFlags::DIM,
            3 => attr.flags |= CellFlags::ITALIC,
            4 => attr.flags |= CellFlags::UNDERLINE,
            7 => attr.flags |= CellFlags::INVERSE,
            8 => attr.flags |= CellFlags::HIDDEN,
            22 => attr.flags &= !(CellFlags::BOLD | CellFlags::DIM),
            23 => attr.flags &= !CellFlags::ITALIC,
            24 => attr.flags &= !CellFlags::UNDERLINE,
            27 => attr.flags &= !CellFlags::INVERSE,
            28 => attr.flags &= !CellFlags::HIDDEN,
            30..=37 => attr.fg = Color::Indexed((p - 30) as u8),
            38 => {
                i += 1;
                parse_extended_color(&collected, &mut i, &mut attr.fg);
                continue;
            }
            39 => attr.fg = Color::Default,
            40..=47 => attr.bg = Color::Indexed((p - 40) as u8),
            48 => {
                i += 1;
                parse_extended_color(&collected, &mut i, &mut attr.bg);
                continue;
            }
            49 => attr.bg = Color::Default,
            90..=97 => attr.fg = Color::Indexed((p - 90 + 8) as u8),
            100..=107 => attr.bg = Color::Indexed((p - 100 + 8) as u8),
            _ => {}
        }
        i += 1;
    }
}

fn parse_extended_color(params: &[&[u16]], i: &mut usize, color: &mut Color) {
    if *i >= params.len() {
        return;
    }
    let kind = params[*i][0];
    match kind {
        5 => {
            *i += 1;
            if *i < params.len() {
                *color = Color::Indexed(params[*i][0] as u8);
                *i += 1;
            }
        }
        2 => {
            if *i + 3 < params.len() {
                let r = params[*i + 1][0] as u8;
                let g = params[*i + 2][0] as u8;
                let b = params[*i + 3][0] as u8;
                *color = Color::Rgb(r, g, b);
                *i += 4;
            } else {
                *i += 1;
            }
        }
        _ => {
            *i += 1;
        }
    }
}

impl CellAttr {
    fn reset(&mut self) {
        *self = Self::default();
    }
}

// -- vte::Perform --

impl Perform for MiniTerm {
    fn print(&mut self, ch: char) {
        // Deferred wrap: only wrap when a new character needs to be placed.
        if self.cursor_col >= self.cols {
            self.cursor_col = 0;
            self.advance_cursor_down();
        }
        self.grid[self.cursor_row][self.cursor_col] = Cell::with_char(ch, self.current_attr);
        self.cursor_col += 1;
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x0d => self.cursor_col = 0,
            0x0a => self.advance_cursor_down(),
            0x08 => {
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                }
            }
            0x09 => {
                self.cursor_col = ((self.cursor_col / 8) + 1) * 8;
                if self.cursor_col >= self.cols {
                    self.cursor_col = self.cols.saturating_sub(1);
                }
            }
            0x07 => {} // BEL
            _ => {}
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &Params,
        intermediates: &[u8],
        _has_ignored_intermediates: bool,
        action: char,
    ) {
        let mut params_iter = params.iter();
        let mut next_param_or = |default: u16| -> u16 {
            params_iter
                .next()
                .map(|p| p[0])
                .filter(|&p| p != 0)
                .unwrap_or(default)
        };

        match (action, intermediates.first()) {
            ('m', None) => parse_sgr(params, &mut self.current_attr),

            ('H', None) | ('f', None) => {
                let row = next_param_or(1) as usize;
                let col = next_param_or(1) as usize;
                self.cursor_row = row.saturating_sub(1).min(self.rows.saturating_sub(1));
                self.cursor_col = col.saturating_sub(1).min(self.cols.saturating_sub(1));
            }

            ('A', None) => {
                let n = next_param_or(1) as usize;
                self.cursor_row = self.cursor_row.saturating_sub(n).max(self.scroll_top);
            }
            ('B', None) => {
                let n = next_param_or(1) as usize;
                self.cursor_row = (self.cursor_row + n).min(self.scroll_bottom - 1);
            }
            ('C', None) => {
                let n = next_param_or(1) as usize;
                self.cursor_col = (self.cursor_col + n).min(self.cols.saturating_sub(1));
            }
            ('D', None) => {
                let n = next_param_or(1) as usize;
                self.cursor_col = self.cursor_col.saturating_sub(n);
            }
            ('G', None) => {
                let n = next_param_or(1) as usize;
                self.cursor_col = n.saturating_sub(1).min(self.cols.saturating_sub(1));
            }
            ('d', None) => {
                let n = next_param_or(1) as usize;
                self.cursor_row = n.saturating_sub(1).min(self.rows.saturating_sub(1));
            }

            ('J', None) => {
                let mode = next_param_or(0);
                match mode {
                    0 => {
                        // Erase from cursor to end of screen.
                        for c in self.cursor_col..self.cols {
                            self.grid[self.cursor_row][c] = Cell::default();
                        }
                        for r in self.cursor_row + 1..self.rows {
                            self.grid[r] = blank_row(self.cols);
                        }
                    }
                    1 => {
                        // Erase from start of screen to cursor.
                        for r in 0..self.cursor_row {
                            self.grid[r] = blank_row(self.cols);
                        }
                        for c in 0..=self.cursor_col.min(self.cols.saturating_sub(1)) {
                            self.grid[self.cursor_row][c] = Cell::default();
                        }
                    }
                    2 | 3 => {
                        for r in 0..self.rows {
                            self.grid[r] = blank_row(self.cols);
                        }
                    }
                    _ => {}
                }
            }
            ('K', None) => {
                let mode = next_param_or(0);
                match mode {
                    0 => {
                        for c in self.cursor_col..self.cols {
                            self.grid[self.cursor_row][c] = Cell::default();
                        }
                    }
                    1 => {
                        for c in 0..=self.cursor_col.min(self.cols.saturating_sub(1)) {
                            self.grid[self.cursor_row][c] = Cell::default();
                        }
                    }
                    2 => {
                        self.grid[self.cursor_row] = blank_row(self.cols);
                    }
                    _ => {}
                }
            }

            ('r', None) => {
                let top = next_param_or(1) as usize;
                let bot = params_iter
                    .next()
                    .map(|p| p[0] as usize)
                    .filter(|&p| p != 0)
                    .unwrap_or(self.rows);
                self.scroll_top = top.saturating_sub(1).min(self.rows.saturating_sub(1));
                self.scroll_bottom = bot.min(self.rows);
                if self.scroll_top >= self.scroll_bottom {
                    self.scroll_top = 0;
                    self.scroll_bottom = self.rows;
                }
                self.cursor_row = self.scroll_top;
                self.cursor_col = 0;
            }

            ('L', None) => {
                let n = next_param_or(1) as usize;
                let bot = self.scroll_bottom;
                let row = self.cursor_row;
                if row < bot {
                    let n = n.min(bot - row);
                    for i in (row + n..bot).rev() {
                        self.grid[i] = self.grid[i - n].clone();
                    }
                    for i in row..row + n {
                        self.grid[i] = blank_row(self.cols);
                    }
                }
            }
            ('M', None) => {
                let n = next_param_or(1) as usize;
                let bot = self.scroll_bottom;
                let row = self.cursor_row;
                if row < bot {
                    let n = n.min(bot - row);
                    for i in row..bot - n {
                        self.grid[i] = self.grid[i + n].clone();
                    }
                    for i in bot - n..bot {
                        self.grid[i] = blank_row(self.cols);
                    }
                }
            }
            ('S', None) => {
                let n = next_param_or(1) as usize;
                self.scroll_up(n);
            }
            ('T', None) => {
                let n = next_param_or(1) as usize;
                self.scroll_down(n);
            }

            ('@', None) => {
                let n = next_param_or(1) as usize;
                let row = self.cursor_row;
                let col = self.cursor_col;
                let end = self.cols;
                if col < end {
                    let n = n.min(end - col);
                    for i in (col + n..end).rev() {
                        self.grid[row][i] = self.grid[row][i - n];
                    }
                    for i in col..col + n {
                        self.grid[row][i] = Cell::default();
                    }
                }
            }
            ('P', None) => {
                let n = next_param_or(1) as usize;
                let row = self.cursor_row;
                let col = self.cursor_col;
                let end = self.cols;
                if col < end {
                    let n = n.min(end - col);
                    for i in col..end - n {
                        self.grid[row][i] = self.grid[row][i + n];
                    }
                    for i in end - n..end {
                        self.grid[row][i] = Cell::default();
                    }
                }
            }
            ('X', None) => {
                let n = next_param_or(1) as usize;
                let row = self.cursor_row;
                let end = (self.cursor_col + n).min(self.cols);
                for c in self.cursor_col..end {
                    self.grid[row][c] = Cell::default();
                }
            }

            ('h', Some(b'?')) => {
                for param in params_iter.map(|p| p[0]) {
                    if param == 1049 {
                        // Switch to alt screen.
                        self.saved_cursor = Some((self.cursor_row, self.cursor_col));
                        let alt = blank_grid(self.cols, self.rows);
                        self.alt_grid = Some(std::mem::replace(&mut self.grid, alt));
                        self.cursor_row = 0;
                        self.cursor_col = 0;
                    }
                }
            }
            ('l', Some(b'?')) => {
                for param in params_iter.map(|p| p[0]) {
                    if param == 1049 {
                        // Switch back from alt screen.
                        if let Some(main) = self.alt_grid.take() {
                            self.grid = main;
                        }
                        if let Some((r, c)) = self.saved_cursor.take() {
                            self.cursor_row = r;
                            self.cursor_col = c;
                        }
                    }
                }
            }

            ('n', None) => {} // DSR — ignored

            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        match (byte, intermediates) {
            (b'7', []) => {
                self.saved_cursor = Some((self.cursor_row, self.cursor_col));
            }
            (b'8', []) => {
                if let Some((r, c)) = self.saved_cursor {
                    self.cursor_row = r;
                    self.cursor_col = c;
                }
            }
            (b'M', []) => {
                // Reverse Index — move cursor up, scroll down if at top of region.
                if self.cursor_row == self.scroll_top {
                    self.scroll_down(1);
                } else if self.cursor_row > 0 {
                    self.cursor_row -= 1;
                }
            }
            (b'D', []) => {
                // Index — move cursor down, scroll up if at bottom of region.
                self.advance_cursor_down();
            }
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {}
    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _c: char) {}
    fn put(&mut self, _byte: u8) {}
    fn unhook(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid_text(term: &MiniTerm) -> Vec<String> {
        term.grid()
            .iter()
            .map(|row| row.iter().map(|c| c.ch).collect::<String>())
            .collect()
    }

    fn row_text(term: &MiniTerm, row: usize) -> String {
        term.grid()[row].iter().map(|c| c.ch).collect()
    }

    #[test]
    fn print_writes_chars_and_advances_cursor() {
        let mut t = MiniTerm::new(10, 5);
        t.process_bytes(b"Hi");
        assert_eq!(t.grid()[0][0].ch, 'H');
        assert_eq!(t.grid()[0][1].ch, 'i');
        assert_eq!(t.cursor_pos(), (0, 2));
    }

    #[test]
    fn line_wrapping() {
        let mut t = MiniTerm::new(4, 3);
        t.process_bytes(b"abcde");
        assert_eq!(row_text(&t, 0), "abcd");
        // 'e' wraps to the next row.
        assert_eq!(t.grid()[1][0].ch, 'e');
        assert_eq!(t.cursor_pos(), (1, 1));
    }

    #[test]
    fn lf_scrolls_at_bottom() {
        let mut t = MiniTerm::new(5, 3);
        // Fill all three rows.
        t.process_bytes(b"aaaa\r\nbbb\r\nccc");
        assert_eq!(t.cursor_pos(), (2, 3));
        // One more LF while on last row should scroll.
        t.process_bytes(b"\n");
        assert_eq!(row_text(&t, 0).trim_end(), "bbb");
        assert_eq!(row_text(&t, 1).trim_end(), "ccc");
        assert_eq!(row_text(&t, 2).trim(), "");
    }

    #[test]
    fn cr_moves_to_col_0() {
        let mut t = MiniTerm::new(10, 3);
        t.process_bytes(b"hello\r");
        assert_eq!(t.cursor_pos(), (0, 0));
    }

    #[test]
    fn sgr_sets_colors_and_flags() {
        let mut t = MiniTerm::new(10, 3);
        // ESC[1;31m = bold + fg red(1)
        t.process_bytes(b"\x1b[1;31mX");
        let cell = t.grid()[0][0];
        assert!(cell.attr.flags.contains(CellFlags::BOLD));
        assert_eq!(cell.attr.fg, Color::Indexed(1));
    }

    #[test]
    fn sgr_extended_256_color() {
        let mut t = MiniTerm::new(10, 3);
        // ESC[38;5;200m = fg indexed 200
        t.process_bytes(b"\x1b[38;5;200mA");
        assert_eq!(t.grid()[0][0].attr.fg, Color::Indexed(200));
    }

    #[test]
    fn sgr_extended_rgb_color() {
        let mut t = MiniTerm::new(10, 3);
        // ESC[48;2;10;20;30m = bg rgb(10,20,30)
        t.process_bytes(b"\x1b[48;2;10;20;30mB");
        assert_eq!(t.grid()[0][0].attr.bg, Color::Rgb(10, 20, 30));
    }

    #[test]
    fn cup_moves_cursor() {
        let mut t = MiniTerm::new(10, 10);
        // ESC[3;5H = move to row 3, col 5 (1-based)
        t.process_bytes(b"\x1b[3;5H");
        assert_eq!(t.cursor_pos(), (2, 4));
    }

    #[test]
    fn cup_defaults_to_1_1() {
        let mut t = MiniTerm::new(10, 10);
        t.process_bytes(b"\x1b[5;5H"); // move away
        t.process_bytes(b"\x1b[H"); // defaults to 1;1
        assert_eq!(t.cursor_pos(), (0, 0));
    }

    #[test]
    fn ed_erase_entire_screen() {
        let mut t = MiniTerm::new(5, 3);
        t.process_bytes(b"hello\r\nworld");
        // ESC[2J = erase entire screen
        t.process_bytes(b"\x1b[2J");
        for row in grid_text(&t) {
            assert_eq!(row, "     ");
        }
    }

    #[test]
    fn ed_erase_below() {
        let mut t = MiniTerm::new(5, 3);
        t.process_bytes(b"aaaaa\r\nbbbbb\r\nccccc");
        // Move to row 1, col 2 and erase below.
        t.process_bytes(b"\x1b[2;3H\x1b[0J");
        assert_eq!(row_text(&t, 0), "aaaaa");
        // Row 1 from col 2 onward should be cleared.
        assert_eq!(row_text(&t, 1), "bb   ");
        assert_eq!(row_text(&t, 2), "     ");
    }

    #[test]
    fn el_erase_to_end_of_line() {
        let mut t = MiniTerm::new(10, 3);
        t.process_bytes(b"0123456789");
        // Move to col 5 and erase to end of line.
        t.process_bytes(b"\x1b[1;6H\x1b[0K");
        assert_eq!(row_text(&t, 0), "01234     ");
    }

    #[test]
    fn el_erase_from_start_of_line() {
        let mut t = MiniTerm::new(10, 3);
        t.process_bytes(b"0123456789");
        // Move to col 3 and erase from start.
        t.process_bytes(b"\x1b[1;4H\x1b[1K");
        assert_eq!(row_text(&t, 0), "    456789");
    }

    #[test]
    fn el_erase_entire_line() {
        let mut t = MiniTerm::new(10, 3);
        t.process_bytes(b"0123456789");
        t.process_bytes(b"\x1b[1;4H\x1b[2K");
        assert_eq!(row_text(&t, 0), "          ");
    }

    #[test]
    fn scroll_region_limits_scrolling() {
        let mut t = MiniTerm::new(5, 5);
        // Fill rows.
        t.process_bytes(b"row0\r\n");
        t.process_bytes(b"row1\r\n");
        t.process_bytes(b"row2\r\n");
        t.process_bytes(b"row3\r\n");
        t.process_bytes(b"row4");
        // Set scroll region to rows 2-4 (1-indexed: 2;4).
        t.process_bytes(b"\x1b[2;4r");
        // Cursor should move to scroll_top.
        assert_eq!(t.cursor_pos(), (1, 0));
        // Move cursor to the bottom of the region and LF to scroll within it.
        t.process_bytes(b"\x1b[4;1H");
        t.process_bytes(b"\n");
        // row0 should be untouched (outside scroll region).
        assert_eq!(row_text(&t, 0).trim_end(), "row0");
        // row4 should also be untouched.
        assert_eq!(row_text(&t, 4).trim_end(), "row4");
        // Within the region, rows should have shifted up.
        assert_eq!(row_text(&t, 1).trim_end(), "row2");
        assert_eq!(row_text(&t, 2).trim_end(), "row3");
        assert_eq!(row_text(&t, 3).trim_end(), "");
    }

    #[test]
    fn take_scrolled_out_returns_and_drains() {
        let mut t = MiniTerm::new(5, 2);
        t.process_bytes(b"AAA\r\nBBB\r\nCCC");
        // AAA should have scrolled out.
        let scrolled = t.take_scrolled_out();
        assert_eq!(scrolled.len(), 1);
        assert_eq!(
            scrolled[0].iter().map(|c| c.ch).collect::<String>().trim_end(),
            "AAA"
        );
        // Calling again should return empty.
        assert!(t.take_scrolled_out().is_empty());
    }

    #[test]
    fn alt_screen_preserves_main() {
        let mut t = MiniTerm::new(10, 3);
        t.process_bytes(b"main text");
        // Enter alt screen: ESC[?1049h
        t.process_bytes(b"\x1b[?1049h");
        // Alt screen should be blank.
        assert_eq!(row_text(&t, 0).trim(), "");
        t.process_bytes(b"alt!");
        assert_eq!(row_text(&t, 0).trim_end(), "alt!");
        // Leave alt screen: ESC[?1049l
        t.process_bytes(b"\x1b[?1049l");
        assert_eq!(row_text(&t, 0).trim_end(), "main text");
    }

    #[test]
    fn cursor_movement_csi_abcd() {
        let mut t = MiniTerm::new(10, 10);
        t.process_bytes(b"\x1b[5;5H"); // row 4, col 4
        t.process_bytes(b"\x1b[2A"); // up 2
        assert_eq!(t.cursor_pos(), (2, 4));
        t.process_bytes(b"\x1b[3B"); // down 3
        assert_eq!(t.cursor_pos(), (5, 4));
        t.process_bytes(b"\x1b[2C"); // right 2
        assert_eq!(t.cursor_pos(), (5, 6));
        t.process_bytes(b"\x1b[4D"); // left 4
        assert_eq!(t.cursor_pos(), (5, 2));
    }

    #[test]
    fn save_and_restore_cursor_via_esc() {
        let mut t = MiniTerm::new(10, 10);
        t.process_bytes(b"\x1b[3;7H"); // row 2, col 6
        t.process_bytes(b"\x1b7"); // save
        t.process_bytes(b"\x1b[1;1H"); // home
        assert_eq!(t.cursor_pos(), (0, 0));
        t.process_bytes(b"\x1b8"); // restore
        assert_eq!(t.cursor_pos(), (2, 6));
    }

    #[test]
    fn backspace() {
        let mut t = MiniTerm::new(10, 3);
        t.process_bytes(b"abc");
        assert_eq!(t.cursor_pos(), (0, 3));
        t.process_bytes(b"\x08");
        assert_eq!(t.cursor_pos(), (0, 2));
        // Can't go below 0.
        t.process_bytes(b"\x08\x08\x08\x08");
        assert_eq!(t.cursor_pos(), (0, 0));
    }

    #[test]
    fn tab_advances_to_next_multiple_of_8() {
        let mut t = MiniTerm::new(20, 3);
        t.process_bytes(b"ab\t");
        assert_eq!(t.cursor_pos(), (0, 8));
        t.process_bytes(b"\t");
        assert_eq!(t.cursor_pos(), (0, 16));
    }

    #[test]
    fn resize_preserves_content_and_clamps_cursor() {
        let mut t = MiniTerm::new(10, 5);
        t.process_bytes(b"hello");
        t.process_bytes(b"\x1b[5;10H"); // cursor at (4, 9)
        t.resize(6, 3);
        assert_eq!(row_text(&t, 0), "hello ");
        // Cursor clamped to new bounds.
        assert_eq!(t.cursor_pos(), (2, 5));
    }

    #[test]
    fn insert_and_delete_lines() {
        let mut t = MiniTerm::new(5, 4);
        t.process_bytes(b"AAA\r\nBBB\r\nCCC\r\nDDD");
        // Move to row 1 and insert 1 line.
        t.process_bytes(b"\x1b[2;1H\x1b[1L");
        assert_eq!(row_text(&t, 0).trim_end(), "AAA");
        assert_eq!(row_text(&t, 1).trim(), "");
        assert_eq!(row_text(&t, 2).trim_end(), "BBB");
        assert_eq!(row_text(&t, 3).trim_end(), "CCC");
        // DDD got pushed off.
    }

    #[test]
    fn delete_characters() {
        let mut t = MiniTerm::new(10, 3);
        t.process_bytes(b"0123456789");
        // Move to col 2, delete 3 chars.
        t.process_bytes(b"\x1b[1;3H\x1b[3P");
        assert_eq!(row_text(&t, 0), "0156789   ");
    }

    #[test]
    fn insert_characters() {
        let mut t = MiniTerm::new(10, 3);
        t.process_bytes(b"0123456789");
        // Move to col 2, insert 2 blanks.
        t.process_bytes(b"\x1b[1;3H\x1b[2@");
        assert_eq!(row_text(&t, 0), "01  234567");
    }

    #[test]
    fn erase_characters() {
        let mut t = MiniTerm::new(10, 3);
        t.process_bytes(b"0123456789");
        // Move to col 3, erase 4 chars.
        t.process_bytes(b"\x1b[1;4H\x1b[4X");
        assert_eq!(row_text(&t, 0), "012    789");
        // Cursor should not have moved.
        assert_eq!(t.cursor_pos(), (0, 3));
    }

    #[test]
    fn reverse_index_scrolls_down_at_top() {
        let mut t = MiniTerm::new(5, 3);
        t.process_bytes(b"AAA\r\nBBB\r\nCCC");
        // Move to row 0 and reverse index.
        t.process_bytes(b"\x1b[1;1H\x1bM");
        assert_eq!(row_text(&t, 0).trim(), "");
        assert_eq!(row_text(&t, 1).trim_end(), "AAA");
        assert_eq!(row_text(&t, 2).trim_end(), "BBB");
    }

    #[test]
    fn bright_fg_and_bg_colors() {
        let mut t = MiniTerm::new(10, 3);
        // ESC[93m = bright yellow fg (index 11)
        // ESC[104m = bright blue bg (index 12)
        t.process_bytes(b"\x1b[93;104mZ");
        let cell = t.grid()[0][0];
        assert_eq!(cell.attr.fg, Color::Indexed(11));
        assert_eq!(cell.attr.bg, Color::Indexed(12));
    }

    #[test]
    fn sgr_reset_clears_attributes() {
        let mut t = MiniTerm::new(10, 3);
        t.process_bytes(b"\x1b[1;3;31mX\x1b[0mY");
        let x = t.grid()[0][0];
        assert!(x.attr.flags.contains(CellFlags::BOLD));
        assert!(x.attr.flags.contains(CellFlags::ITALIC));
        let y = t.grid()[0][1];
        assert_eq!(y.attr.flags, CellFlags::empty());
        assert_eq!(y.attr.fg, Color::Default);
    }

    #[test]
    fn cha_and_vpa() {
        let mut t = MiniTerm::new(10, 10);
        // CHA: ESC[5G = set cursor col to 4 (0-indexed).
        t.process_bytes(b"\x1b[5G");
        assert_eq!(t.cursor_pos(), (0, 4));
        // VPA: ESC[3d = set cursor row to 2 (0-indexed).
        t.process_bytes(b"\x1b[3d");
        assert_eq!(t.cursor_pos(), (2, 4));
    }

    #[test]
    fn scroll_up_and_down_csi() {
        let mut t = MiniTerm::new(5, 4);
        t.process_bytes(b"AAA\r\nBBB\r\nCCC\r\nDDD");
        // Scroll up 1: ESC[1S
        t.process_bytes(b"\x1b[1S");
        assert_eq!(row_text(&t, 0).trim_end(), "BBB");
        assert_eq!(row_text(&t, 3).trim(), "");
        // Scroll down 1: ESC[1T
        t.process_bytes(b"\x1b[1T");
        assert_eq!(row_text(&t, 0).trim(), "");
        assert_eq!(row_text(&t, 1).trim_end(), "BBB");
    }
}
