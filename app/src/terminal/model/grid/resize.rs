// The code in this file is adapted from the alacritty_terminal crate under the
// Apache license; see: crates/warp_terminal/src/model/LICENSE-ALACRITTY.

use string_offset::ByteOffset;
use warp_terminal::model::{
    grid::{
        cell::{self, LineLength as _},
        Dimensions as _,
    },
    Point, VisiblePoint, VisibleRow,
};

use crate::terminal::{model::grid::Cursor, SizeInfo};

use super::GridHandler;

impl GridHandler {
    /// Resize terminal to new dimensions.
    pub fn resize(&mut self, size: SizeInfo) {
        self.ansi_handler_state.cell_width = size.cell_width_px.as_f32() as usize;
        self.ansi_handler_state.cell_height = size.cell_height_px.as_f32() as usize;

        let old_cols = self.columns();
        let old_rows = self.visible_rows();

        let num_cols = size.columns();
        let num_rows = size.rows();

        if old_cols == num_cols && old_rows == num_rows {
            log::debug!("Term::resize dimensions unchanged");
            return;
        }

        if num_rows == 0 {
            log::debug!("Ignoring resize down to zero visible lines");
            return;
        }

        log::debug!("New num_cols is {num_cols} and num_lines is {num_rows}");

        if old_cols != num_cols {
            // Recreate tabs list.
            self.ansi_handler_state.tabs.resize(num_cols);
        }

        // Resize the internal storage structures.
        self.resize_storage(num_rows, num_cols);

        // Reset scrolling region.
        self.ansi_handler_state.scroll_region = VisibleRow(0)..VisibleRow(self.visible_rows());

        // If the current grid has secrets, we now need to rescan the grid to refind any secrets.
        if !self.secrets.is_empty() {
            self.scan_for_secrets_after_resize();
        }

        // Re-apply the grid filter, if one exists.
        self.refilter_lines();
    }

    pub(super) fn resize_storage(&mut self, num_rows: usize, num_cols: usize) {
        use std::cmp::min;

        // If this is the alt screen, we can skip reflowing the grid and simply
        // adjust the size of rows.
        if self.ansi_handler_state.is_alt_screen {
            // We should never finish the alt screen grid.
            debug_assert!(!self.finished);
            // We can delegate to the old grid resizing logic, as there's no
            // flat storage for the alt screen.
            self.grid.resize(false, num_rows, num_cols, self.finished);
            return;
        }

        // Store information about the initial cursor position in the grid.
        let cursor = InitialCursorState::new(
            self.grid.cursor.point,
            self.grid.cursor.input_needs_wrap,
            self,
        );
        let saved_cursor = InitialCursorState::new(
            self.grid.saved_cursor.point,
            self.grid.saved_cursor.input_needs_wrap,
            self,
        );
        let max_cursor = InitialCursorState::new(self.grid.max_cursor_point, false, self);

        // Push all rows from grid storage into flat storage.  We make sure not
        // to truncate rows that exceed the maximum scrollback size, as we only
        // want to apply that limit after we've pulled rows back out into the
        // grid.
        for row_idx in 0..self.grid.total_rows() {
            self.flat_storage
                .push_rows_without_truncation([&self.grid[VisibleRow(row_idx)]]);
        }

        // Now that all data is in flat storage, convert the cursor state to
        // reference a flat storage content offset.
        let cursor = cursor.into_content_offset(self);
        let saved_cursor = saved_cursor.into_content_offset(self);
        let max_cursor = max_cursor.into_content_offset(self);

        // Resize flat storage.
        self.flat_storage.set_columns(num_cols);

        // If the grid is finished, don't let the number of visible rows exceed
        // the number of total rows (i.e.: if we can't pop a full num_rows
        // from flat storage, limit visible_rows to the number of rows we
        // _could_ pop).
        let visible_rows = if self.finished {
            num_rows.min(self.flat_storage.total_rows())
        } else {
            num_rows
        };

        // Convert back from a content offset to an actual cursor position.
        let cursor = cursor.into_cursor_point(num_cols, self);
        let saved_cursor = saved_cursor.into_cursor_point(num_cols, self);
        let max_cursor = max_cursor.into_cursor_point(num_cols, self);

        // If we're reducing the number of visible rows, we want to first drop
        // rows after the cursor before we start pushing rows into scrollback.
        //
        // It's easiest to think about this in the context of a traditional
        // terminal.  If the window has a height of 10 rows but only 5 rows of
        // content, those 5 rows will be at the top of the window.  If the
        // window is resized to be 8 rows tall, the bottom two rows of the grid
        // will be truncated.
        //
        // Here, we eliminate those final rows by not pushing them back into
        // grid storage after pulling them out of flat storage.
        let rows_after_cursor = self
            .flat_storage
            .total_rows()
            .saturating_sub(cursor.row() + 1);
        let shrink_amount = self.visible_rows().saturating_sub(num_rows);
        let rows_to_drop = shrink_amount.min(rows_after_cursor);
        let rows_to_pop = visible_rows + rows_to_drop;

        // Pop the rows from the bottom of flat storage, and drop some of them
        // if necessary.
        //
        // Note: This may produce a Vec with len < num_rows.
        let mut grid_rows = self.flat_storage.pop_rows(rows_to_pop);
        grid_rows.truncate(grid_rows.len().saturating_sub(rows_to_drop));

        // Set `GridStorage` contents to the given number of rows.
        self.grid.set_stored_rows(grid_rows, visible_rows, num_cols);

        // Set the new cursor positions.
        let history_size = self.history_size();
        cursor.update_cursor(&mut self.grid.cursor, history_size);
        saved_cursor.update_cursor(&mut self.grid.saved_cursor, history_size);
        self.grid.max_cursor_point = max_cursor.into_visible_point(self);

        // Clamp cursors to the new visible region.
        //
        // TODO(vorporeal): This can lead to a `max_cursor_point` that has
        // content after it.  We should decide if this is something important
        // to fix or not.  (The behavior is inherited from grid storage resize
        // logic.)
        let last_row = VisibleRow(visible_rows - 1);
        self.grid.cursor.point.row = min(self.grid.cursor.point.row, last_row);
        self.grid.max_cursor_point.row = min(self.grid.max_cursor_point.row, last_row);
        self.grid.saved_cursor.point.row = min(self.grid.saved_cursor.point.row, last_row);

        // Finally, make sure we don't have too many rows in scrollback.
        self.flat_storage.apply_max_rows();
    }
}

