// The code in this file is adapted from the alacritty_terminal crate under the
// Apache license; see: crates/warp_terminal/src/model/LICENSE-ALACRITTY.

// path attribute needed due to current non-fs-based nesting of ansi_handler
// under grid_handler.
#[path = "ansi_handler/tab_stops.rs"]
mod tab_stops;

use std::cmp::min;
use std::collections::HashMap;
use std::io;
use std::ops::Range;
use std::sync::Arc;

use base64::Engine as _;
use bounded_vec_deque::BoundedVecDeque;
use pathfinder_geometry::vector::Vector2F;
use rand::Rng;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use warp_core::channel::ChannelState;
use warp_core::features::FeatureFlag;
use warp_terminal::model::ansi::CharsetIndex;
use warp_terminal::model::grid::cell;
use warp_terminal::model::{KeyboardModes, KeyboardModesApplyBehavior};
use warpui::image_cache::{resize_dimensions, FitType};

use crate::server::telemetry::ImageProtocol;
use crate::terminal::event::Event;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::ansi::{
    self, Attr, Color, CursorStyle, Handler as _, NamedColor, PrecmdValue, PreexecValue,
};
use crate::terminal::model::cell::{Cell, Flags};
use crate::terminal::model::char_or_str::CharOrStr;
use crate::terminal::model::grid::indexing::IndexRegion as _;
use crate::terminal::model::grid::{grapheme_cursor, Dimensions as _};
use crate::terminal::model::image_map::{ImagePlacementData, ImageType, StoredImageMetadata};
use crate::terminal::model::index::{Point, VisibleRow};
use crate::terminal::model::iterm_image::{ITermImage, ITermImageDimensionUnit};
use crate::terminal::model::kitty::{
    CursorMovementPolicy, KittyAction, KittyError, KittyResponse, StorageError,
};
use crate::terminal::model::selection::ScrollDelta;
use crate::terminal::model::ObfuscateSecrets;
use crate::terminal::{ClipboardType, SizeInfo};

use super::{AbsolutePoint, FullGridClearBehavior, GridHandler, PerformResetGridChecks, TermMode};

use tab_stops::TabStops;

const MAX_IMAGE_CELL_HEIGHT: u32 = 255;

/// State needed for the grid-level implementation of [`ansi::Handler`].
#[derive(Clone)]
pub(super) struct State {
    /// Information about cell dimensions.
    pub cell_width: usize,
    pub cell_height: usize,

    /// Mode flags.
    pub mode: TermMode,

    /// Tabstops.
    pub tabs: TabStops,

    /// Current style of the cursor.
    pub cursor_style: CursorStyle,

    /// Index into `charsets`, pointing to what ASCII is currently being mapped to.
    pub active_charset: CharsetIndex,

    /// Scroll region.
    ///
    /// Range going from top to bottom of the terminal.
    pub scroll_region: Range<VisibleRow>,

    /// Proxy for sending events to the event loop.
    pub event_proxy: ChannelEventListener,

    /// Whether the grid is for the alt screen.
    pub is_alt_screen: bool,

    /// Whether or not to obfuscate secrets on copy, respecting the Safe Mode setting.
    pub obfuscate_secrets: ObfuscateSecrets,

    /// Whether this Grid is in a shell context which supports handling the emoji presentation selector
    /// correctly. Notably, Zsh does NOT support this well in bracketed paste mode (which we use for all Warp
    /// commands), which can lead to cursor misalignment issues.
    pub supports_emoji_presentation_selector: bool,

    /// State related to the Reset Grid logic for ConPTY.
    reset_grid_checks: ResetGridChecks,

    /// Range of cells that were dirtied during the current run of byte parsing. See
    /// [`Self::finish_byte_processing`] for where this is reset.
    pub dirty_cells_range: Range<Point>,

    // Dimension of pane.
    pub pane_size: Vector2F,

    /// The currently active keyboard mode.
    pub keyboard_mode: KeyboardModes,

    /// Kitty keyboard enhancement protocol mode stack.
    /// Used purely for push/pop save/restore semantics.
    /// `push` appends the new mode; `pop` truncates and restores
    /// the previous entry as the active mode.
    pub keyboard_mode_stack: BoundedVecDeque<KeyboardModes>,
}

impl State {
    pub fn new(
        size_info: &SizeInfo,
        event_proxy: ChannelEventListener,
        is_alt_screen: bool,
        obfuscate_secrets: ObfuscateSecrets,
        perform_reset_grid_checks: PerformResetGridChecks,
    ) -> Self {
        let scroll_region = VisibleRow(0)..VisibleRow(size_info.rows());
        let tabs = TabStops::new(size_info.columns());
        let reset_grid_checks = match perform_reset_grid_checks {
            PerformResetGridChecks::Yes => ResetGridChecks::Enabled {
                received_osc: false,
            },
            PerformResetGridChecks::No => ResetGridChecks::Disabled,
        };

        Self {
            cell_width: size_info.cell_width_px.as_f32() as usize,
            cell_height: size_info.cell_height_px.as_f32() as usize,
            mode: Default::default(),
            tabs,
            cursor_style: Default::default(),
            active_charset: Default::default(),
            scroll_region,
            event_proxy,
            is_alt_screen,
            obfuscate_secrets,
            // Assume that the Grid supports emoji presentation selector, until set otherwise.
            supports_emoji_presentation_selector: true,
            reset_grid_checks,
            dirty_cells_range: Default::default(),
            pane_size: size_info.pane_size_px(),
            keyboard_mode: KeyboardModes::NO_MODE,
            keyboard_mode_stack: BoundedVecDeque::new(super::KEYBOARD_MODE_STACK_MAX_DEPTH),
        }
    }
}

#[derive(Default, Clone, Copy, PartialEq, Debug)]
enum ResetGridChecks {
    /// Checks are enabled for the grid.
    Enabled { received_osc: bool },
    /// Checks are disabled for the grid.
    #[default]
    Disabled,
}

impl ansi::Handler for GridHandler {
    fn set_title(&mut self, _: Option<String>) {
        log::error!("Handler method GridHandler::set_title should never be called. This should be handled by TerminalModel.");
    }

    fn set_cursor_style(&mut self, style: Option<ansi::CursorStyle>) {
        self.ansi_handler_state.cursor_style = style.unwrap_or_default();

        // Notify UI about blinking changes.
        self.ansi_handler_state
            .event_proxy
            .send_terminal_event(Event::CursorBlinkingChange(
                self.ansi_handler_state.cursor_style.blinking,
            ));
    }

    fn set_cursor_shape(&mut self, shape: ansi::CursorShape) {
        self.ansi_handler_state.cursor_style.shape = shape;
    }

