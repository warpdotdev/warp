use std::borrow::Cow;

use warp_terminal::model::grid::CellType;
use warp_terminal::model::grid::cell::{self, Cell};
use warp_terminal::model::grid::row::Row;

use super::grid_handler::GridHandler;
use super::{CursorDirection, CursorState, Dimensions as _};
use crate::terminal::model::index::Point;

/// The set of possible grapheme cursor wrapping behaviors.
#[derive(PartialEq)]
pub enum Wrap {
    /// Does not wrap at all (stops at the start and end of a row).
    None,
    /// Wraps at the start/end of a row, but stops upon reaching a newline.
    Soft,
    /// Wraps at the start and end of each row, ignoring newlines.
    All,
}

/// A cursor for iterating forward or backward over graphemes in a terminal
/// grid.
///
/// If constructed on a wide char spacer cell, the cursor will be moved one
/// cell to the left, snapping it to the cell which contains the content for
/// the grapheme under the cursor.  If constructed on a leading wide char
/// spacer cell, the cursor will be moved one cell to the right (for the same
/// reason).
pub struct GraphemeCursor<'g> {
    grid: &'g GridHandler,
    cur: Point,
    cursor_state: CursorState,
    wrap: Wrap,

    /// The index of the row that is stored in `cached_row`.
    cached_row_idx: usize,
    /// A cached shared reference to the row with index `cached_row_idx`.
    ///
    /// This is a `Cow` because grid storage internally holds `Row`s, and can
    /// return a reference to an existing one, whereas flat storage needs to
    /// construct one on-demand from its internal representation.
    cached_row: Option<Cow<'g, Row>>,
}

impl<'g> GraphemeCursor<'g> {
    /// Returns a new grapheme cursor that starts at the given point and
    /// adheres to the provided wrapping behavior.
    pub fn new(point: Point, grid: &'g GridHandler, wrap: Wrap) -> Self {
        if let Some(row) = grid.row(point.row)
            && let Some(cell) = row.get(point.col)
        {
            let flags = cell.flags;

            let mut cursor = Self {
                grid,
                cur: point,
                cursor_state: CursorState::Valid,
                wrap,

                cached_row_idx: point.row,
                cached_row: Some(row),
            };

            // The cursor should never start on a spacer cell.  If we're on
            // the second cell in a wide char, move to the first cell.  If
            // we're on the cell where a wide char _should_ have started,
            // move to its actual start cell.
            if flags.intersects(cell::Flags::WIDE_CHAR_SPACER) {
                cursor.move_backward();
            } else if flags.intersects(cell::Flags::LEADING_WIDE_CHAR_SPACER) {
                cursor.move_forward();
            }

            return cursor;
        }

        Self {
            grid,
            cur: point,
            cursor_state: CursorState::Invalid,
            wrap,

            cached_row_idx: point.row,
            cached_row: None,
        }
    }

