// The code in this file is adapted from the alacritty_terminal crate under the
// Apache license; see: crates/warp_terminal/src/model/LICENSE-ALACRITTY.

//! A specialized 2D grid implementation optimized for use in a terminal.

mod resize;

use std::cmp::min;
use std::ops::{Index, IndexMut, Range};

use serde::{Deserialize, Serialize};
pub use warp_terminal::model::grid::Dimensions;

use crate::features::FeatureFlag;
use crate::terminal::model::ansi::{CharsetIndex, StandardCharset};
use crate::terminal::model::cell::{Cell, Flags};
use crate::terminal::model::grid::row::Row;
use crate::terminal::model::grid::storage::Storage;
use crate::terminal::model::index::{IndexRange, Point, VisiblePoint, VisibleRow};
use crate::terminal::model::secrets::ObfuscateSecrets;

impl ::std::cmp::PartialEq for GridStorage {
    fn eq(&self, other: &Self) -> bool {
        // Compare struct fields and check result of grid comparison.
        self.raw.eq(&other.raw) && self.columns.eq(&other.columns) && self.rows.eq(&other.rows)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Cursor {
    /// The location of this cursor.
    pub point: VisiblePoint,

    /// Template cell when using this cursor.
    pub template: Cell,

    /// Currently configured graphic character sets.
    pub charsets: Charsets,

    /// Tracks if the next call to input will need to first handle wrapping.
    ///
    /// This is true after the last column is set with the input function. Any function that
    /// implicitly sets the line or column needs to set this to false to avoid wrapping twice.
    ///
    /// Tracking `input_needs_wrap` makes it possible to not store a cursor position that exceeds
    /// the number of columns, which would lead to index out of bounds when interacting with arrays
    /// without sanitization.
    pub input_needs_wrap: bool,
}

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct Charsets([StandardCharset; 4]);

impl Index<CharsetIndex> for Charsets {
    type Output = StandardCharset;

    fn index(&self, index: CharsetIndex) -> &StandardCharset {
        &self.0[index as usize]
    }
}

impl IndexMut<CharsetIndex> for Charsets {
    fn index_mut(&mut self, index: CharsetIndex) -> &mut StandardCharset {
        &mut self.0[index as usize]
    }
}

/// Grid based terminal content storage.
///
/// The grid is a 0-based buffer that goes from index 0 to however large the grid needs to be.
///
///   ┌────────────────────────────────┐
/// 0:│cat best_shakespeare_plays.txt  │ <-- command grid
///   ├────────────────────────────────┤
/// 0:│Hamlet                          │
/// 1:│Othello                         │
/// 2:│Macbeth                         │ <-- output grid
/// 3:│The Tempest                     │
/// 4:│Pericles                        │
///   └────────────────────────────────┘
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GridStorage {
    /// Current cursor for writing data.
    #[serde(skip)]
    pub(super) cursor: Cursor,

    /// The maximum line and column the cursor has been on.
    #[serde(skip)]
    pub max_cursor_point: VisiblePoint,

    /// Last saved cursor.
    #[serde(skip)]
    pub saved_cursor: Cursor,

    /// VisibleRows in the grid. Each row holds a list of cells corresponding to the
    /// columns in that row.
    pub(super) raw: Storage,

    /// Number of columns.
    pub(crate) columns: usize,

    /// Number of visible rows.
    pub(crate) rows: usize,

    /// Maximum number of lines in history.
    pub(crate) max_scroll_limit: usize,

    #[serde(skip)]
    /// The number of lines that have been truncated due
    /// to the [`Grid::max_scroll_limit`].
    ///
    /// This is specifically of type [`u64`] to ensure
    /// that we don't overflow for blocks with a lot of truncation.
    /// We might want to consider using a smalluint if this becomes a problem.
    pub(super) num_lines_truncated: u64,

    /// Whether the resize fix feature flag is enabled (Alacritty cursor reflow bug during resize). Gating
    /// fix to gain confidence in the fix before enabling for all users.
    #[serde(skip)]
    pub(crate) resize_fix_ff_enabled: bool,
}

impl GridStorage {
    pub fn new(
        rows: usize,
        columns: usize,
        max_scroll_limit: usize,
        // TODO(vorporeal): remove this argument entirely
        _secret_obfuscation_mode: ObfuscateSecrets,
    ) -> GridStorage {
        GridStorage {
            raw: Storage::with_capacity(rows, columns, FeatureFlag::SequentialStorage.is_enabled()),
            max_scroll_limit,
            saved_cursor: Cursor::default(),
            cursor: Cursor::default(),
            max_cursor_point: Default::default(),
            rows,
            columns,
            resize_fix_ff_enabled: FeatureFlag::ResizeFix.is_enabled(),
            num_lines_truncated: 0,
        }
    }

    /// Constructs a new [`GridStorage`] with a subset of rows from `self`.
    ///
    /// `num_preceding_rows` is the number of rows in `self` that come before
    /// the rows in `rows`.
    ///
    /// `initial_history_size` is the number of rows in the parent
    /// [`GridHandler`] that are in scrollback.  This is needed to properly
    /// convert [`VisiblePoint`]s to [`Point`]s.
    pub(in crate::terminal::model) fn new_for_split(
        &self,
        mut rows: Vec<Row>,
        num_preceding_rows: usize,
        initial_history_size: usize,
    ) -> GridStorage {
        let visible_lines = rows.len();

        // If we're not using sequential storage, we store the rows in reverse
        // order, so reverse them here.
        if !self.raw.is_sequential() {
            rows.reverse();
        }

        let mut grid = GridStorage {
            cursor: self.cursor.clone(),
            max_cursor_point: self.max_cursor_point,
            saved_cursor: self.saved_cursor.clone(),
            raw: Storage::with_rows(rows, self.raw.is_sequential(), visible_lines),
            columns: self.columns,
            rows: visible_lines,
            max_scroll_limit: self.max_scroll_limit,
            resize_fix_ff_enabled: self.resize_fix_ff_enabled,
            num_lines_truncated: 0,
        };

        let new_history_size = grid.history_size();
        let new_visible_rows = grid.visible_rows();
        let columns = grid.columns();

        let adjust_cursor_point = |cursor_point: &mut VisiblePoint| {
            // Convert the cursor row to absolute coordinates, using the
            // pre-split history size from the `GridHandler` level (so that we
            // account for rows in flat storage).
            let row = cursor_point.row.0 + initial_history_size;

            // Check if the cursor is within this grid or before it.
            if let Some(row) = row.checked_sub(num_preceding_rows) {
                let visible_row = row.saturating_sub(new_history_size);
                if visible_row < new_visible_rows {
                    // If the cursor is within this grid, update the row
                    // accordingly.
                    cursor_point.row = VisibleRow(visible_row);
                } else {
                    // If it is past the end of the grid, put it at the end.
                    cursor_point.row = VisibleRow(new_visible_rows - 1);
                    cursor_point.col = columns - 1;
                }
            } else {
                // The cursor is before this grid, so put it at the start of
                // the grid.
                cursor_point.row = VisibleRow(0);
                cursor_point.col = 0;
            }
        };

        adjust_cursor_point(&mut grid.cursor.point);
        adjust_cursor_point(&mut grid.saved_cursor.point);
        adjust_cursor_point(&mut grid.max_cursor_point);

        grid
    }

    pub(super) fn set_stored_rows(
        &mut self,
        mut rows: Vec<Row>,
        visible_rows: usize,
        columns: usize,
    ) {
        // Ensure there are a full set of rows.
        rows.resize_with(visible_rows, || Row::new(columns));
        debug_assert_eq!(rows.len(), visible_rows);

        self.rows = visible_rows;
        self.columns = columns;

        let is_sequential = self.raw.is_sequential();
        if !is_sequential {
            rows.reverse();
        }

        self.raw = Storage::with_rows(rows, is_sequential, visible_rows);
    }

    /// Update the size of the scrollback history.
    pub fn update_history(&mut self, history_size: usize) {
        let current_history_size = self.history_size();
        if current_history_size > history_size {
            self.raw.shrink_lines(current_history_size - history_size);
        }
        self.max_scroll_limit = history_size;
    }

    fn increase_scroll_limit(&mut self, count: usize) {
        let count = min(count, self.max_scroll_limit - self.history_size());
        if count != 0 {
            self.raw.initialize(count, self.columns);
        }
    }

    pub(crate) fn decrease_scroll_limit(&mut self, count: usize) {
        let count = min(count, self.history_size());
        if count != 0 {
            self.raw.shrink_lines(min(count, self.history_size()));
        }
    }

    pub fn update_max_cursor(&mut self) {
        let point = self.cursor.point;
        if point.row.0 >= self.rows {
            return;
        }
        if point.row > self.max_cursor_point.row
            || (point.row == self.max_cursor_point.row && point.col > self.max_cursor_point.col)
        {
            self.max_cursor_point = point;
        }
    }

    /// Returns an immutable reference to the active [`Cursor`]. See [`Self::update_cursor`] as an
    /// alternative that provides mutable access to the `Cursor`.
    pub fn cursor(&self) -> &Cursor {
        &self.cursor
    }

    pub(super) fn row_wraps(&self, row_idx: usize) -> bool {
        let Some(cell) = self.get(row_idx).and_then(|row| row.last()) else {
            return false;
        };
        cell.flags().contains(Flags::WRAPLINE)
    }

    /// Moves everything in the visible screen down by "positions" amount of rows.
    /// Importantly, this is the concept of grid scrolling, not blocklist scrolling. Examples
    /// of operations that use this method are any alt-screen command, git log, or git diff.
    ///
    /// The "region" parameter is a subset of the VisibleScreen, and it tells us about any fixed lines
    /// at the top or bottom of the screen that should be excluded from the scroll action. For example,
    /// the VisibleScreen might be (0..40) and the region might be (1..40), indicating that there's
    /// one fixed line at the top of the grid.
    ///
    /// Say the grid has 10 rows (0..9) and visible_rows = grid.len() (i.e., no scrollback).
    /// If there's one fixed line at the bottom of the grid and we scroll down by one line...
    /// (1) Swap the fixed line at the bottom with the line above it. (ix 9 <--> ix 8)
    /// (2) Inform the storage layer of the change to the bottommost row. ("shift up by one")
    /// (3) Clear the topmost row (ix 0).
    ///
    /// If there is a scrollback, this process requires piecewise swapping of the visible rows.
    /// For a more detailed specification, look at test_grid_scroll_down() and test_grid_git_diff_or_log()
    #[inline]
    pub fn scroll_down(&mut self, region: &Range<VisibleRow>, scroll_distance: usize) {
        let visible_rows = self.visible_rows();

        // When rotating the entire region, just reset everything.
        if scroll_distance >= region.end - region.start {
            for line in IndexRange(region.start..region.end) {
                self.raw[line].reset(&self.cursor.template);
            }
            return;
        }

        // Which implementation we can use depends on the existence of a scrollback history.
        //
        // Since a scrollback history prevents us from rotating the entire buffer downwards, we
        // instead have to rely on a slower, swap-based implementation.
        if self.max_scroll_limit == 0 {
            // Swap the lines fixed at the bottom to their target positions after rotation.
            //
            // Since we've made sure that the rotation will never rotate away the entire region, we
            // know that the position of the fixed lines before the rotation must already be
            // visible.
            //
            // We need to start from the top, to make sure the fixed lines aren't swapped with each
            // other.
            let fixed_lines = visible_rows - region.end.0;
            for i in (0..fixed_lines).rev() {
                let index = visible_rows - i - 1;
                self.raw.swap(index, index - scroll_distance);
            }

            // Reduce the bottommost row of the grid. In other words, retract the concept of the bottom_row
            // by some integer value.
            self.raw.retract(scroll_distance as isize);

            // The new lines appear at the top of the screen. Reset them.
            for i in 0..scroll_distance {
                self.raw[i].reset(&self.cursor.template);
            }

            // Swap the fixed lines at the top back into position by swapping them upwards.
            for i in 0..region.start.0 {
                self.raw.swap(i, i + scroll_distance);
            }
        } else {
            // Subregion rotation is a two-step process of bubbling the bottom rows to the top
            // and then clearing them. Like so:
            //   0: a         0: e         0:
            //   1: b  swap   1: a  clear  1: a
            //   2: c  ---->  2: b  ---->  2: b
            //   3: d         3: c         3: c
            //   4: e         4: d         4: d

            // Starting with the bottommost visible row, swap everything up by one, resulting in
            // the bottommost row ending up as the topmost row and everything else shifted down by one.
            for line in IndexRange((region.start + scroll_distance)..region.end).rev() {
                self.raw.swap_lines(line, line - scroll_distance);
            }

            // Clear rows at the top, including the rows we just swapped there.
            for line in IndexRange(region.start..(region.start + scroll_distance)) {
                self.raw[line].reset(&self.cursor.template);
            }
        }
    }

    /// Moves everything in the visible screen up by `scroll_distance` number of rows.
    /// Importantly, this is the concept of grid scrolling, not blocklist scrolling. Examples
    /// of operations that use this method are any alt-screen command, git log, or git diff.
    ///
    /// This is the performance-sensitive part of scrolling!
    ///
    /// For a more detailed specification, look at test_grid_scroll_up() and test_grid_git_diff_or_log()
    pub fn scroll_up(&mut self, region: &Range<VisibleRow>, scroll_distance: usize) {
        let visible_rows = self.visible_rows();

        // When rotating the entire region with fixed lines at the top, just reset everything.
        if scroll_distance >= region.end - region.start && region.start != VisibleRow(0) {
            for line in IndexRange(region.start..region.end) {
                self.raw[line].reset(&self.cursor.template);
            }
            return;
        }

        // Before extending the scrollback by the scroll amount, calculate
        // how many lines are going to be truncated, if any.
        let num_lines_to_truncate =
            (self.history_size() + scroll_distance).saturating_sub(self.max_scroll_limit);

        // Create scrollback for the new lines.
        self.increase_scroll_limit(scroll_distance);

        // Swap the lines fixed at the top to their target positions after rotation.
        //
        // Since we've made sure that the rotation will never rotate away the entire region, we
        // know that the position of the fixed lines before the rotation must already be
        // visible.
        //
        // We need to start from the bottom, to make sure the fixed lines aren't swapped with each
        // other.
        for i in (0..region.start.0).rev() {
            self.raw.swap(i, i + scroll_distance);
        }

        // Mark that the bottommost row is now extended.
        self.raw.extend(scroll_distance as isize);

        // The new lines appear at the bottom of the screen. Reset them.
        for i in self.raw.len() - scroll_distance..self.raw.len() {
            self.raw[i].reset(&self.cursor.template);
        }

        // Swap the fixed lines at the bottom back into position.
        let fixed_lines = VisibleRow(visible_rows) - region.end;
        for i in 0..fixed_lines {
            let index = visible_rows - i - 1;
            self.raw.swap(index, index - scroll_distance);
        }

        // After modifying the scrollback buffer, check if we're truncating
        // rows and keep track of how many rows we've truncated.
        if self.max_scroll_limit == self.history_size() {
            self.num_lines_truncated += num_lines_to_truncate as u64;
        }
    }

    /// Clear the grid, leaving the cursor's line at the top and preserving the cursor location
    /// within the line.
    pub(super) fn clear_and_reset_saving_cursor_line(&mut self) {
        self.clear_history();

        self.raw.swap_lines(self.cursor.point.row, VisibleRow(0));
        self.cursor.point.row = VisibleRow(0);

        // Reset all visible lines except the top most row.
        for row in 1..self.raw.len() {
            self.raw[row].reset(&Cell::default());
        }
    }

    /// Completely reset the grid state.
    pub fn reset(&mut self) {
        self.clear_history();

        self.saved_cursor = Cursor::default();
        self.cursor = Cursor::default();
        self.max_cursor_point = Default::default();

        // Reset all visible lines.
        for row in 0..self.raw.len() {
            self.raw[row].reset(&self.cursor.template);
        }
    }

    /// Populate grid from 2D char array - particularly useful for testing purposes.
    #[cfg(test)]
    pub fn populate_from_array(&mut self, arr: &[&[char]]) {
        for (i, row) in arr.iter().enumerate() {
            for (j, &cell) in row.iter().enumerate() {
                if i < self.rows && j < self.columns {
                    self[i][j].c = cell;
                }
            }
        }
    }

    pub fn estimated_memory_usage_bytes(&self) -> usize {
        std::mem::size_of_val(self) + self.estimated_heap_usage_bytes()
    }

    pub fn estimated_heap_usage_bytes(&self) -> usize {
        // For right now, we're only factoring in the heap size of
        // the grid storage.  Some other fields (e.g.: `secrets` and
        // `secrets_in_plaintext`) contain heap-allocated data, but we
        // aren't as worried about them for now.
        self.raw.estimated_heap_usage_bytes()
    }
}

#[allow(clippy::len_without_is_empty)]
impl GridStorage {
    #[inline]
    pub fn clear_history(&mut self) {
        // Explicitly purge all lines from history.
        self.raw.shrink_lines(self.history_size());
    }

    /// The number of rows in the grid up until the cursor.
    fn rows_to_cursor(&self) -> usize {
        self.cursor.point.row.0 + self.history_size()
    }

    /// The number of visible rows in the grid up until the cursor.
    pub fn visible_rows_to_cursor(&self) -> usize {
        self.cursor.point.row.0
    }

    /// The number of columns in the grid up until the cursor.
    pub fn cols_to_cursor(&self) -> usize {
        self.cursor.point.col
    }

    /// Truncate all rows after the cursor's row from the Grid.
    #[inline]
    pub(super) fn truncate_to_cursor_rows(&mut self) {
        let cursor_absolute_row = self.rows_to_cursor();

        // We want to include the line _with_ the cursor, so add one here.
        let rows_to_include = cursor_absolute_row + 1;
        self.raw.truncate_to(rows_to_include);

        self.rows = self.rows.min(rows_to_include);

        // Reposition the cursor on the visible screen. If the cursor was not at the bottom of the
        // grid _and_ we had scrollback available, then truncating the lines below the cursor will
        // effectively move the contents down the grid, so we need to update the cursor to match.
        // Ultimately, the _absolute_ position of the cursor in the grid contents should not change
        // by truncating the content after the cursor.
        let cursor_position_shift = cursor_absolute_row.saturating_sub(self.rows_to_cursor());
        self.cursor.point.row += cursor_position_shift;

        // Reset the max cursor to be the value of the cursor, since the the max cursor isn't
        // guaranteed to be in the grid anymore.
        self.max_cursor_point = self.cursor.point;
    }

    /// Truncate columns in Grid and Storage to specified target # of columns.
    fn truncate_columns(&mut self, col_to_truncate_to: usize) {
        self.raw.truncate_columns(col_to_truncate_to);
        self.columns = col_to_truncate_to;
    }

    /// Truncate all columns to the right of the cursor (including the column of the cursor, if the cursor is
    /// not at the end of a line that needs to wrap for the next character).
    pub(super) fn truncate_to_cursor_cols(&mut self) {
        // If the cursor indicates that the input needs to wrap for the next character AND the cursor is at the
        // end of the line, then, by definition, the entire line that the cursor is on, INCLUDING the column that
        // the cursor is on, should be preserved. Hence, we don't need to truncate any columns.
        // Note the cursor columns are zero-indexed hence the +1 below!
        if !(self.cols_to_cursor() + 1 == self.columns() && self.cursor.input_needs_wrap) {
            // If the cursor is NOT at the end of the line, then the cursor should be positioned at the next
            // available cell to print output to. Hence, we truncate all columns from the cursor's column
            // onwards (INCLUSIVE).
            self.truncate_columns(self.cols_to_cursor());
        }
    }

    #[inline]
    pub fn cursor_cell(&mut self) -> &mut Cell {
        let mut point = self.cursor_point();

        if point.row >= self.total_rows() || point.col >= self.columns() {
            log::error!(
                "Error retrieving cursor cell, cursor point was outside the bounds of the grid: {point:?}"
            );
            point = Point {
                row: self.total_rows().saturating_sub(1),
                col: self.columns().saturating_sub(1),
            };
        }

        &mut self[&point]
    }

    pub(super) fn cursor_point(&self) -> Point {
        Point::new(
            self.history_size() + self.cursor.point.row.0,
            self.cursor.point.col,
        )
    }

    pub fn get(&self, index: usize) -> Option<&Row> {
        if index < self.total_rows() {
            Some(&self.raw[index])
        } else {
            None
        }
    }
}

impl Dimensions for GridStorage {
    #[inline]
    fn total_rows(&self) -> usize {
        self.raw.len()
    }

    #[inline]
    fn visible_rows(&self) -> usize {
        self.rows
    }

    #[inline]
    fn columns(&self) -> usize {
        self.columns
    }
}

#[cfg(test)]
#[path = "grid_tests.rs"]
mod tests;