    fn input(&mut self, c: char) {
        // We disable Reset Grid checks in unit tests, as they are not designed to test
        // PTY integration. `#[cfg(test)]` only applies to unit tests, not integration tests.
        #[cfg(all(windows, not(test)))]
        if let ResetGridChecks::Enabled { received_osc } = self.ansi_handler_state.reset_grid_checks
        {
            debug_assert!(
                received_osc,
                "Grid received input but did not receive Reset Grid OSC"
            );
        }
        // Number of cells the char will occupy.
        let Some(width) = c.width() else {
            return;
        };

        let num_cols = self.columns();

        // Handle zero-width characters.
        if width == 0 {
            // Get previous column.
            let mut col = self.grid.cursor().point.col;
            if !self.grid.cursor().input_needs_wrap {
                col = col.saturating_sub(1);
            }

            // Put zerowidth characters over first fullwidth character cell.
            let row = self.grid.cursor_point().row;
            if self.grid[row][col].flags.contains(Flags::WIDE_CHAR_SPACER) {
                col = col.saturating_sub(1);
            }

            self.grid[row][col].push_zerowidth(c, /* log_long_grapheme_warnings */ true);
            let cell_content_width = match self.grid[row][col].raw_content() {
                CharOrStr::Str(s) => s.width(),
                // Note that we should never reach here since we are pushing a zerowidth character,
                // which should always make the cell content a string. However, we cover these cases
                // exhaustively as a safeguard (to avoid panics).
                CharOrStr::Char(c) => match c.width() {
                    Some(width) => width,
                    None => {
                        return;
                    }
                },
            };

            // Bash and Fish support emoji variation selectors, but Zsh does not in bracketed paste
            // mode. Specifically, this references sequences such as \0x2601\0xFE0F (☁️),
            // which are commonly used in prompts e.g. GCloud prompt chip in Starship.
            if cell_content_width == 2
                && self.ansi_handler_state.supports_emoji_presentation_selector
            {
                // Current cursor cell contains a wide character (double-width).
                self.grid[row][col].flags.insert(Flags::WIDE_CHAR);

                // Insert spacer at the next cell.
                self.write_at_cursor(cell::DEFAULT_CHAR)
                    .flags
                    .insert(Flags::WIDE_CHAR_SPACER);

                // Update cursor appropriately before early-return.
                self.advance_cursor_by_one_cell();
            }
            return;
        }

        // Move cursor to next line.
        if self.grid.cursor().input_needs_wrap {
            self.wrapline();
        }

        // If in insert mode, first shift cells to the right.
        if self.ansi_handler_state.mode.contains(TermMode::INSERT)
            && self.grid.cursor().point.col + width < num_cols
        {
            let cursor_point = self.grid.cursor_point();
            let col = self.grid.cursor().point.col;
            let bg = self.grid.cursor().template.bg;

            // Reset any wide char pair at the insertion point before the
            // shift moves cells away (write_at_cursor's own start boundary
            // check runs too late — after the shift).
            self.reset_wide_char_at_start_boundary(cursor_point.row, col, bg);

            // Reset any wide char pair straddling the push-off boundary.
            // Cells at [num_cols - width, num_cols) are discarded by the
            // shift.
            let push_off = num_cols - width;
            self.reset_wide_char_at_end_boundary(cursor_point.row, push_off, bg);

            let row = &mut self.grid[cursor_point.row][..];

            for col in (col..(num_cols - width)).rev() {
                row.swap(col + width, col);
            }
        }

        if width == 1 {
            self.write_at_cursor(c);
        } else {
            if self.grid.cursor().point.col + 1 >= num_cols {
                if self.ansi_handler_state.mode.contains(TermMode::LINE_WRAP) {
                    // Insert placeholder before wide char if glyph does not fit in this row.
                    self.write_at_cursor(cell::DEFAULT_CHAR)
                        .flags
                        .insert(Flags::LEADING_WIDE_CHAR_SPACER);
                    self.wrapline();
                } else {
                    // Prevent out of bounds crash when linewrapping is disabled.
                    self.move_cursor_forward(|cursor| {
                        cursor.input_needs_wrap = true;
                    });
                    return;
                }
            }

            // Write full width glyph to current cursor cell.
            self.write_at_cursor(c).flags.insert(Flags::WIDE_CHAR);

            // Write spacer to cell following the wide glyph.
            self.move_cursor_forward(|cursor| {
                cursor.point.col += 1;
            });

            self.write_at_cursor(cell::DEFAULT_CHAR)
                .flags
                .insert(Flags::WIDE_CHAR_SPACER);
        }

        self.advance_cursor_by_one_cell();
    }

    fn goto(&mut self, row: VisibleRow, column: usize) {
        log::trace!("Going to: line={row}, col={column}");
        let (y_offset, max_y) = if self.ansi_handler_state.mode.contains(TermMode::ORIGIN) {
            (
                self.ansi_handler_state.scroll_region.start,
                self.ansi_handler_state.scroll_region.end - 1,
            )
        } else {
            (VisibleRow(0), VisibleRow(self.visible_rows() - 1))
        };

        let columns = self.columns();
        self.update_cursor(|cursor| {
            cursor.point.row = min(row + y_offset, max_y);
            cursor.point.col = min(column, columns.saturating_sub(1));
            cursor.input_needs_wrap = false;
        });
    }

    fn goto_line(&mut self, row: VisibleRow) {
        self.goto(row, self.grid.cursor().point.col);
    }

    fn goto_col(&mut self, column: usize) {
        log::trace!("Going to column: {column}");
        self.goto(self.grid.cursor().point.row, column)
    }

    fn insert_blank(&mut self, count: usize) {
        let cursor = &self.grid.cursor();
        let bg = cursor.template.bg;

        // Ensure inserting within terminal bounds
        let count = min(count, self.columns() - cursor.point.col);

        let source = cursor.point.col;
        let destination = cursor.point.col + count;
        let num_cells = self.columns() - destination;

        let cursor_point = self.grid.cursor_point();

        // Reset any wide character pair that straddles the insertion boundary.
        self.reset_wide_char_at_start_boundary(cursor_point.row, source, bg);

        // Reset any wide char pair that straddles the push-off boundary.
        // Cells at [cols - count, cols) are discarded when shifted right; if
        // the first discarded cell is a WIDE_CHAR_SPACER, its partner in the
        // kept zone would become orphaned after the shift.
        let cols = self.columns();
        let push_off = cols - count;
        if push_off < cols {
            self.reset_wide_char_at_end_boundary(cursor_point.row, push_off, bg);
        }

        let row = &mut self.grid[cursor_point.row][..];

        for offset in (0..num_cells).rev() {
            row.swap(destination + offset, source + offset);
        }

        // Cells were just moved out toward the end of the line;
        // fill in between source and dest with blanks.
        for cell in &mut row[source..destination] {
            *cell = bg.into();
        }
    }

    fn move_up(&mut self, lines: usize) {
        log::trace!("Moving up: {lines}");

        let move_to = self.grid.cursor().point.row.saturating_sub(lines);
        self.goto(move_to, self.grid.cursor().point.col)
    }

    fn move_down(&mut self, lines: usize) {
        log::trace!("Moving down: {lines}");
        let move_to = self.grid.cursor().point.row + lines;
        self.goto(move_to, self.grid.cursor().point.col)
    }

    fn identify_terminal<W: std::io::Write>(&mut self, writer: &mut W, intermediate: Option<char>) {
        match intermediate {
            None => {
                log::trace!("Reporting primary device attributes");
                let _ = writer.write_all(b"\x1b[?62c");
            }
            Some('>') => {
                log::trace!("Reporting secondary device attributes");
                // The version here is hardcoded, but there is a reason for that! :)
                // Following the documentation from: https://invisible-island.net/xterm/ctlseqs/ctlseqs.html
                // and Send Device Attributes (Secondary DA), the value should be of format: ` CSI  > Pp ; Pv ; Pc c`.
                // Pp denotes the terminal type, where 0 stands for `VT100` which we emulate. The
                // `Pv` is a firmware version, and `Pc` indicates ROM cartridge.
                //
                // It turns out that this values are necessary for vim to recognize the terminal's
                // mouse capabilities (SGR_MOUSE). Ie. if you want to use your mouse in vim and `:set mouse=a`
                // to work like a charm, this value needs to pass set of checks.
                // Apparently, in actual Vim's source code[1] there's a hardcoded list of versions
                // for different recognized terminals/use cases. Otherwise, the code checks that
                // the version `Pv` is higher than the `xterm` version when the SGR_MOUSE support
                // was introduced[2] - version 277.
                //
                // Since we didn't want to claim xterm functionalities that we haven't yet implemented in
                // Warp, rather than passing the higher `Pv` value, we decided to use one of the
                // hardcoded ones. `0;95;0` is set what iTerm2 sends.

                // [1] https://github.com/vim/vim/blob/20c370d9f2ee89cb854054edf71f5004f6efff77/src/term.c#L4630
                // [2] https://invisible-island.net/xterm/xterm.log.html#xterm_277
                //
                // Further reading for even more context:
                // * http://vimdoc.sourceforge.net/htmldoc/options.html#'ttymouse'
                // * https://github.com/alacritty/alacritty/issues/803
                // * https://github.com/vim/vim/issues/2309
                let version = "0;95;0";
                let _ = writer.write_all(format!("\x1b[>{version}c").as_bytes());
            }
            _ => log::debug!("Unsupported device attributes intermediate"),
        }
    }

    fn report_xtversion<W: std::io::Write>(&mut self, writer: &mut W) {
        log::trace!("Reporting xtversion");
        let version = ChannelState::app_version().unwrap_or("");
        let _ = writer.write_all(format!("\x1bP>|Warp({version})\x1b\\").as_bytes());
    }

    fn device_status<W: std::io::Write>(&mut self, writer: &mut W, arg: usize) {
        log::trace!("Reporting device status: {arg}");
        match arg {
            5 => {
                let _ = writer.write_all(b"\x1b[0n");
            }
            6 => {
                let pos = self.grid.cursor().point;
                let response = format!("\x1b[{};{}R", pos.row + 1, pos.col + 1);
                let _ = writer.write_all(response.as_bytes());
            }
            _ => log::debug!("unknown device status query: {arg}"),
        };
    }

    fn move_forward(&mut self, columns: usize) {
        log::trace!("Moving forward: {columns}");
        let num_cols = self.columns();

        self.move_cursor_forward(|cursor| {
            cursor.point.col = min(cursor.point.col + columns, num_cols.saturating_sub(1));
            cursor.input_needs_wrap = false;
        });
    }

    fn move_backward(&mut self, columns: usize) {
        log::trace!("Moving backward: {columns}");

        self.update_cursor(|cursor| {
            cursor.point.col = cursor.point.col.saturating_sub(columns);
            cursor.input_needs_wrap = false;
        });
    }