#[derive(Debug)]
enum InitialCursorState {
    /// Cursor is at some point in the grid.
    AtPoint(Point),
    /// Cursor is at the cell after the given point in the grid.
    ///
    /// We set this in two situations:
    /// 1. When the cursor has `input_needs_wrap = True`, and
    /// 2. When the cursor is over an empty cell, we describe it as being after
    ///    the preceding cell.  This allows us to set `input_needs_wrap`
    ///    properly when doing the final conversion back to a cursor.
    AtCellAfterPoint(Point),
}

impl InitialCursorState {
    fn new(mut cursor_point: VisiblePoint, input_needs_wrap: bool, grid: &mut GridHandler) -> Self {
        // Start by clamping the cursor to the visible region of the grid, just
        // in case some bug causes it to end up in an invalid place.
        if cursor_point.row.0 >= grid.visible_rows() {
            #[cfg(debug_assertions)]
            log::error!(
                "cursor should not be outside the bounds of the grid! \
                 cursor at ({}, {}) but grid has {} rows and {} columns",
                cursor_point.row,
                cursor_point.col,
                grid.total_rows(),
                grid.columns()
            );
            cursor_point.row.0 = grid.visible_rows() - 1;
        }
        if cursor_point.col >= grid.columns() {
            #[cfg(debug_assertions)]
            log::error!(
                "cursor should not be outside the bounds of the grid! \
                 cursor at ({}, {}) but grid has {} rows and {} columns",
                cursor_point.row,
                cursor_point.col,
                grid.total_rows(),
                grid.columns()
            );
            cursor_point.col = grid.columns() - 1;
        }

        let history_size = grid.history_size();
        let mut point = Point {
            row: cursor_point.row.0 + history_size,
            col: cursor_point.col,
        };
        let mut cell_after_point = false;

        let row = &grid.grid[cursor_point.row];

        let cell_follows_newline = |point: Point| -> bool {
            // The cell cannot follow a newline unless it's the first cell in the row.
            if point.col > 0 {
                return false;
            }

            // If the cursor is at the first cell in the first row, treat it as if it follows a newline.
            let Some(prev_row_idx) = point.row.checked_sub(1) else {
                return true;
            };

            // The cell follows a newline if the previous row does not wrap.
            !grid.row_wraps(prev_row_idx)
        };

        if input_needs_wrap {
            // If the input needs wrapping, the target cell is the one
            // after the current cursor point.
            cell_after_point = true;
        } else if row[point.col].c == cell::DEFAULT_CHAR
            && point.col >= row.line_length()
            && !cell_follows_newline(point)
        {
            // If the cursor is on an empty cell at the end of a row and could
            // wrap back to the previous cell, track it relative to the
            // previous cell.  This allows us to set `Cursor.input_needs_wrap`
            // instead of putting the cursor at the start of an
            // otherwise-unneeded blank row.
            cell_after_point = true;
            point = point.wrapping_sub(grid.columns(), 1);
        }

        // Mark the location of the cursor within the grid, to ensure that
        // the cell under the cursor exists post-resize.
        if let Some(super::StorageRow::GridStorage(row_idx)) = grid.storage_row(point.row) {
            grid.grid[row_idx][point.col]
                .flags
                .insert(cell::Flags::HAS_CURSOR);
        }

        if cell_after_point {
            Self::AtCellAfterPoint(point)
        } else {
            Self::AtPoint(point)
        }
    }

