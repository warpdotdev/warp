// The code in this file is adapted from the alacritty_terminal crate under the
// Apache license; see: crates/warp_terminal/src/model/LICENSE-ALACRITTY.

//! Grid resize and reflow.

use std::cmp::{min, Ordering};
use std::mem;

use crate::terminal::model::cell::{Cell, Flags};
use crate::terminal::model::grid::grid_storage::{Dimensions, GridStorage};
use crate::terminal::model::grid::row::Row;
use crate::terminal::model::index::{VisiblePoint, VisibleRow};

impl GridStorage {
    /// Resize the grid's width and/or height.
    pub fn resize(&mut self, reflow: bool, lines: usize, cols: usize, finished: bool) {
        if lines == 0 {
            return;
        }
        // Use empty template cell for resetting cells due to resize.
        let template = mem::take(&mut self.cursor.template);

        match self.columns.cmp(&cols) {
            Ordering::Less => self.grow_cols(reflow, cols),
            Ordering::Greater => self.shrink_cols(reflow, cols),
            Ordering::Equal => (),
        }

        // If the grid is finished, don't let the number of visible rows exceed
        // the number of rows to the cursor (including the cursor row).
        let lines = if finished {
            lines.min(self.cursor.point.row.0 + 1)
        } else {
            lines
        };

        match self.rows.cmp(&lines) {
            Ordering::Less => self.grow_lines(lines),
            Ordering::Greater => self.shrink_lines(lines),
            Ordering::Equal => (),
        }

        // Restore template cell.
        self.cursor.template = template;
    }

    /// Add lines to the visible area.
    ///
    /// Alacritty keeps the cursor at the bottom of the terminal as long as there
    /// is scrollback available. Once scrollback is exhausted, new lines are
    /// simply added to the bottom of the screen.
    fn grow_lines(&mut self, new_line_count: usize) {
        let lines_added = new_line_count - self.rows;

        // Need to resize before updating buffer.
        self.raw.grow_visible_lines(new_line_count);
        self.rows = new_line_count;

        let history_size = self.history_size();
        let from_history = min(history_size, lines_added);

        // Move existing lines up for every line that couldn't be pulled from history.
        if from_history != lines_added {
            let delta = lines_added - from_history;
            self.scroll_up(&(VisibleRow(0)..VisibleRow(new_line_count)), delta);
        }

        // Move cursor down for every line pulled from history.
        self.saved_cursor.point.row += from_history;
        self.cursor.point.row += from_history;
        self.max_cursor_point.row += from_history;

        self.decrease_scroll_limit(lines_added);
    }

    /// Remove lines from the visible area.
    ///
    /// The behavior in Terminal.app and iTerm.app is to keep the cursor at the
    /// bottom of the screen. This is achieved by pushing history "out the top"
    /// of the terminal window.
    ///
    /// Alacritty takes the same approach.
    pub(crate) fn shrink_lines(&mut self, target: usize) {
        // Scroll up to keep content inside the window.
        let required_scrolling = (self.cursor.point.row + 1).saturating_sub(target).0;
        let last_row = VisibleRow(target - 1);
        if required_scrolling > 0 {
            self.scroll_up(&(VisibleRow(0)..VisibleRow(self.rows)), required_scrolling);

            // Clamp cursors to the new viewport size.
            self.cursor.point.row = min(self.cursor.point.row, last_row);
        }

        // Clamp saved cursor, since only primary cursor is scrolled into viewport.
        self.saved_cursor.point.row = min(self.saved_cursor.point.row, last_row);

        if self.max_cursor_point.row > last_row {
            self.max_cursor_point.row = last_row;
        }

        self.raw.rotate((self.rows - target) as isize);
        self.raw.shrink_visible_lines(target);
        self.rows = target;
    }