    fn move_down_and_cr(&mut self, lines: usize) {
        log::trace!("Moving down and cr: {lines}");
        let move_to = self.grid.cursor().point.row + lines;
        self.goto(move_to, 0)
    }

    fn move_up_and_cr(&mut self, lines: usize) {
        log::trace!("Moving up and cr: {lines}");
        self.goto(self.grid.cursor().point.row.saturating_sub(lines), 0)
    }

    /// Insert tab at cursor position.
    fn put_tab(&mut self, mut count: u16) {
        // A tab after the last column is the same as a linebreak.
        if self.grid.cursor().input_needs_wrap {
            self.wrapline();
            return;
        }

        while self.grid.cursor().point.col < self.columns() && count != 0 {
            count -= 1;

            let c = self.grid.cursor().charsets[self.ansi_handler_state.active_charset].map('\t');
            let cell = self.grid.cursor_cell();
            // Overwrite empty cells or ones containing whitespace with the
            // current charset's tab character.
            if cell.c == cell::DEFAULT_CHAR || cell.c == ' ' {
                cell.c = c;
            }

            loop {
                if (self.grid.cursor().point.col + 1) == self.columns() {
                    break;
                }

                self.move_cursor_forward(|cursor| {
                    cursor.point.col += 1;
                });

                if self.ansi_handler_state.tabs[self.grid.cursor().point.col] {
                    break;
                }
            }
        }
    }

    fn backspace(&mut self) {
        log::trace!("Backspace");

        if self.grid.cursor().point.col > 0 {
            self.update_cursor(|cursor| {
                cursor.point.col -= 1;
                cursor.input_needs_wrap = false;
            });
        }
    }

    fn carriage_return(&mut self) {
        log::trace!("Carriage return");
        self.update_cursor(|cursor| {
            cursor.point.col = 0;
            cursor.input_needs_wrap = false;
        });
    }

    fn linefeed(&mut self) -> ScrollDelta {
        log::trace!("Linefeed");
        let next = self.grid.cursor().point.row + 1;
        if next == self.ansi_handler_state.scroll_region.end {
            return self.scroll_up(1);
        }
        if next.0 < self.visible_rows() {
            self.move_cursor_forward(|cursor| {
                cursor.point.row += 1;
            });
        }
        ScrollDelta::zero()
    }

    /// Ring the terminal bell.
    fn bell(&mut self) {
        log::trace!("Bell");
        self.ansi_handler_state
            .event_proxy
            .send_terminal_event(Event::Bell);
    }

    fn substitute(&mut self) {}

    /// Run LF/NL.
    ///
    /// LF/NL mode has some interesting history. According to ECMA-48 4th
    /// edition, in LINE FEED mode,
    ///
    /// > The execution of the formatter functions LINE FEED (LF), FORM FEED
    /// > (FF), LINE TABULATION (VT) cause only movement of the active position in
    /// > the direction of the line progression.
    ///
    /// In NEW LINE mode,
    ///
    /// > The execution of the formatter functions LINE FEED (LF), FORM FEED
    /// > (FF), LINE TABULATION (VT) cause movement to the line home position on
    /// > the following line, the following form, etc. In the case of LF this is
    /// > referred to as the New Line (NL) option.
    ///
    /// Additionally, ECMA-48 4th edition says that this option is deprecated.
    /// ECMA-48 5th edition only mentions this option (without explanation)
    /// saying that it's been removed.
    ///
    /// As an emulator, we need to support it since applications may still rely
    /// on it.
    fn newline(&mut self) {
        self.linefeed();

        if self
            .ansi_handler_state
            .mode
            .contains(TermMode::LINE_FEED_NEW_LINE)
        {
            self.carriage_return();
        }
    }

    fn set_horizontal_tabstop(&mut self) {
        log::trace!("Setting horizontal tabstop");
        self.ansi_handler_state.tabs[self.grid.cursor().point.col] = true;
    }

    fn scroll_up(&mut self, lines: usize) -> ScrollDelta {
        let origin = self.ansi_handler_state.scroll_region.start;
        self.scroll_up_relative(origin, lines)
    }

    fn scroll_down(&mut self, lines: usize) -> ScrollDelta {
        let origin = self.ansi_handler_state.scroll_region.start;
        self.scroll_down_relative(origin, lines)
    }

    fn insert_blank_lines(&mut self, lines: usize) -> ScrollDelta {
        log::trace!("Inserting blank {lines} lines");

        let origin = self.grid.cursor().point.row;
        if self.ansi_handler_state.scroll_region.contains(&origin) {
            self.scroll_down_relative(origin, lines)
        } else {
            ScrollDelta::zero()
        }
    }

    fn delete_lines(&mut self, lines: usize) -> ScrollDelta {
        let origin = self.grid.cursor().point.row;
        let lines = min(self.visible_rows() - origin.0, lines);

        log::trace!("Deleting {lines} lines");

        if lines > 0
            && self
                .ansi_handler_state
                .scroll_region
                .contains(&self.grid.cursor().point.row)
        {
            self.scroll_up_relative(origin, lines)
        } else {
            ScrollDelta::zero()
        }
    }

    fn erase_chars(&mut self, count: usize) {
        if count == 0 {
            return;
        }

        let cursor = &self.grid.cursor();

        log::trace!("Erasing chars: count={}, col={}", count, cursor.point.col);

        let start = cursor.point.col;
        let end = min(start + count, self.columns());

        // Cleared cells have current background color set.
        let bg = self.grid.cursor().template.bg;
        let cursor_point = self.grid.cursor_point();

        // Reset any wide character pair that straddles the erase boundary.
        self.reset_wide_char_at_start_boundary(cursor_point.row, start, bg);
        if end < self.columns() {
            self.reset_wide_char_at_end_boundary(cursor_point.row, end, bg);
        }

        let row = &mut self.grid[cursor_point.row];
        for cell in &mut row[start..end] {
            *cell = bg.into();
        }
    }

    fn delete_chars(&mut self, count: usize) {
        if count == 0 {
            return;
        }

        let cols = self.columns();
        let cursor = &self.grid.cursor();
        let bg = cursor.template.bg;

        // Ensure deleting within terminal bounds.
        let count = min(count, cols);

        let start = cursor.point.col;
        let end = min(start + count, cols - 1);
        let num_cells = cols - end;

        let cursor_point = self.grid.cursor_point();

        // Reset any wide character pair that straddles the delete boundary.
        self.reset_wide_char_at_start_boundary(cursor_point.row, start, bg);
        if end < cols {
            self.reset_wide_char_at_end_boundary(cursor_point.row, end, bg);
        }

        let row = &mut self.grid[cursor_point.row][..];

        for offset in 0..num_cells {
            row.swap(start + offset, end + offset);
        }

        // Clear last `count` cells in the row. If deleting 1 char, need to delete
        // 1 cell.
        let end = cols - count;
        for cell in &mut row[end..] {
            *cell = bg.into();
        }
    }

    fn move_backward_tabs(&mut self, count: u16) {
        log::trace!("Moving backward {count} tabs");

        for _ in 0..count {
            let mut col = self.grid.cursor().point.col;
            for i in (0..(col)).rev() {
                if self.ansi_handler_state.tabs[i] {
                    col = i;
                    break;
                }
            }
            self.update_cursor(|cursor| {
                cursor.point.col = col;
            });
        }
    }

    fn move_forward_tabs(&mut self, count: u16) {
        log::trace!("[unimplemented] Moving forward {count} tabs")
    }

    fn save_cursor_position(&mut self) {
        log::trace!("Saving cursor position");

        self.grid.saved_cursor = self.grid.cursor().clone();
    }

    fn restore_cursor_position(&mut self) {
        log::trace!("Restoring cursor position");

        let saved_cursor = self.grid.saved_cursor.clone();
        self.update_cursor(|cursor| {
            *cursor = saved_cursor;
        });
    }