    /// Returns a struct that provides access to information about the grapheme
    /// under the cursor, or [`None`] if the cursor is invalid or exhausted.
    pub fn current_item(&self) -> Option<GraphemeCursorItem<'_>> {
        match self.cursor_state {
            CursorState::Valid => {
                let cell = self
                    .cached_row
                    .as_ref()
                    .expect("row should be valid")
                    .get(self.cur.col)
                    .expect("col should be valid");

                Some(GraphemeCursorItem {
                    cell,
                    point: self.cur,
                })
            }
            _ => None,
        }
    }

    /// Returns the position the cursor had when it was last valid.
    ///
    /// This is useful for knowing the "final" cursor position after it has
    /// been exhausted (e.g.: hitting the start/end of the grid or a wrapping
    /// boundary condition).
    pub fn last_valid_position(&self) -> Point {
        self.cur
    }

    fn current_point_valid(&self) -> bool {
        self.cur.row < self.grid.total_rows() && self.cur.col < self.grid.columns()
    }

    fn has_next(&self) -> bool {
        if !self.current_point_valid() {
            return false;
        }

        let at_end_of_row = self.is_at_end_of_row();
        if self.wrap == Wrap::None && at_end_of_row
            || self.wrap == Wrap::Soft && self.is_at_end_of_line()
        {
            return false;
        }

        !at_end_of_row || self.cur.row < self.grid.total_rows() - 1
    }

    fn has_prev(&self) -> bool {
        if !self.current_point_valid() {
            return false;
        }

        let at_start_of_row = self.is_at_start_of_row();
        if self.wrap == Wrap::None && at_start_of_row
            || self.wrap == Wrap::Soft && self.is_at_start_of_line()
        {
            return false;
        }

        !at_start_of_row || self.cur.row > 0
    }

    fn update_cached_row(&mut self) {
        self.cached_row_idx = self.cur.row;
        self.cached_row = self.grid.row(self.cached_row_idx);
    }

    /// Moves the cursor forward by a single grapheme.
    pub fn move_forward(&mut self) {
        match self.cursor_state {
            CursorState::Valid if self.has_next() => {
                let start_row = self.cur.row;

                self.cur = self.cur.wrapping_add(self.grid.columns(), 1);
                // Skip over any spacer cells.
                if matches!(
                    self.grid.cell_type(self.cur),
                    Some(CellType::WideCharSpacer | CellType::LeadingWideCharSpacer)
                ) {
                    self.move_forward();
                }

                if self.cur.row != start_row {
                    self.update_cached_row();
                }
            }
            CursorState::Valid => {
                self.cursor_state = CursorState::Exhausted(CursorDirection::Right);
            }
            CursorState::Exhausted(CursorDirection::Left) => {
                self.cursor_state = CursorState::Valid;
            }
            _ => (),
        }
    }

    /// Moves the cursor backward by a single grapheme.
    pub fn move_backward(&mut self) {
        match self.cursor_state {
            CursorState::Valid if self.has_prev() => {
                let start_row = self.cur.row;

                self.cur = self.cur.wrapping_sub(self.grid.columns(), 1);
                // Skip over any spacer cells.
                if matches!(
                    self.grid.cell_type(self.cur),
                    Some(CellType::WideCharSpacer | CellType::LeadingWideCharSpacer)
                ) {
                    self.move_backward();
                }

                if self.cur.row != start_row {
                    self.update_cached_row();
                }
            }
            CursorState::Valid => {
                self.cursor_state = CursorState::Exhausted(CursorDirection::Left);
            }
            CursorState::Exhausted(CursorDirection::Right) => {
                self.cursor_state = CursorState::Valid;
            }
            _ => (),
        }
    }

    fn is_at_start_of_row(&self) -> bool {
        self.cur.col == 0
    }

    fn is_at_end_of_row(&self) -> bool {
        self.cur.col == self.grid.columns() - 1
    }

    /// Returns whether or not the cursor is at the start of a line.
    ///
    /// In other words, this returns true if moving the cursor backwards would
    /// transition across a newline/hard wrap.
    pub fn is_at_start_of_line(&self) -> bool {
        self.is_at_start_of_row() && (self.cur.row == 0 || !self.grid.row_wraps(self.cur.row - 1))
    }

    /// Returns whether or not the cursor is at the end of a line.
    ///
    /// In other words, this returns true if moving the cursor forwards would
    /// transition across a newline/hard wrap.
    pub fn is_at_end_of_line(&self) -> bool {
        self.is_at_end_of_row()
            && (self.cur.row == self.grid.total_rows() - 1 || !self.grid.row_wraps(self.cur.row))
    }
}

/// A helper struct for providing information about the grapheme at which the
/// cursor is currently located.
pub struct GraphemeCursorItem<'g> {
    cell: &'g Cell,
    point: Point,
}

impl GraphemeCursorItem<'_> {
    /// Returns a reference to the cell that holds the content for the grapheme
    /// under the cursor.
    ///
    /// TODO(CORE-2955): Fix the fact that many callers look at `cell().c` to
    ///                  get the cell content, which is incorrect for cells
    ///                  which have additional content in `CellExtra`.
    pub fn cell(&self) -> &Cell {
        self.cell
    }

    /// Returns the character in the cell under the grapheme cursor.
    ///
    /// TODO(CORE-2955): Fix the fact that many callers look at `cell().c` to
    ///                  get the cell content, which is incorrect for cells
    ///                  which have additional content in `CellExtra`.
    pub fn content_char(&self) -> char {
        if self.cell.c == cell::DEFAULT_CHAR {
            ' '
        } else {
            self.cell.c
        }
    }

    /// Returns the item's position in the grid.
    pub fn point(&self) -> Point {
        self.point
    }
}

#[cfg(test)]
#[path = "grapheme_cursor_tests.rs"]
mod tests;