    /// Grow number of columns in each row, reflowing if necessary.
    fn grow_cols(&mut self, reflow: bool, columns: usize) {
        // Check if a row needs to be wrapped.
        let should_reflow = |row: &Row| -> bool {
            let len = row.len();
            reflow && len > 0 && len < columns && row[len - 1].flags().contains(Flags::WRAPLINE)
        };

        self.columns = columns;

        let mut reversed: Vec<Row> = Vec::with_capacity(self.raw.len());
        let mut cursor_line_delta = 0_usize;

        // Remove the linewrap special case, by moving the cursor outside of the grid.
        if self.cursor.input_needs_wrap && reflow {
            self.cursor.input_needs_wrap = false;
            self.cursor.point.col += 1;
        }

        let mut rows = self.raw.take_all();
        let raw_total_rows_len = rows.len();

        for (i, mut row) in rows.drain(..).enumerate().rev() {
            // Check if reflowing should be performed.
            let last_row = match reversed.last_mut() {
                Some(last_row) if should_reflow(last_row) => last_row,
                _ => {
                    reversed.push(row);
                    continue;
                }
            };

            // Confirm that the last row is a wrapped row.
            debug_assert!(
                last_row[last_row.len() - 1]
                    .flags()
                    .contains(Flags::WRAPLINE),
                "Trying to reflow a non-wrapped row"
            );

            // Remove wrap flag before appending additional cells.
            if let Some(cell) = last_row.last_mut() {
                cell.flags_mut().remove(Flags::WRAPLINE);
            }

            // Remove leading spacers when reflowing wide char to the previous line.
            let mut last_len = last_row.len();
            if last_len >= 1
                && last_row[last_len - 1]
                    .flags()
                    .contains(Flags::LEADING_WIDE_CHAR_SPACER)
            {
                last_row.shrink(last_len - 1);
                last_len -= 1;
            }

            // Don't try to pull more cells from the next line than available.
            let mut num_wrapped = columns - last_len;
            let len = min(row.len(), num_wrapped);

            // Insert leading spacer when there's not enough room for reflowing wide char.
            let mut cells = if row[len - 1].flags().contains(Flags::WIDE_CHAR) {
                num_wrapped -= 1;

                let mut cells = row.front_split_off(len - 1);

                let mut spacer = Cell::default();
                spacer.flags_mut().insert(Flags::LEADING_WIDE_CHAR_SPACER);
                cells.push(spacer);

                cells
            } else {
                row.front_split_off(len)
            };

            // Add removed cells to previous row and reflow content.
            last_row.append(&mut cells);

            // We do not want to consider rows containing the end of prompt to be "clear", even if they are "empty".
            let row_is_clear = row.is_clear() && row.has_no_end_of_prompt_marker();

            // First, reflow the max_cursor position, if necessary
            let max_cursor_line = self.rows - self.max_cursor_point.row.0 - 1;
            if i == max_cursor_line && reflow {
                let target = self.max_cursor_point.wrapping_sub(columns, num_wrapped);
                self.max_cursor_point.col = target.col;
            } else if row_is_clear && i < max_cursor_line {
                self.max_cursor_point.row += 1;
            }

            // Next, reflow the actual cursor position - This has an impact on how much history is
            // pulled in, so it includes control flow not needed for max_cursor.
            let cursor_buffer_line = self.rows - self.cursor.point.row.0 - 1;
            if i == cursor_buffer_line && reflow {
                let visible_point_origin = VisiblePoint {
                    row: VisibleRow(0),
                    col: 0,
                };
                let mut adjust_cursor = false;

                // We want to adjust the cursor properly IF we need to pull a row from scrollback history.
                // This applies when:
                // 1. The cursor adjustment would incorrectly saturate at (0, 0) if no adjustment was made.
                // 2. We have at least 1 row in scrollback (raw_total_rows_len - self.rows >= 1)
                if self.resize_fix_ff_enabled
                    && row_is_clear
                    && self.cursor.point.wrapping_sub(columns, num_wrapped) == visible_point_origin
                    && self.cursor.point.wrapping_sub(columns, num_wrapped + 1)
                        == visible_point_origin
                    && raw_total_rows_len - self.rows >= 1
                {
                    // Fixes the bug of incorrect cursor position reflow in situations where we are
                    // re-growing the terminal window after shrinking it. Specifically, this results in
                    // scrollback history lines, which we want to pull back into the "visible lines".
                    // We rotate the cursor down to ensure that the wrapping subtraction logic below with
                    // `num_wrapped` can be completed correctly, without saturating at (0, 0) (which previously
                    // occurred, if the cursor isn't adjusted). Ultimately, the user-facing impact resulted in an
                    // incorrect cursor position comparison leading to incorrect block heights, since Warp
                    // erroneously believed a command to be "empty" (when comparing the "end of prompt" cursor
                    // to the "end of the command").
                    // Note that last_row is a wrapped line in this case (see `should_reflow` and `debug_assert` above)!
                    self.cursor.point.row += 1;
                    adjust_cursor = true;
                }
                // Resize cursor's line and reflow the cursor if necessary.
                let mut target = self.cursor.point.wrapping_sub(columns, num_wrapped);

                // Clamp to the last column, if no content was reflown with the cursor.
                if target.col == 0 && row_is_clear {
                    self.cursor.input_needs_wrap = true;
                    target = target.wrapping_sub(columns, 1);
                }
                self.cursor.point.col = target.col;

                // Get required cursor line changes. Since `num_wrapped` is smaller than `cols`
                // this will always be either `0` or `1`.
                let line_delta = self.cursor.point.row - target.row;

                if line_delta != 0 && row_is_clear {
                    if self.resize_fix_ff_enabled && adjust_cursor {
                        // We move the cursor up a line, if the current row is being entirely reflowed and removed.
                        self.cursor.point.row = self.cursor.point.row - line_delta;
                    }
                    continue;
                }

                cursor_line_delta += line_delta;
            } else if row_is_clear {
                // Rotate cursor down if content below them was pulled from history.
                if i < cursor_buffer_line {
                    self.cursor.point.row += 1;
                }

                // Don't push line into the new buffer.
                continue;
            }

            if let Some(cell) = last_row.last_mut() {
                // Set wrap flag if next line still has cells.
                cell.flags_mut().insert(Flags::WRAPLINE);
            }

            reversed.push(row);
        }

        // Make sure we have at least the viewport filled.
        if reversed.len() < self.rows {
            let delta = self.rows - reversed.len();
            self.cursor.point.row = self.cursor.point.row.saturating_sub(delta);
            self.max_cursor_point.row = self.max_cursor_point.row.saturating_sub(delta);
            reversed.resize_with(self.rows, || Row::new(columns));
        }

        // Pull content down to put cursor in correct position, or move cursor up if there's no
        // more lines to delete below the cursor.
        if cursor_line_delta != 0 {
            let cursor_buffer_line = self.rows - self.cursor.point.row.0 - 1;
            let available = min(cursor_buffer_line, reversed.len() - self.rows);
            let overflow = cursor_line_delta.saturating_sub(available);
            reversed.truncate(reversed.len() + overflow - cursor_line_delta);
            self.cursor.point.row = self.cursor.point.row.saturating_sub(overflow);
            self.max_cursor_point.row = self.max_cursor_point.row.saturating_sub(overflow);
        }

        // Reverse iterator and fill all rows that are still too short.
        let mut new_raw = Vec::with_capacity(reversed.len());
        for mut row in reversed.drain(..).rev() {
            if row.len() < columns {
                row.grow(columns);
            }
            new_raw.push(row);
        }

        self.raw.replace_inner(new_raw);

        // Confirm we haven't adjusted the cursor incorrectly into an invalid state (beyond max number of rows).
        debug_assert!(
            self.cursor.point.row <= VisibleRow(self.rows - 1),
            "Cursor in invalid state (beyond max number of rows)"
        );
    }