    fn clear_line(&mut self, mode: ansi::LineClearMode) {
        log::trace!("Clearing line: {mode:?}");

        let cursor = &self.grid.cursor();
        let bg = cursor.template.bg;

        let point = self.grid.cursor_point();

        // Reset any wide character pair that straddles the clear boundary.
        match mode {
            ansi::LineClearMode::Right => {
                self.reset_wide_char_at_start_boundary(point.row, point.col, bg);
            }
            ansi::LineClearMode::Left => {
                let num_cols = self.columns();
                if point.col + 1 < num_cols {
                    self.reset_wide_char_at_end_boundary(point.row, point.col + 1, bg);
                }
            }
            ansi::LineClearMode::All => {}
        }

        let row = &mut self.grid[point.row];

        let mut start_point = point;
        let mut end_point = point;

        match mode {
            ansi::LineClearMode::Right => {
                for cell in &mut row[point.col..] {
                    *cell = bg.into();
                }
                end_point.col = usize::MAX;
            }
            ansi::LineClearMode::Left => {
                for cell in &mut row[..=point.col] {
                    *cell = bg.into();
                }
                start_point.col = usize::MIN;
            }
            ansi::LineClearMode::All => {
                for cell in &mut row[..] {
                    *cell = bg.into();
                }
                end_point.col = usize::MAX;
                end_point.col = usize::MAX;
            }
        }

        self.images.evict_image_ids_between_points_with_type(
            AbsolutePoint::from_point(start_point, self),
            AbsolutePoint::from_point(end_point, self),
            vec![ImageType::ITerm],
        );

        // TODO(alokedesai): Need to handle selection here.
    }

    fn clear_screen(&mut self, mode: ansi::ClearMode) {
        log::trace!("Clearing screen: {mode:?}");
        let bg = self.grid.cursor().template.bg;

        let num_lines = self.visible_rows();

        match mode {
            ansi::ClearMode::Above => {
                // Clearing above the cursor is guaranteed to clear everything
                // in scrollback.
                self.flat_storage.clear();

                let cursor = self.grid.cursor().point;

                // If clearing more than one line.
                if cursor.row > VisibleRow(1) {
                    // Fully clear all lines before the current line.
                    self.grid
                        .region_mut(..cursor.row)
                        .each(|cell| *cell = bg.into());
                }

                // Clear up to the current column in the current line.
                let end = min(cursor.col + 1, self.columns());
                let cursor_point = self.grid.cursor_point();
                // Reset any wide char pair that straddles the boundary just
                // past the cleared region on the cursor row.
                if end < self.columns() {
                    self.reset_wide_char_at_end_boundary(cursor_point.row, end, bg);
                }
                for cell in &mut self.grid[cursor_point.row][..end] {
                    *cell = bg.into();
                }
            }
            ansi::ClearMode::Below => {
                let cursor = self.grid.cursor().point;
                let cursor_point = self.grid.cursor_point();
                // Reset any wide char pair that straddles the start of the
                // cleared region on the cursor row.
                self.reset_wide_char_at_start_boundary(cursor_point.row, cursor.col, bg);
                for cell in &mut self.grid[cursor_point.row][cursor.col..] {
                    *cell = bg.into();
                }

                if cursor.row.0 < num_lines - 1 {
                    self.grid
                        .region_mut((cursor.row + 1)..)
                        .each(|cell| *cell = bg.into());
                }
            }
            ansi::ClearMode::All => {
                if self.ansi_handler_state.is_alt_screen {
                    self.grid.region_mut(..).each(|cell| *cell = bg.into());
                } else if self.full_grid_clear_behavior == FullGridClearBehavior::Clear {
                    self.clear_visible_rows_in_place(bg);
                } else {
                    self.clear_viewport();
                }
            }
            ansi::ClearMode::Saved if self.history_size() > 0 => {
                self.flat_storage.clear();
                self.grid.clear_history();
            }
            // We have no history to clear.
            ansi::ClearMode::Saved => (),
            ansi::ClearMode::ResetAndClear | ansi::ClearMode::ActiveBlock => {
                self.flat_storage.clear();
                self.grid.clear_and_reset_saving_cursor_line();

                // Clear out state that will no longer be valid now that the
                // grid has been cleared.
                self.clear_secrets();
                self.clear_displayed_rows_and_filter_matches();

                // The row with the cursor still exists, though, so mark it as
                // dirty and re-compute state accordingly.
                let cursor_row = self.cursor_point().row;
                self.ansi_handler_state.dirty_cells_range =
                    Point::new(cursor_row, 0)..Point::new(cursor_row + 1, 0);
                self.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
            }
        }
    }

    fn clear_tabs(&mut self, mode: ansi::TabulationClearMode) {
        log::trace!("Clearing tabs: {mode:?}");
        match mode {
            ansi::TabulationClearMode::Current => {
                self.ansi_handler_state.tabs[self.grid.cursor().point.col] = false;
            }
            ansi::TabulationClearMode::All => {
                self.ansi_handler_state.tabs.clear_all();
            }
        }
    }

    /// Reset all important fields in the term struct.
    #[inline]
    fn reset_state(&mut self) {
        self.grid.reset();
        self.flat_storage.clear();

        self.clear_secrets();

        self.ansi_handler_state.active_charset = Default::default();
        self.ansi_handler_state.cursor_style = CursorStyle::default();
        self.ansi_handler_state.scroll_region = VisibleRow(0)..VisibleRow(self.visible_rows());
        self.ansi_handler_state.tabs = TabStops::new(self.columns());

        let blinking = self.ansi_handler_state.cursor_style.blinking;
        self.ansi_handler_state
            .event_proxy
            .send_terminal_event(Event::CursorBlinkingChange(blinking));
    }

    fn reverse_index(&mut self) -> ScrollDelta {
        log::trace!("Reversing index");
        // If cursor is at the top.
        if self.grid.cursor().point.row == self.ansi_handler_state.scroll_region.start {
            self.scroll_down(1)
        } else {
            self.update_cursor(|cursor| {
                cursor.point.row = cursor.point.row.saturating_sub(1);
            });

            ScrollDelta::zero()
        }
    }

    /// Set a terminal attribute.
    #[inline]
    fn terminal_attribute(&mut self, attr: ansi::Attr) {
        log::trace!("Setting attribute: {attr:?}");
        let template = &mut self.grid.cursor.template;
        match attr {
            Attr::Foreground(color) => template.fg = color,
            Attr::Background(color) => template.bg = color,
            Attr::Reset => {
                template.fg = Color::Named(NamedColor::Foreground);
                template.bg = Color::Named(NamedColor::Background);
                template.flags = Flags::empty();
            }
            Attr::Reverse => template.flags.insert(Flags::INVERSE),
            Attr::CancelReverse => template.flags.remove(Flags::INVERSE),
            Attr::Bold => template.flags.insert(Flags::BOLD),
            Attr::CancelBold => template.flags.remove(Flags::BOLD),
            Attr::Dim => template.flags.insert(Flags::DIM),
            Attr::CancelBoldDim => template.flags.remove(Flags::BOLD | Flags::DIM),
            Attr::Italic => template.flags.insert(Flags::ITALIC),
            Attr::CancelItalic => template.flags.remove(Flags::ITALIC),
            Attr::Underline => {
                template.flags.remove(Flags::DOUBLE_UNDERLINE);
                template.flags.insert(Flags::UNDERLINE);
            }
            Attr::DoubleUnderline => {
                template.flags.remove(Flags::UNDERLINE);
                template.flags.insert(Flags::DOUBLE_UNDERLINE);
            }
            Attr::CancelUnderline => {
                template
                    .flags
                    .remove(Flags::UNDERLINE | Flags::DOUBLE_UNDERLINE);
            }
            Attr::Hidden => template.flags.insert(Flags::HIDDEN),
            Attr::CancelHidden => template.flags.remove(Flags::HIDDEN),
            Attr::Strike => template.flags.insert(Flags::STRIKEOUT),
            Attr::CancelStrike => template.flags.remove(Flags::STRIKEOUT),
            _ => {
                log::debug!("Term got unhandled attr: {attr:?}");
            }
        }
    }