    fn into_content_offset(self, grid: &GridHandler) -> CursorContentOffset {
        match self {
            Self::AtPoint(point) => {
                let content_offset =
                    Self::resolve_content_offset(point, grid, "InitialCursorState::AtPoint");
                CursorContentOffset::AtPoint(content_offset)
            }
            Self::AtCellAfterPoint(point) => {
                let content_offset = Self::resolve_content_offset(
                    point,
                    grid,
                    "InitialCursorState::AtCellAfterPoint",
                );
                CursorContentOffset::AtCellAfterPoint(content_offset)
            }
        }
    }

    /// Looks up `point` in `grid.flat_storage` and returns its byte offset, or
    /// falls back to byte offset 0 (start of content) on error.
    ///
    /// `flat_storage.content_offset_at_point` errors when the point is outside
    /// the storage bounds or refers to an empty cell after the end of a
    /// hard-wrapped line. Both can happen during resize when the cursor's
    /// pre-resize point doesn't survive the layout change (e.g. the cursor was
    /// past the end of a line that's no longer wrapped, or in a row that's
    /// been folded into history).
    ///
    /// Previously this was an `.expect("should have a content offset for
    /// point")`, which crashed the app with SIGABRT during resize for users
    /// hitting the edge case (GH-9013, 8+ reports in one week, all with the
    /// same stack). Now we log the failure and fall back to byte offset 0,
    /// which makes the cursor land at the top-left of the resized grid — a
    /// recoverable, user-correctable state instead of a hard crash.
    fn resolve_content_offset(point: Point, grid: &GridHandler, source: &str) -> ByteOffset {
        match grid.flat_storage.content_offset_at_point(point) {
            Ok(offset) => offset,
            Err(err) => {
                log::warn!(
                    "GH-9013: {source} resize fell back to ByteOffset(0) because \
                     `content_offset_at_point({point:?})` failed: {err:?}"
                );
                ByteOffset::default()
            }
        }
    }
}

enum CursorContentOffset {
    /// Cursor is at the location with the given byte offset in flat storage.
    AtPoint(ByteOffset),
    /// Cursor is at cell _after_ the location with the given byte offset in
    /// flat storage.  This helps ensure we properly handle `input_needs_wrap`
    /// cases.
    AtCellAfterPoint(ByteOffset),
}

impl CursorContentOffset {
    fn into_cursor_point(self, new_cols: usize, grid: &GridHandler) -> FinalCursorState {
        match self {
            Self::AtPoint(byte_offset) => FinalCursorState::AtPoint(Self::resolve_point(
                byte_offset,
                grid,
                "CursorContentOffset::AtPoint",
            )),
            Self::AtCellAfterPoint(byte_offset) => {
                let mut point =
                    Self::resolve_point(byte_offset, grid, "CursorContentOffset::AtCellAfterPoint");
                // All data is in flat storage at the moment, so we need to
                // explicitly ask it about row wrapping.
                let input_needs_wrap =
                    point.col == new_cols - 1 && !grid.flat_storage.row_wraps(point.row);
                if !input_needs_wrap {
                    point = point.wrapping_add(new_cols, 1);
                }
                FinalCursorState::AtCellAfterPoint {
                    point,
                    input_needs_wrap,
                }
            }
        }
    }

    /// Looks up the grid `Point` for `byte_offset`, falling back to
    /// `Point::default()` (origin) on error.
    ///
    /// Mirrors [`InitialCursorState::resolve_content_offset`] in spirit: the
    /// underlying call can fail when the offset is outside stored content or
    /// points at a non-cell location (e.g. a newline). Previously this was an
    /// `.expect("content offset should be valid")` and would crash the app
    /// during resize edge cases — see GH-9013.
    fn resolve_point(byte_offset: ByteOffset, grid: &GridHandler, source: &str) -> Point {
        match grid.flat_storage.content_offset_to_point(byte_offset) {
            Ok(point) => point,
            Err(err) => {
                log::warn!(
                    "GH-9013: {source} resize fell back to Point::default() because \
                     `content_offset_to_point({byte_offset:?})` failed: {err:?}"
                );
                Point::default()
            }
        }
    }
}

enum FinalCursorState {
    AtPoint(Point),
    AtCellAfterPoint {
        point: Point,
        input_needs_wrap: bool,
    },
}

impl FinalCursorState {
    fn update_cursor(self, cursor: &mut Cursor, history_size: usize) {
        let (point, input_needs_wrap) = match self {
            FinalCursorState::AtPoint(point) => (point, false),
            FinalCursorState::AtCellAfterPoint {
                point,
                input_needs_wrap,
            } => (point, input_needs_wrap),
        };

        cursor.point = point.to_visible_point(history_size);
        cursor.input_needs_wrap = input_needs_wrap;
    }

    fn into_visible_point(self, grid: &GridHandler) -> VisiblePoint {
        let (Self::AtPoint(point) | Self::AtCellAfterPoint { point, .. }) = self;
        point.to_visible_point(grid.history_size())
    }

    fn row(&self) -> usize {
        let (Self::AtPoint(point) | Self::AtCellAfterPoint { point, .. }) = self;
        point.row
    }
}