    /// Shrink number of columns in each row, reflowing if necessary.
    fn shrink_cols(&mut self, reflow: bool, columns: usize) {
        self.columns = columns;

        // Remove the linewrap special case, by moving the cursor outside of the grid.
        if self.cursor.input_needs_wrap && reflow {
            self.cursor.input_needs_wrap = false;
            self.cursor.point.col += 1;
        }

        let mut new_raw = Vec::with_capacity(self.raw.len());
        let mut buffered: Option<Vec<Cell>> = None;

        let mut rows = self.raw.take_all();
        for (i, mut row) in rows.drain(..).enumerate().rev() {
            // Append lines left over from the previous row.
            if let Some(buffered) = buffered.take() {
                // Add a column for every cell added before the cursor, if it goes beyond the new
                // width it is then later reflown.
                let cursor_buffer_line = self.rows - self.cursor.point.row.0 - 1;
                if i == cursor_buffer_line {
                    self.cursor.point.col += buffered.len();
                }

                // Also add to the max_cursor position in the same manner
                let max_cursor_line = self.rows - self.max_cursor_point.row.0 - 1;
                if i == max_cursor_line {
                    self.max_cursor_point.col += buffered.len();
                }

                row.append_front(buffered);
            }

            loop {
                // Remove all cells which require reflowing.
                let mut wrapped = match row.shrink(columns) {
                    Some(wrapped) if reflow => wrapped,
                    _ => {
                        let cursor_buffer_line =
                            self.rows.saturating_sub(self.cursor.point.row.0 + 1);
                        let max_cursor_line =
                            self.rows.saturating_sub(self.max_cursor_point.row.0 + 1);
                        if reflow
                            && ((i == cursor_buffer_line && self.cursor.point.col >= columns)
                                || (i == max_cursor_line && self.max_cursor_point.col >= columns))
                        {
                            // If there are empty cells before the cursor or max_cursor, we assume
                            // it is explicit whitespace and need to wrap it like normal content.
                            Vec::new()
                        } else {
                            // Since it fits, just push the existing line without any reflow.
                            new_raw.push(row);
                            break;
                        }
                    }
                };

                // Insert spacer if a wide char would be wrapped into the last column.
                if row.len() >= columns && row[columns - 1].flags().contains(Flags::WIDE_CHAR) {
                    let mut spacer = Cell::default();
                    spacer.flags_mut().insert(Flags::LEADING_WIDE_CHAR_SPACER);

                    let wide_char = mem::replace(&mut row[columns - 1], spacer);
                    wrapped.insert(0, wide_char);
                }

                // Remove wide char spacer before shrinking.
                let len = wrapped.len();
                if len > 0
                    && wrapped[len - 1]
                        .flags()
                        .contains(Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    if len == 1 {
                        row[columns - 1].flags_mut().insert(Flags::WRAPLINE);
                        new_raw.push(row);
                        break;
                    } else {
                        // Remove the leading spacer from the end of the wrapped row.
                        wrapped[len - 2].flags_mut().insert(Flags::WRAPLINE);
                        wrapped.truncate(len - 1);
                    }
                }

                new_raw.push(row);

                // Set line as wrapped if cells got removed.
                if let Some(cell) = new_raw.last_mut().and_then(|r| r.last_mut()) {
                    cell.flags_mut().insert(Flags::WRAPLINE);
                }

                if wrapped
                    .last()
                    .map(|c| c.flags().contains(Flags::WRAPLINE) && i >= 1)
                    .unwrap_or(false)
                    && wrapped.len() < columns
                {
                    // Make sure previous wrap flag doesn't linger around.
                    if let Some(cell) = wrapped.last_mut() {
                        cell.flags_mut().remove(Flags::WRAPLINE);
                    }

                    // Add removed cells to start of next row.
                    buffered = Some(wrapped);
                    break;
                } else {
                    // Reflow cursor if a line below it is deleted.
                    let cursor_buffer_line = self.rows - self.cursor.point.row.0 - 1;
                    if (i == cursor_buffer_line && self.cursor.point.col < columns)
                        || i < cursor_buffer_line
                    {
                        self.cursor.point.row = self.cursor.point.row.saturating_sub(1);
                    }

                    // Reflow the cursor if it is on this line beyond the width.
                    if i == cursor_buffer_line && self.cursor.point.col >= columns {
                        // Since only a single new line is created, we subtract only `cols`
                        // from the cursor instead of reflowing it completely.
                        self.cursor.point.col -= columns;
                    }

                    // Reflow max_cursor if a line below it is deleted
                    let max_cursor_line = self.rows - self.max_cursor_point.row.0 - 1;
                    if (i == max_cursor_line && self.max_cursor_point.col < columns)
                        || i < max_cursor_line
                    {
                        self.max_cursor_point.row = self.max_cursor_point.row.saturating_sub(1);
                    }

                    // Reflow the max cursor if it is on this line beyond the width.
                    if i == max_cursor_line && self.max_cursor_point.col >= columns {
                        self.max_cursor_point.col -= columns;
                    }

                    // Make sure new row is at least as long as new width.
                    let occ = wrapped.len();
                    if occ < columns {
                        wrapped.resize_with(columns, Cell::default);
                    }
                    row = Row::from_vec(wrapped, occ);
                }
            }
        }

        // Reverse iterator and use it as the new grid storage.
        let mut reversed: Vec<Row> = new_raw.drain(..).rev().collect();
        reversed.truncate(self.max_scroll_limit + self.rows);
        self.raw.replace_inner(reversed);

        // Reflow the cursor and max_cursor positions, clamping if reflow is disabled
        if !reflow {
            self.cursor.point.col = min(self.cursor.point.col, columns - 1);
            self.max_cursor_point.col = min(self.max_cursor_point.col, columns - 1);
        } else {
            if self.cursor.point.col == columns
                && !self[self.cursor_point().row][columns - 1]
                    .flags
                    .contains(Flags::WRAPLINE)
            {
                self.cursor.input_needs_wrap = true;
                self.cursor.point.col -= 1;
            } else {
                self.cursor.point = self.cursor.point.wrap(columns);
            }
            self.max_cursor_point = self.max_cursor_point.wrap(columns);
        }

        // Clamp the saved cursor to the grid.
        self.saved_cursor.point.col = min(self.saved_cursor.point.col, columns - 1);
    }
}