    fn set_mode(&mut self, mode: ansi::Mode) {
        log::trace!("Setting mode: {mode:?}");
        match mode {
            ansi::Mode::UrgencyHints => {
                self.ansi_handler_state.mode.insert(TermMode::URGENCY_HINTS)
            }
            ansi::Mode::SwapScreen { .. } => unreachable!("Handled in model layer"),
            ansi::Mode::ShowCursor => self.ansi_handler_state.mode.insert(TermMode::SHOW_CURSOR),
            ansi::Mode::CursorKeys => self.ansi_handler_state.mode.insert(TermMode::APP_CURSOR),
            // Mouse protocols are mutually exclusive.
            ansi::Mode::ReportMouseClicks => {
                self.ansi_handler_state.mode.remove(TermMode::MOUSE_MODE);
                self.ansi_handler_state
                    .mode
                    .insert(TermMode::MOUSE_REPORT_CLICK);
                self.ansi_handler_state
                    .event_proxy
                    .send_terminal_event(Event::MouseCursorDirty);
            }
            ansi::Mode::ReportCellMouseMotion => {
                self.ansi_handler_state.mode.remove(TermMode::MOUSE_MODE);
                self.ansi_handler_state.mode.insert(TermMode::MOUSE_DRAG);
                self.ansi_handler_state
                    .event_proxy
                    .send_terminal_event(Event::MouseCursorDirty);
            }
            ansi::Mode::ReportAllMouseMotion => {
                self.ansi_handler_state.mode.remove(TermMode::MOUSE_MODE);
                self.ansi_handler_state.mode.insert(TermMode::MOUSE_MOTION);
                self.ansi_handler_state
                    .event_proxy
                    .send_terminal_event(Event::MouseCursorDirty);
            }
            ansi::Mode::ReportFocusInOut => {
                self.ansi_handler_state.mode.insert(TermMode::FOCUS_IN_OUT)
            }
            ansi::Mode::BracketedPaste => self
                .ansi_handler_state
                .mode
                .insert(TermMode::BRACKETED_PASTE),
            // Mouse encodings are mutually exclusive.
            ansi::Mode::SgrMouse => {
                self.ansi_handler_state.mode.remove(TermMode::UTF8_MOUSE);
                self.ansi_handler_state.mode.insert(TermMode::SGR_MOUSE);
            }
            ansi::Mode::Utf8Mouse => {
                self.ansi_handler_state.mode.remove(TermMode::SGR_MOUSE);
                self.ansi_handler_state.mode.insert(TermMode::UTF8_MOUSE);
            }
            ansi::Mode::AlternateScroll => self
                .ansi_handler_state
                .mode
                .insert(TermMode::ALTERNATE_SCROLL),
            ansi::Mode::LineWrap => self.ansi_handler_state.mode.insert(TermMode::LINE_WRAP),
            ansi::Mode::LineFeedNewLine => self
                .ansi_handler_state
                .mode
                .insert(TermMode::LINE_FEED_NEW_LINE),
            ansi::Mode::Origin => self.ansi_handler_state.mode.insert(TermMode::ORIGIN),
            ansi::Mode::DECCOLM => self.deccolm(),
            ansi::Mode::Insert => self.ansi_handler_state.mode.insert(TermMode::INSERT),
            ansi::Mode::BlinkingCursor => {
                self.ansi_handler_state.cursor_style.blinking = true;
                self.ansi_handler_state
                    .event_proxy
                    .send_terminal_event(Event::CursorBlinkingChange(true));
            }
            ansi::Mode::SyncOutput => {}
        }
    }

    fn unset_mode(&mut self, mode: ansi::Mode) {
        log::trace!("Unsetting mode: {mode:?}");
        match mode {
            ansi::Mode::UrgencyHints => {
                self.ansi_handler_state.mode.remove(TermMode::URGENCY_HINTS)
            }
            ansi::Mode::SwapScreen { .. } => unreachable!("Handled in model layer"),
            ansi::Mode::ShowCursor => self.ansi_handler_state.mode.remove(TermMode::SHOW_CURSOR),
            ansi::Mode::CursorKeys => self.ansi_handler_state.mode.remove(TermMode::APP_CURSOR),
            ansi::Mode::ReportMouseClicks => {
                self.ansi_handler_state
                    .mode
                    .remove(TermMode::MOUSE_REPORT_CLICK);
                self.ansi_handler_state
                    .event_proxy
                    .send_terminal_event(Event::MouseCursorDirty);
            }
            ansi::Mode::ReportCellMouseMotion => {
                self.ansi_handler_state.mode.remove(TermMode::MOUSE_DRAG);
                self.ansi_handler_state
                    .event_proxy
                    .send_terminal_event(Event::MouseCursorDirty);
            }
            ansi::Mode::ReportAllMouseMotion => {
                self.ansi_handler_state.mode.remove(TermMode::MOUSE_MOTION);
                self.ansi_handler_state
                    .event_proxy
                    .send_terminal_event(Event::MouseCursorDirty);
            }
            ansi::Mode::ReportFocusInOut => {
                self.ansi_handler_state.mode.remove(TermMode::FOCUS_IN_OUT)
            }
            ansi::Mode::BracketedPaste => self
                .ansi_handler_state
                .mode
                .remove(TermMode::BRACKETED_PASTE),
            ansi::Mode::SgrMouse => self.ansi_handler_state.mode.remove(TermMode::SGR_MOUSE),
            ansi::Mode::Utf8Mouse => self.ansi_handler_state.mode.remove(TermMode::UTF8_MOUSE),
            ansi::Mode::AlternateScroll => self
                .ansi_handler_state
                .mode
                .remove(TermMode::ALTERNATE_SCROLL),
            ansi::Mode::LineWrap => self.ansi_handler_state.mode.remove(TermMode::LINE_WRAP),
            ansi::Mode::LineFeedNewLine => self
                .ansi_handler_state
                .mode
                .remove(TermMode::LINE_FEED_NEW_LINE),
            ansi::Mode::Origin => self.ansi_handler_state.mode.remove(TermMode::ORIGIN),
            ansi::Mode::DECCOLM => self.deccolm(),
            ansi::Mode::Insert => self.ansi_handler_state.mode.remove(TermMode::INSERT),
            ansi::Mode::BlinkingCursor => {
                self.ansi_handler_state.cursor_style.blinking = false;
                self.ansi_handler_state
                    .event_proxy
                    .send_terminal_event(Event::CursorBlinkingChange(false));
            }
            _ => {}
        }
    }

    fn set_scrolling_region(&mut self, top: usize, bottom: Option<usize>) {
        // Fallback to the last line as default.
        let bottom = bottom.unwrap_or_else(|| self.visible_rows());

        if top >= bottom {
            log::debug!("Invalid scrolling region: ({top};{bottom})");
            return;
        }

        // Bottom should be included in the range, but range end is not
        // usually included. One option would be to use an inclusive
        // range, but instead we just let the open range end be 1
        // higher.
        let start = VisibleRow(top - 1);
        let end = VisibleRow(bottom);

        log::trace!("Setting scrolling region: ({start};{end})");

        self.ansi_handler_state.scroll_region.start = min(start, VisibleRow(self.visible_rows()));
        self.ansi_handler_state.scroll_region.end = min(end, VisibleRow(self.visible_rows()));
        self.goto(VisibleRow(0), 0);
    }

    fn set_keypad_application_mode(&mut self) {
        self.ansi_handler_state.mode.insert(TermMode::APP_KEYPAD);
    }

    fn unset_keypad_application_mode(&mut self) {
        self.ansi_handler_state.mode.remove(TermMode::APP_KEYPAD);
    }

    fn set_active_charset(&mut self, index: ansi::CharsetIndex) {
        self.ansi_handler_state.active_charset = index;
    }

    fn configure_charset(&mut self, index: ansi::CharsetIndex, charset: ansi::StandardCharset) {
        self.grid.cursor.charsets[index] = charset;
    }

    fn set_color(&mut self, _: usize, _: warpui::color::ColorU) {
        log::error!("Handler method GridHandler::set_color should never be called. This should be handled by TerminalModel.");
    }

    fn dynamic_color_sequence<W: std::io::Write>(&mut self, _: &mut W, _: u8, _: usize, _: &str) {
        log::error!("Handler method GridHandler::dynamic_color_sequence should never be called. This should be handled by TerminalModel.");
    }

    fn reset_color(&mut self, _: usize) {
        log::error!("Handler method GridHandler::reset_color should never be called. This should be handled by TerminalModel.");
    }

    fn clipboard_store(&mut self, clipboard: u8, base64: &[u8]) {
        let clipboard_type = match clipboard {
            b'c' => ClipboardType::Clipboard,
            b'p' | b's' => ClipboardType::Selection,
            _ => return,
        };

        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(base64) {
            if let Ok(text) = String::from_utf8(bytes) {
                self.ansi_handler_state
                    .event_proxy
                    .send_terminal_event(Event::ClipboardStore(clipboard_type, text));
            }
        }
    }

    fn clipboard_load(&mut self, clipboard: u8, terminator: &str) {
        let clipboard_type = match clipboard {
            b'c' => ClipboardType::Clipboard,
            b'p' | b's' => ClipboardType::Selection,
            _ => return,
        };

        let terminator = terminator.to_owned();

        self.ansi_handler_state
            .event_proxy
            .send_terminal_event(Event::ClipboardLoad(
                clipboard_type,
                Arc::new(move |text| {
                    let base64 = base64::engine::general_purpose::STANDARD.encode(text);
                    format!("\x1b]52;{};{}{}", clipboard as char, base64, terminator)
                }),
            ));
    }

    fn decaln(&mut self) {
        log::trace!("Decalnning");

        self.grid.region_mut(..).each(|cell| {
            *cell = Cell::default();
            cell.c = 'E';
        });
    }

    fn push_title(&mut self) {
        log::error!("Handler method GridHandler::push_title should never be called. This should be handled by TerminalModel.");
    }

