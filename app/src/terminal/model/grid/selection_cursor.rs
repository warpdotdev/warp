use warp_terminal::model::grid::CellType;

use super::grid_handler::GridHandler;
use super::{CursorDirection, CursorState, Dimensions as _};
use crate::terminal::model::index::Point;

/// A structure to help with movement of the cursor for keyboard-driven
/// text selection.
pub struct SelectionCursor<'g> {
    /// Reference to the underlying grid.
    grid: &'g GridHandler,

    /// Current position of the cursor within the grid.
    pos: Point,

    /// The state of the cursor.
    cursor_state: CursorState,
}

impl<'g> SelectionCursor<'g> {
    pub fn new(grid: &'g GridHandler, pos: Point) -> Self {
        let mut cursor = Self {
            grid,
            pos,
            cursor_state: CursorState::Invalid,
        };
        if cursor.current_point_valid() {
            cursor.cursor_state = CursorState::Valid;
        }
        cursor
    }

    /// Returns the cursor's current position, if it is valid.
    pub fn position(&self) -> Option<Point> {
        matches!(self.cursor_state, CursorState::Valid).then_some(self.pos)
    }

    /// Moves the cursor forward by a single grapheme.
    pub fn move_forward(&mut self) {
        match self.cursor_state {
            CursorState::Valid if self.has_next() => {
                self.increment_cursor();
                // Original behavior (verified against the real, pre-existing
                // `test_select_left_select_right` test): after the single
                // step above, if we landed on the BASE of a wide/Indic
                // cluster, its span tells us exactly how many more cells to
                // skip to reach that same cluster's last cell -- for the
                // old fixed CJK width=2 case this is always exactly one more
                // step, identical to what the original code did. If instead
                // we landed on a SPACER (we started already partway through
                // a cluster, e.g. right on its base cell), take exactly ONE
                // more step -- matching the original code's literal
                // single-conditional-step behavior exactly, rather than
                // walking to the cluster's actual last cell. (A "smarter"
                // walk-to-the-end here changes existing width=2 behavior in
                // a way the real test catches -- see the plan's Phase 4
                // execution log for the full trace.)
                match self.grid.cell_type(self.pos) {
                    Some(CellType::WideChar) => {
                        let span = self.current_cell_span().unwrap_or(1);
                        for _ in 1..span {
                            self.increment_cursor();
                        }
                    }
                    Some(CellType::WideCharSpacer) => {
                        self.increment_cursor();
                    }
                    _ => {}
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
                self.decrement_cursor();
                // Mirror of `move_forward`: moving backward should land on
                // the cluster's FIRST (base) cell, not its last. If we
                // landed on a spacer, keep decrementing while still inside
                // spacer cells of the same cluster -- this naturally stops
                // exactly on the base (`WideChar`, which does NOT match
                // `WideCharSpacer`), regardless of span.
                while matches!(self.grid.cell_type(self.pos), Some(CellType::WideCharSpacer)) {
                    self.decrement_cursor();
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

    /// Moves the cursor up a row.
    ///
    /// Unlike the horizontal movement functions, this is not grapheme-aware -
    /// the cursor may end up on top of a wide char spacer cell.
    pub fn move_up(&mut self) {
        match self.cursor_state {
            CursorState::Valid if self.pos.row > 0 => {
                self.pos.row -= 1;
            }
            CursorState::Valid => {
                self.cursor_state = CursorState::Exhausted(CursorDirection::Up);
            }
            CursorState::Exhausted(CursorDirection::Down) => {
                self.cursor_state = CursorState::Valid;
            }
            _ => (),
        }
    }

    /// Moves the cursor down a row.
    ///
    /// Unlike the horizontal movement functions, this is not grapheme-aware -
    /// the cursor may end up on top of a wide char spacer cell.
    pub fn move_down(&mut self) {
        match self.cursor_state {
            CursorState::Valid if self.pos.row < self.grid.total_rows() - 1 => {
                self.pos.row += 1;
            }
            CursorState::Valid => {
                self.cursor_state = CursorState::Exhausted(CursorDirection::Down);
            }
            CursorState::Exhausted(CursorDirection::Up) => {
                self.cursor_state = CursorState::Valid;
            }
            _ => (),
        }
    }

    fn increment_cursor(&mut self) {
        if self.pos.col == self.grid.columns() - 1 {
            self.pos.row += 1;
            self.pos.col = 0;
        } else {
            self.pos.col += 1
        }
    }

    fn decrement_cursor(&mut self) {
        if self.pos.col == 0 {
            self.pos.row -= 1;
            self.pos.col = self.grid.columns() - 1;
        } else {
            self.pos.col -= 1;
        }
    }

    /// Returns the span of the cell at the current position, if valid. Only
    /// meaningful when that cell is a cluster's base (`CellType::WideChar`)
    /// -- spacer cells don't carry span information themselves.
    fn current_cell_span(&self) -> Option<u8> {
        self.grid
            .row(self.pos.row)?
            .get(self.pos.col)
            .map(|cell| cell.span())
    }

    /// Returns whether the current cursor point is valid (i.e.: is within the
    /// bounds of the grid).
    fn current_point_valid(&self) -> bool {
        self.pos.row < self.grid.total_rows() && self.pos.col < self.grid.columns()
    }

    /// Returns whether the cursor would be valid if it were incremented.
    fn has_next(&self) -> bool {
        (self.pos.row != self.grid.total_rows() - 1 || self.pos.col != self.grid.columns() - 1)
            && self.current_point_valid()
    }

    /// Returns whether the cursor would be valid if it were decremented.
    fn has_prev(&self) -> bool {
        (self.pos.row != 0 || self.pos.col != 0) && self.current_point_valid()
    }
}

#[cfg(test)]
#[path = "selection_cursor_tests.rs"]
mod tests;