    fn pop_title(&mut self) {
        log::error!("Handler method GridHandler::pop_title should never be called. This should be handled by TerminalModel.");
    }

    fn text_area_size_pixels<W: std::io::Write>(&mut self, writer: &mut W) {
        let width = self.ansi_handler_state.cell_width * self.columns();
        let height = self.ansi_handler_state.cell_height * self.visible_rows();
        let _ = write!(writer, "\x1b[4;{height};{width}t");
    }

    fn text_area_size_chars<W: std::io::Write>(&mut self, writer: &mut W) {
        let _ = write!(writer, "\x1b[8;{};{}t", self.visible_rows(), self.columns());
    }

    fn precmd(&mut self, _: PrecmdValue) {
        unreachable!("Precmd hook is handled at block layer")
    }

    fn preexec(&mut self, _: PreexecValue) {
        unreachable!("Precmd hook is handled at block layer")
    }

    fn on_finish_byte_processing(&mut self, _: &ansi::ProcessorInput<'_>) {
        // Make sure the max cursor and dirty cell range are up-to-date.
        self.grid.update_max_cursor();
        self.update_dirty_cells_range();

        self.maybe_scan_dirty_cells_for_secrets();

        if self.finished && self.num_lines_truncated() > 0 {
            // Occasionally upon finishing the grid there are truncated rows that we have
            // not yet accounted for.
            self.refilter_lines();
        } else {
            self.maybe_filter_dirty_lines();
        }

        self.reset_dirty_cells_range_to_cursor_point();

        // Update bottommost visible content row for content_len() trimming.
        if self.track_content_length {
            self.bottommost_visible_content_row = self.bottommost_visible_content_row_backward();
        }

        if self.ansi_handler_state.is_alt_screen {
            debug_assert_eq!(
                self.flat_storage.total_rows(),
                0,
                "the alt screen grid should never put any rows in flat storage"
            );
        }
    }

    fn on_reset_grid(&mut self) {
        match &mut self.ansi_handler_state.reset_grid_checks {
            ResetGridChecks::Enabled { received_osc } => {
                debug_assert!(
                    !*received_osc,
                    "Grid has already received a Reset Grid OSC."
                );
                *received_osc = true;
            }
            ResetGridChecks::Disabled => (),
        }
    }

    fn handle_completed_iterm_image(&mut self, image: ITermImage) {
        if !FeatureFlag::ITermImages.is_enabled() {
            return;
        }

        if image.metadata.image_size.x() == 0.0 || image.metadata.image_size.y() == 0.0 {
            return;
        }

        if let Some((width, _)) = image.metadata.desired_width {
            if width == 0 {
                return;
            }
        }

        if let Some((height, _)) = image.metadata.desired_height {
            if height == 0 {
                return;
            }
        }

        let mut desired_width_px = image.metadata.image_size.x();
        let mut desired_height_px = image.metadata.image_size.y();

        // We may have received a desired dimension in the form of pixels, percent of pane, or # of cells.
        // We need to convert this into pixels given the current state of the terminal and its sizing.
        if let Some((width, width_units)) = image.metadata.desired_width {
            desired_width_px = match width_units {
                ITermImageDimensionUnit::Cell => {
                    ((self.ansi_handler_state.cell_width as u32) * width) as f32
                }
                ITermImageDimensionUnit::Percent => {
                    (self.columns() as f32)
                        * (self.ansi_handler_state.cell_width as f32)
                        * (width as f32)
                        * 0.01
                }
                ITermImageDimensionUnit::Pixel => width as f32,
            }
        }

        if let Some((height, height_units)) = image.metadata.desired_height {
            desired_height_px = match height_units {
                ITermImageDimensionUnit::Cell => {
                    ((self.ansi_handler_state.cell_height as u32) * height) as f32
                }
                ITermImageDimensionUnit::Percent => {
                    self.ansi_handler_state.pane_size.y() * (height as f32) * 0.01
                }
                ITermImageDimensionUnit::Pixel => height as f32,
            }
        }

        // The largest iTerm will render an image is the width of the terminal when it receives the image.
        // This logic ensures that when an image's size is calculated based on the current state of the terminal,
        // it will not be rendered partially off the right of the screen. At max, it will occupy the columns from the
        // cursor to the right edge.
        let max_width = (self.columns() as u32 - self.cursor_point().col as u32)
            * (self.ansi_handler_state.cell_width as u32);

        let desired_width_px = min(desired_width_px as u32, max_width);

        let max_height = MAX_IMAGE_CELL_HEIGHT * self.ansi_handler_state.cell_height as u32;

        let desired_height_px = min(desired_height_px as u32, max_height);

        let (width_px, height_px) = resize_dimensions(
            image.metadata.image_size.x() as u32,
            image.metadata.image_size.y() as u32,
            desired_width_px,
            desired_height_px,
            if image.metadata.preserve_aspect_ratio {
                FitType::Contain
            } else {
                FitType::Stretch
            },
        );

        let image_size = Vector2F::new(width_px as f32, height_px as f32);

        // Convert the visual dimension in pixels to cells. We want to round up if this doesn't perfectly fit within an amount of cells.
        let height_cells =
            (height_px as f32 / (self.ansi_handler_state.cell_height as f32)).ceil() as usize;

        // Convert the user requested dimension in pixels to cells. This is needed to scroll the cursor by the space the user requested.
        // If preserve_aspect_ratio is true, this may be larger than the visual dimension of the image.
        // We want to round up if this doesn't perfectly fit within an amount of cells.
        let (scroll_up_px, scroll_right_px) =
            if image.metadata.desired_height.is_some() && image.metadata.desired_width.is_some() {
                (desired_height_px as f32, desired_width_px as f32)
            } else {
                (height_px as f32, width_px as f32)
            };

        let scroll_up_cells =
            (scroll_up_px / (self.ansi_handler_state.cell_height as f32)).ceil() as usize;

        let scroll_right_cells =
            (scroll_right_px / (self.ansi_handler_state.cell_width as f32)).ceil() as usize;

        let image_id = image.metadata.id;
        let placement_id = rand::thread_rng().gen();

        self.ansi_handler_state
            .event_proxy
            .send_terminal_event(Event::ImageReceived {
                image_id,
                image_data: image.data,
                image_protocol: ImageProtocol::ITerm,
            });

        self.images.add_image_placement_data(
            image_id,
            placement_id,
            ImagePlacementData {
                z_index: 0,
                height_cells,
                image_size,
            },
        );

        self.images.place(
            image_id,
            placement_id,
            AbsolutePoint::from_point(self.cursor_point(), self),
            ImageType::ITerm,
            self.num_lines_truncated(),
        );

        // Create the whitespace to fit the image on.
        for _ in 0..scroll_up_cells - 1 {
            self.newline();
        }

        let num_cols = self.columns();

        // Move the cursor to the same row but after the image.
        self.move_cursor_forward(|cursor| {
            if cursor.point.col + scroll_right_cells < num_cols {
                cursor.point.col += scroll_right_cells;
            } else {
                cursor.input_needs_wrap = true;
            }
        });
    }

    fn handle_completed_kitty_action(
        &mut self,
        action: KittyAction,
        metadata: &mut HashMap<u32, StoredImageMetadata>,
    ) -> Option<KittyResponse> {
        Some(self.handle_completed_kitty_action_internal(action, metadata))
    }

    fn set_keyboard_enhancement_flags(
        &mut self,
        mode: KeyboardModes,
        apply: KeyboardModesApplyBehavior,
    ) {
        if !FeatureFlag::KittyKeyboardProtocol.is_enabled() {
            return;
        }
        self.set_keyboard_mode(mode, apply);
    }

    fn push_keyboard_enhancement_flags(&mut self, mode: KeyboardModes) {
        if !FeatureFlag::KittyKeyboardProtocol.is_enabled() {
            return;
        }
        self.push_keyboard_mode(mode);
    }

    fn pop_keyboard_enhancement_flags(&mut self, count: u16) {
        if !FeatureFlag::KittyKeyboardProtocol.is_enabled() {
            return;
        }
        self.pop_keyboard_modes(count);
    }

    fn query_keyboard_enhancement_flags<W: io::Write>(&mut self, writer: &mut W) {
        if !FeatureFlag::KittyKeyboardProtocol.is_enabled() {
            return;
        }
        // Respond with CSI ? flags u
        let mode = self.ansi_handler_state.keyboard_mode;
        let response = format!("\x1b[?{}u", mode.bits());
        let _ = writer.write_all(response.as_bytes());
    }
}

/// Helper functions for the [`ansi::Handler`] implementation.
impl GridHandler {
    /// Advances the cursor by one cell, handling wrapping appropriately.
    fn advance_cursor_by_one_cell(&mut self) {
        let num_cols = self.columns();

        self.move_cursor_forward(|cursor| {
            if cursor.point.col + 1 < num_cols {
                cursor.point.col += 1;
            } else {
                cursor.input_needs_wrap = true;
            }
        });
    }

    /// Insert a linebreak at the current cursor position.
    #[inline]
    pub(crate) fn wrapline(&mut self) {
        if !self.ansi_handler_state.mode.contains(TermMode::LINE_WRAP) {
            return;
        }

        log::trace!("Wrapping input");

        self.grid.cursor_cell().flags.insert(Flags::WRAPLINE);

        if (self.grid.cursor().point.row + 1) >= self.ansi_handler_state.scroll_region.end {
            self.linefeed();
        } else {
            self.move_cursor_forward(|cursor| {
                cursor.point.row += 1;
            });
        }

        // CORRECTNESS: Even though we're technically moving the cursor
        // backwards, the cursor is guaranteed to be ahead of where it
        // was at the start of `wrapline()`, which is the important thing
        // from a correctness perspective.
        self.move_cursor_forward_unchecked(|cursor| {
            cursor.point.col = 0;
            cursor.input_needs_wrap = false;
        });
    }

    /// Resets both halves of a wide character pair when `col` is at the
    /// **start** of an operation's range (i.e., `col` itself is about to be
    /// overwritten).  Handles both WIDE_CHAR_SPACER (resets the preceding
    /// WIDE_CHAR) and WIDE_CHAR (resets the following spacer).
    fn reset_wide_char_at_start_boundary(&mut self, row: usize, col: usize, bg: Color) {
        let num_cols = self.columns();
        let grid_row = &mut self.grid[row][..];

        if grid_row[col].flags.contains(Flags::WIDE_CHAR_SPACER) && col > 0 {
            // Reset the spacer and its WIDE_CHAR partner.
            grid_row[col] = bg.into();
            grid_row[col - 1] = bg.into();
        }

        if grid_row[col].flags.contains(Flags::WIDE_CHAR) && col + 1 < num_cols {
            // Reset the WIDE_CHAR and its spacer partner.
            grid_row[col] = bg.into();
            grid_row[col + 1] = bg.into();
        }
    }

    /// Resets a wide character pair when `col` is at the **end** of an
    /// operation's range (i.e., `col` is the first cell just past the range).
    /// Only handles WIDE_CHAR_SPACER, because a WIDE_CHAR at `col` means the
    /// entire pair is outside the range and should be left intact.
    fn reset_wide_char_at_end_boundary(&mut self, row: usize, col: usize, bg: Color) {
        let grid_row = &mut self.grid[row][..];

        if grid_row[col].flags.contains(Flags::WIDE_CHAR_SPACER) && col > 0 {
            // The spacer's WIDE_CHAR partner is inside the operation's range
            // and is being cleared, so reset the spacer too.
            grid_row[col] = bg.into();
            grid_row[col - 1] = bg.into();
        }
    }

    /// Write `c` to the cell at the cursor position.
    #[inline(always)]
    fn write_at_cursor(&mut self, c: char) -> &mut Cell {
        self.images.evict_images_at_point_with_type(
            AbsolutePoint::from_point(self.cursor_point(), self),
            &[ImageType::ITerm],
        );

        // If the cursor cell is part of a wide character pair, reset the
        // other half so we don't leave an orphaned flag.  The check is
        // inlined here (rather than delegating to
        // reset_wide_char_at_start_boundary) to avoid an opaque function
        // call inside this #[inline(always)] hot path — that call would
        // force the compiler to reload all self state from memory
        // afterward.
        let cursor_point = self.grid.cursor_point();
        let col = cursor_point.col;
        let row = cursor_point.row;
        let cell_flags = self.grid[row][col].flags;

        if cell_flags.intersects(Flags::WIDE_CHAR | Flags::WIDE_CHAR_SPACER) {
            let bg = self.grid.cursor().template.bg;
            let num_cols = self.columns();

            // Clear LEADING_WIDE_CHAR_SPACER on the previous row when
            // overwriting a wrapped wide char at cols 0-1.  A fresh write
            // during the normal wrapping flow has an empty cell, so the
            // outer intersects guard prevents us from touching a spacer
            // that was just placed.
            if col <= 1 && row > 0 {
                let is_wrapped = (col == 0 && cell_flags.contains(Flags::WIDE_CHAR))
                    || (col == 1 && cell_flags.contains(Flags::WIDE_CHAR_SPACER));
                if is_wrapped {
                    self.grid[row - 1][num_cols - 1]
                        .flags
                        .remove(Flags::LEADING_WIDE_CHAR_SPACER);
                }
            }

            // Reset the other half of the wide char pair.
            if cell_flags.contains(Flags::WIDE_CHAR_SPACER) && col > 0 {
                self.grid[row][col] = bg.into();
                self.grid[row][col - 1] = bg.into();
            } else if cell_flags.contains(Flags::WIDE_CHAR) && col + 1 < num_cols {
                self.grid[row][col] = bg.into();
                self.grid[row][col + 1] = bg.into();
            }
        }

        let c = self.grid.cursor().charsets[self.ansi_handler_state.active_charset].map(c);
        let fg = self.grid.cursor().template.fg;
        let bg = self.grid.cursor().template.bg;
        let flags = self.grid.cursor().template.flags;

        let cursor_cell = self.grid.cursor_cell();

        cursor_cell.drop_extra();

        cursor_cell.c = c;
        cursor_cell.fg = fg;
        cursor_cell.bg = bg;
        cursor_cell.flags = flags;

        cursor_cell
    }

    /// Scroll screen down.
    ///
    /// Text moves down; clear at bottom
    /// Expects origin to be in scroll range.
    #[inline]
    fn scroll_down_relative(&mut self, origin_row: VisibleRow, mut lines: usize) -> ScrollDelta {
        log::trace!("Scrolling down relative: origin={origin_row}, lines={lines}");

        lines = min(
            lines,
            self.ansi_handler_state.scroll_region.end - self.ansi_handler_state.scroll_region.start,
        );
        lines = min(
            lines,
            self.ansi_handler_state.scroll_region.end - origin_row,
        );

        let region = origin_row..self.ansi_handler_state.scroll_region.end;

        // Scroll between origin and bottom
        self.grid.scroll_down(&region, lines);

        ScrollDelta::Down { lines }
    }

    /// Scroll screen up
    ///
    /// Text moves up; clear at top
    /// Expects origin to be in scroll range.
    #[inline]
    fn scroll_up_relative(&mut self, origin_row: VisibleRow, mut lines: usize) -> ScrollDelta {
        log::trace!("Scrolling up relative: origin={origin_row}, lines={lines}");

        lines = min(
            lines,
            self.ansi_handler_state.scroll_region.end - self.ansi_handler_state.scroll_region.start,
        );

        let region = origin_row..self.ansi_handler_state.scroll_region.end;
        self.scroll_region_up(region, lines);
        ScrollDelta::Up { lines }
    }

    fn scroll_region_up(&mut self, region: Range<VisibleRow>, lines: usize) {
        // Move lines into scrollback.  We don't do this for the alt screen,
        // as it has no scrollback.
        if !self.ansi_handler_state.is_alt_screen {
            let range = region.start..std::cmp::min(region.end, region.start + lines);
            for row_idx in range.start.0..range.end.0 {
                self.flat_storage
                    .push_rows([&self.grid[VisibleRow(row_idx)]]);
            }
        }

        // Scroll from origin to bottom less number of lines.
        self.grid.scroll_up(&region, lines);

        // The cursor point implicitly grows when a line is added into scrollback,
        // so we need to make sure the dirty cells range accounts for this.
        self.update_dirty_cells_range();
    }

    fn deccolm(&mut self) {
        // Setting 132 column font makes no sense, but run the other side effects.
        // Clear scrolling region.
        ansi::Handler::set_scrolling_region(self, 1, None);

        // Clear grid.
        let bg = self.grid.cursor().template.bg;
        self.grid.region_mut(..).each(|cell| *cell = bg.into());
    }

    fn clear_viewport(&mut self) {
        // Determine how many lines to scroll up by.
        let end = Point {
            row: 0,
            col: self.columns(),
        };

        let mut cursor = self.grapheme_cursor_from(end, grapheme_cursor::Wrap::All);
        cursor.move_backward();
        while let Some(cursor_item) = cursor.current_item() {
            if !cursor_item.cell().is_empty() || cursor_item.point().row >= self.visible_rows() {
                break;
            }
            cursor.move_backward();
        }

        let row = cursor.last_valid_position().row;
        debug_assert!(row <= self.visible_rows());
        let positions = self.visible_rows() - row;
        let region = VisibleRow(0)..VisibleRow(self.visible_rows());

        self.scroll_region_up(region, positions);

        // Reset rotated lines.
        let template = self.grid.cursor().template.clone();
        for i in positions..self.visible_rows() {
            self.grid[i].reset(&template);
        }
    }

    fn clear_visible_rows_in_place(&mut self, bg: Color) {
        self.grid.region_mut(..).each(|cell| *cell = bg.into());
        let visible_start_row = self.history_size();
        let visible_end_row = visible_start_row + self.visible_rows();
        if visible_start_row < visible_end_row {
            self.images.evict_image_ids_between_points_with_type(
                AbsolutePoint::from_point(Point::new(visible_start_row, 0), self),
                AbsolutePoint::from_point(Point::new(visible_end_row - 1, usize::MAX), self),
                vec![ImageType::ITerm, ImageType::Kitty],
            );
            if self.columns() > 0 {
                self.clear_secrets_in_range(
                    Point::new(visible_start_row, 0)
                        ..=Point::new(visible_end_row - 1, self.columns() - 1),
                );
            }
        }
        self.clear_displayed_rows_and_filter_matches();
        if self.track_content_length {
            self.bottommost_visible_content_row = self.bottommost_visible_content_row_backward();
        }
        self.ansi_handler_state.dirty_cells_range =
            Point::new(visible_start_row, 0)..Point::new(visible_end_row, 0);
    }

    pub(in crate::terminal::model) fn disable_reset_grid_checks(&mut self) {
        self.ansi_handler_state.reset_grid_checks = ResetGridChecks::Disabled;
    }

    /// Marks the grid as having NOT received the Reset Grid OSC.
    /// This is useful for grids that expect to receive multiple OSCs.
    pub(in crate::terminal::model) fn reset_received_osc(&mut self) {
        if let ResetGridChecks::Enabled { received_osc } =
            &mut self.ansi_handler_state.reset_grid_checks
        {
            *received_osc = false;
        }
    }

    fn handle_completed_kitty_action_internal(
        &mut self,
        action: KittyAction,
        metadata: &mut HashMap<u32, StoredImageMetadata>,
    ) -> Result<(), KittyError> {
        if !FeatureFlag::KittyImages.is_enabled() {
            return Err(KittyError::KittyFeatureDisabled);
        }

        match action {
            KittyAction::StoreOnly(action) => {
                let metadata = match metadata.get(&action.image_id) {
                    Some(StoredImageMetadata::Kitty(metadata)) => metadata,
                    Some(_) | None => {
                        return Err(StorageError::UnknownId {
                            id: action.image_id,
                        }
                        .into())
                    }
                };

                if metadata.image_size.x() == 0.0 || metadata.image_size.y() == 0.0 {
                    return Ok(());
                }

                self.ansi_handler_state
                    .event_proxy
                    .send_terminal_event(Event::ImageReceived {
                        image_id: action.image_id,
                        image_data: action.image.data,
                        image_protocol: ImageProtocol::Kitty,
                    });
            }
            KittyAction::StoreAndDisplay(action) => {
                let metadata = match metadata.get(&action.image_id) {
                    Some(StoredImageMetadata::Kitty(metadata)) => metadata,
                    Some(_) | None => {
                        return Err(StorageError::UnknownId {
                            id: action.image_id,
                        }
                        .into())
                    }
                };

                if metadata.image_size.x() == 0.0 || metadata.image_size.y() == 0.0 {
                    return Ok(());
                }

                if let Some(0) = action.placement_data.cols {
                    return Ok(());
                }

                if let Some(0) = action.placement_data.rows {
                    return Ok(());
                }

                let max_width =
                    (self.columns() - self.cursor_point().col) * self.ansi_handler_state.cell_width;

                let max_height =
                    MAX_IMAGE_CELL_HEIGHT as usize * self.ansi_handler_state.cell_height;

                let desired_dimensions = action.placement_data.get_desired_dimensions(
                    metadata.image_size,
                    self.ansi_handler_state.cell_height,
                    self.ansi_handler_state.cell_width,
                    max_width,
                    max_height,
                );

                let (width_px, height_px) = resize_dimensions(
                    metadata.image_size.x() as u32,
                    metadata.image_size.y() as u32,
                    desired_dimensions.x() as u32,
                    desired_dimensions.y() as u32,
                    FitType::Stretch,
                );

                let image_size = Vector2F::new(width_px as f32, height_px as f32);

                // Convert the dimension in pixels to cells. We want to round up if this doesn't perfectly fit within an amount of cells.
                let height_cells = (height_px as f32 / (self.ansi_handler_state.cell_height as f32))
                    .ceil() as usize;
                let width_cells =
                    (width_px as f32 / (self.ansi_handler_state.cell_width as f32)).ceil() as usize;

                self.ansi_handler_state
                    .event_proxy
                    .send_terminal_event(Event::ImageReceived {
                        image_id: action.image_id,
                        image_data: action.image.data,
                        image_protocol: ImageProtocol::Kitty,
                    });

                self.images.add_image_placement_data(
                    action.image_id,
                    action.placement_id,
                    ImagePlacementData {
                        z_index: action.placement_data.z_index,
                        height_cells,
                        image_size,
                    },
                );

                self.images.place(
                    action.image_id,
                    action.placement_id,
                    AbsolutePoint::from_point(self.cursor_point(), self),
                    ImageType::Kitty,
                    self.num_lines_truncated(),
                );

                if !matches!(
                    action.placement_data.cursor_movement_policy,
                    CursorMovementPolicy::MoveCursor
                ) {
                    return Ok(());
                }

                // Create the whitespace to fit the image on.
                for _ in 0..height_cells - 1 {
                    self.newline();
                }

                let num_cols = self.columns();

                // Move the cursor to the same row but after the image.
                self.move_cursor_forward(|cursor| {
                    if cursor.point.col + width_cells < num_cols {
                        cursor.point.col += width_cells;
                    } else {
                        cursor.input_needs_wrap = true;
                    }
                });
            }
            KittyAction::DisplayStoredImage(action) => {
                let metadata = match metadata.get(&action.image_id) {
                    Some(StoredImageMetadata::Kitty(metadata)) => metadata,
                    Some(_) | None => {
                        return Err(StorageError::UnknownId {
                            id: action.image_id,
                        }
                        .into())
                    }
                };

                if let Some(0) = action.placement_data.cols {
                    return Ok(());
                }

                if let Some(0) = action.placement_data.rows {
                    return Ok(());
                }

                let max_width =
                    (self.columns() - self.cursor_point().col) * self.ansi_handler_state.cell_width;

                let max_height =
                    MAX_IMAGE_CELL_HEIGHT as usize * self.ansi_handler_state.cell_height;

                let desired_dimensions = action.placement_data.get_desired_dimensions(
                    metadata.image_size,
                    self.ansi_handler_state.cell_height,
                    self.ansi_handler_state.cell_width,
                    max_width,
                    max_height,
                );

                let (width_px, height_px) = resize_dimensions(
                    metadata.image_size.x() as u32,
                    metadata.image_size.y() as u32,
                    desired_dimensions.x() as u32,
                    desired_dimensions.y() as u32,
                    FitType::Stretch,
                );

                let image_size = Vector2F::new(width_px as f32, height_px as f32);

                // Convert the dimension in pixels to cells. We want to round up if this doesn't perfectly fit within an amount of cells.
                let height_cells = (height_px as f32 / (self.ansi_handler_state.cell_height as f32))
                    .ceil() as usize;
                let width_cells =
                    (width_px as f32 / (self.ansi_handler_state.cell_width as f32)).ceil() as usize;

                self.images.add_image_placement_data(
                    action.image_id,
                    action.placement_id,
                    ImagePlacementData {
                        z_index: action.placement_data.z_index,
                        height_cells,
                        image_size,
                    },
                );

                self.images.place(
                    action.image_id,
                    action.placement_id,
                    AbsolutePoint::from_point(self.cursor_point(), self),
                    ImageType::Kitty,
                    self.num_lines_truncated(),
                );

                if !matches!(
                    action.placement_data.cursor_movement_policy,
                    CursorMovementPolicy::MoveCursor
                ) {
                    return Ok(());
                }

                // Create the whitespace to fit the image on.
                for _ in 0..height_cells - 1 {
                    self.newline();
                }

                let num_cols = self.columns();

                // Move the cursor to the same row but after the image.
                self.move_cursor_forward(|cursor| {
                    if cursor.point.col + width_cells < num_cols {
                        cursor.point.col += width_cells;
                    } else {
                        cursor.input_needs_wrap = true;
                    }
                });
            }
            KittyAction::QuerySupport(_) => {}
            KittyAction::Delete { .. } => {}
        }

        Ok(())
    }
}
