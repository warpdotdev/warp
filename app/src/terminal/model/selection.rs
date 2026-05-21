//! State management for a selection in the grid.
//!
//! A selection should start when the mouse is clicked, and it should be
//! finalized when the button is released. The selection should be cleared
//! when text is added/removed/scrolled on the screen. The selection should
//! also be cleared if the user clicks off of the selection.
use std::fmt::Debug;
use vec1::Vec1;
use warp_terminal::model::grid::cell;

use crate::terminal::model::ansi::CursorShape;
use crate::terminal::model::cell::Flags;
use crate::terminal::model::grid::grid_handler::GridHandler;
use crate::terminal::model::grid::Dimensions;
use crate::terminal::model::index::{Point, Side};
use crate::terminal::model::GridStorage;
use crate::terminal::Vector2F;
use std::cmp::{max, min};
use std::mem;
use std::ops::RangeInclusive;
pub use std::ops::{Range, RangeBounds};
use warp_core::semantic_selection::SemanticSelection;
use warpui::text::SelectionType;
use warpui::units::Lines;

use super::index::{Direction, VisibleRow};

/// A Point and side within that point.
#[derive(Debug, Copy, Clone, PartialEq)]
struct Anchor {
    point: Point,
    side: Side,
}

impl Anchor {
    fn new(point: Point, side: Side) -> Anchor {
        Anchor { point, side }
    }
}

/// An expanded selection range after processing the active selection mode.
#[derive(Debug, PartialEq)]
pub enum ExpandedSelectionRange<T: Debug> {
    /// A normal text selection.
    Regular {
        /// Start of the renderable selection. This is always before end.
        start: T,
        /// End of the renderable selection. This is always after start.
        end: T,
        /// If true, start and end were reversed to make start before end, but the user actually started
        /// the selection from end and dragged toward start.
        reversed: bool,
    },
    /// A text selection in rect mode. Note that we don't keep track of whether the selection is
    /// reversed or not since extending selection by keyboard is not supported in this mode.
    Rect {
        /// Each row of the renderable rect selection, marked by its start and end point.
        rows: Vec1<(T, T)>,
    },
}

impl<T: Debug> ExpandedSelectionRange<T> {
    /// Create a new regular renderable selection.
    pub fn regular(start: T, end: T, reversed: bool) -> Self {
        ExpandedSelectionRange::Regular {
            start,
            end,
            reversed,
        }
    }

    /// Create a new rect renderable selection.
    pub fn rect(rows: Vec1<(T, T)>) -> Self {
        ExpandedSelectionRange::Rect { rows }
    }

    /// The head of the selection (where the user first clicked before dragging).
    pub fn head(&self) -> &T {
        match self {
            Self::Regular {
                start,
                end,
                reversed,
            } => {
                if *reversed {
                    end
                } else {
                    start
                }
            }
            Self::Rect { rows } => &rows.first().0,
        }
    }

    /// The tail of the selection (where the user most recently dragged to).
    pub fn tail(&self) -> &T {
        match self {
            Self::Regular {
                start,
                end,
                reversed,
            } => {
                if *reversed {
                    start
                } else {
                    end
                }
            }
            Self::Rect { rows } => &rows.last().1,
        }
    }

    pub fn start(&self) -> &T {
        match self {
            Self::Regular { start, .. } => start,
            Self::Rect { rows } => &rows.first().0,
        }
    }

    /// Returns the end of the renderable selection where start is always before end.
    pub fn end(&self) -> &T {
        match self {
            Self::Regular { end, .. } => end,
            Self::Rect { rows } => &rows.last().1,
        }
    }

    /// If true, start and end were reversed to make start before end, but the user actually started
    /// the selection from end and dragged toward start.
    pub fn is_reversed(&self) -> bool {
        match self {
            Self::Regular { reversed, .. } => *reversed,
            Self::Rect { .. } => false,
        }
    }
}

/// Represents a range of selected cells.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SelectionRange {
    /// Start point, top left of the selection.
    pub start: Point,
    /// End point, bottom right of the selection.
    pub end: Point,
    /// If true, the user started the selection from the end and dragged backward to the start.
    pub is_reversed: bool,
}

impl SelectionRange {
    pub fn new(start: Point, end: Point, is_reversed: bool) -> Self {
        Self {
            start,
            end,
            is_reversed,
        }
    }

    /// Check if a point lies within the selection.
    pub fn contains(&self, point: Point) -> bool {
        self.start.row <= point.row
            && self.end.row >= point.row
            && (self.start.col <= point.col || (self.start.row != point.row))
            && (self.end.col >= point.col || (self.end.row != point.row))
    }
}

impl SelectionRange {
    /// Check if the cell at a point is part of the selection.
    pub fn contains_cell(
        &self,
        grid: &GridStorage,
        point: Point,
        cursor_point: Point,
        cursor_shape: CursorShape,
    ) -> bool {
        // Do not invert block cursor at selection boundaries.
        if cursor_shape == CursorShape::Block
            && cursor_point == point
            && (self.start == point || self.end == point)
        {
            return false;
        }

        // Point itself is selected.
        if self.contains(point) {
            return true;
        }

        let num_cols = grid.columns();

        let cell = &grid[point];

        // Check if wide char's spacers are selected.
        if cell.flags().contains(Flags::WIDE_CHAR) {
            let prev = point.wrapping_sub(num_cols, 1);
            let next = point.wrapping_add(num_cols, 1);

            // Check trailing spacer.
            self.contains(next)
                // Check line-wrapping, leading spacer.
                || (grid[prev].flags().contains(Flags::LEADING_WIDE_CHAR_SPACER)
                && self.contains(prev))
        } else {
            false
        }
    }
}

/// Different directions to move the selection tail.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SelectionDirection {
    Left,
    Right,
    Up,
    Down,
}

/// Used to share code between different kinds of points
/// representing selection ranges.
/// The units for row are lines in blocklist space.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SelectionPoint {
    pub row: Lines,
    pub col: usize,
}

impl PartialOrd for SelectionPoint {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SelectionPoint {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.row, self.col).cmp(&(other.row, other.col))
    }
}

#[derive(Debug, Clone)]
pub enum SelectAction<T> {
    Begin {
        point: T,
        side: Side,
        selection_type: SelectionType,
        position: Vector2F,
    },
    Update {
        point: T,
        side: Side,
        delta: Lines,
        position: Vector2F,
    },
    End,
}

/// A scroll position delta, either up or down.
#[derive(Copy, Clone)]
pub enum ScrollDelta {
    Up { lines: usize },
    Down { lines: usize },
}

impl ScrollDelta {
    /// Technically, `Self::Down { lines: 0 }` and `Self::Up { lines: 0 }` are equivalent.
    pub fn zero() -> Self {
        Self::Down { lines: 0 }
    }
}

impl Default for ScrollDelta {
    fn default() -> Self {
        ScrollDelta::zero()
    }
}

/// Describes a region of a 2-dimensional area.
///
/// Used to track a text selection. There are four supported modes, each with its own constructor:
/// [`simple`], [`rect`], [`semantic`], and [`lines`]. The [`simple`] mode precisely tracks which
/// cells are selected without any expansion. [`rect`] will select rectangular regions.
/// [`semantic`] mode expands the initial selection to the nearest semantic escape char in either
/// direction. [`lines`] will always select entire lines.
///
/// Calls to [`update`] operate different based on the selection kind. The [`simple`] and [`block`]
/// mode do nothing special, simply track points and sides. [`semantic`] will continue to expand
/// out to semantic boundaries as the selection point changes. Similarly, [`lines`] will always
/// expand the new point to encompass entire lines.
///
/// [`simple`]: enum.Selection.html#method.simple
/// [`rect`]: enum.Selection.html#method.block
/// [`semantic`]: enum.Selection.html#method.semantic
/// [`lines`]: enum.Selection.html#method.rows
/// [`update`]: enum.Selection.html#method.update
#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    pub ty: SelectionType,
    region: Range<Anchor>,
    smart_select_side: Option<Direction>,
    smart_select_override: Option<RangeInclusive<Point>>,
}

impl Selection {
    pub fn new(ty: SelectionType, location: Point, side: Side) -> Selection {
        Self {
            region: Range {
                start: Anchor::new(location, side),
                end: Anchor::new(location, side),
            },
            ty,
            smart_select_side: None,
            smart_select_override: None,
        }
    }

    pub fn set_smart_select_side(&mut self, smart_select_side: Direction) {
        self.smart_select_side = Some(smart_select_side);
    }

    pub fn set_smart_select_override(&mut self, smart_select_override: RangeInclusive<Point>) {
        self.smart_select_override = Some(smart_select_override);
    }

    /// Update the end of the selection.
    pub fn update(&mut self, point: Point, side: Side) {
        self.region.end = Anchor::new(point, side);
    }

    pub fn is_tail_before_head(&self) -> bool {
        Self::points_need_swap(self.region.start.point, self.region.end.point)
    }

    pub fn rotate(
        mut self,
        range: &Range<VisibleRow>,
        delta: ScrollDelta,
        end_column: usize,
    ) -> Option<Selection> {
        let range_bottom = range.end.0;
        let range_top = range.start.0;

        let (mut start, mut end) = (&mut self.region.start, &mut self.region.end);

        if start.point > end.point {
            mem::swap(&mut start, &mut end);
        }

        // Rotate start of selection.
        if (start.point.row >= range_top || range_top == 0) && start.point.row <= range_bottom {
            match delta {
                ScrollDelta::Up { lines } => {
                    // When scrolling up and the top selected rows are out of the visible window.
                    if start.point.row < lines + range_top {
                        start.point.col = 0;
                        start.side = Side::Left;
                        start.point.row = range_top;
                    } else {
                        start.point.row -= lines;
                    }
                }
                ScrollDelta::Down { lines } => {
                    // If start is below the bottom of the visible window, delete selection.
                    if start.point.row + lines >= range_bottom {
                        return None;
                    }

                    start.point.row += lines;
                }
            }
        }

        // Rotate end of selection.
        if (end.point.row >= range_top || range_top == 0) && end.point.row <= range_bottom {
            match delta {
                ScrollDelta::Up { lines } => {
                    // If end is out of the top of the visible window, delete selection.
                    if end.point.row < range_top + lines {
                        return None;
                    }

                    end.point.row = max(end.point.row - lines, 0);
                }
                ScrollDelta::Down { lines } => {
                    // When scrolling down and the bottom selected rows are out of the visible window.
                    if end.point.row + lines >= range_bottom {
                        end.point.col = end_column - 1;
                        end.side = Side::Right;
                        end.point.row = range_bottom - 1;
                    } else {
                        end.point.row += lines;
                    }
                }
            }
        }

        Some(self)
    }

    pub fn is_empty(&self) -> bool {
        match self.ty {
            SelectionType::Simple | SelectionType::Rect => {
                let (mut start, mut end) = (self.region.start, self.region.end);
                if Self::points_need_swap(start.point, end.point) {
                    mem::swap(&mut start, &mut end);
                }

                // Simple selection is empty when the points are identical
                // or two adjacent cells have the sides right -> left.
                start == end
                    || (start.side == Side::Right
                        && end.side == Side::Left
                        && (start.point.row == end.point.row)
                        && start.point.col + 1 == end.point.col)
            }
            SelectionType::Semantic | SelectionType::Lines => false,
        }
    }

    /// Convert selection to grid coordinates.
    pub fn to_range(
        &self,
        grid: &GridHandler,
        selection: &SemanticSelection,
    ) -> Option<SelectionRange> {
        if self.is_empty() {
            return None;
        }

        let num_cols = grid.columns();

        // Order start above the end.
        let mut start = self.region.start;
        let mut end = self.region.end;
        let mut is_reversed = false;

        if Self::points_need_swap(start.point, end.point) {
            mem::swap(&mut start, &mut end);
            is_reversed = true;
        } else if start.point == end.point && matches!(end.side, Side::Left) {
            is_reversed = true;
        }

        // Clamp to inside the grid buffer.
        let (start, end) = Self::grid_clamp(start, end, grid.total_rows()).ok()?;

        Some(match self.ty {
            SelectionType::Simple | SelectionType::Rect => {
                self.range_simple(start, end, num_cols, is_reversed)
            }
            SelectionType::Semantic => {
                self.range_semantic(grid, start.point, end.point, selection, is_reversed)
            }
            SelectionType::Lines => Self::range_lines(grid, start.point, end.point, is_reversed),
        })
    }

    /// Bring start and end points in the correct order.
    fn points_need_swap(start: Point, end: Point) -> bool {
        start.row > end.row || start.row == end.row && start.col > end.col
    }

    /// Clamp selection inside grid to prevent OOB.
    fn grid_clamp(mut start: Anchor, end: Anchor, lines: usize) -> Result<(Anchor, Anchor), ()> {
        // Clamp selection inside of grid to prevent OOB.
        if start.point.row >= lines {
            // Remove selection if it is fully out of the grid.
            if end.point.row >= lines {
                return Err(());
            }

            // Clamp to grid if it is still partially visible.
            start.side = Side::Left;
            start.point.col = 0;
            start.point.row = lines - 1;
        }

        Ok((start, end))
    }

    fn range_semantic(
        &self,
        grid_handler: &GridHandler,
        mut start: Point,
        mut end: Point,
        selection: &SemanticSelection,
        is_reversed: bool,
    ) -> SelectionRange {
        if start == end {
            if let Some(matching) = grid_handler.bracket_search(start) {
                if (matching.row == start.row && matching.col < start.col)
                    || (matching.row > start.row)
                {
                    start = matching;
                } else {
                    end = matching;
                }

                return SelectionRange {
                    start,
                    end,
                    is_reversed,
                };
            }
        }

        // first, get the bounds for normal, non-smart-selection
        let mut range_start =
            grid_handler.semantic_search_left(start, |c| is_word_boundary_char(selection, c));
        let mut range_end =
            grid_handler.semantic_search_right(end, |c| is_word_boundary_char(selection, c));

        if selection.smart_select_enabled() && self.smart_select_override.is_some() {
            let smart_select_override = self
                .smart_select_override
                .as_ref()
                .expect("already checked this is Some");
            if smart_select_override.contains(&start) || smart_select_override.contains(&end) {
                range_start = min(range_start, *smart_select_override.start());
                range_end = max(range_end, *smart_select_override.end());
            }
        } else {
            // if there is a smart-select match, only override the bounds if it has selected a LARGER
            // range than normal selection.
            match self.smart_select_side {
                Some(Direction::Left) => {
                    if let Some((smart_start, smart_end)) =
                        grid_handler.smart_search(start, selection)
                    {
                        range_start = min(range_start, smart_start);
                        range_end = max(range_end, smart_end);
                    }
                }
                Some(Direction::Right) => {
                    if let Some((smart_start, smart_end)) =
                        grid_handler.smart_search(end, selection)
                    {
                        range_start = min(range_start, smart_start);
                        range_end = max(range_end, smart_end);
                    }
                }
                None => {}
            }
        }

        SelectionRange {
            start: range_start,
            end: range_end,
            is_reversed,
        }
    }

    fn range_lines(
        grid_handler: &GridHandler,
        start: Point,
        end: Point,
        is_reversed: bool,
    ) -> SelectionRange {
        let start = grid_handler.maybe_translate_point_from_displayed_to_original(start);
        let end = grid_handler.maybe_translate_point_from_displayed_to_original(end);

        let start = grid_handler.line_search_left(start);
        let end = grid_handler.line_search_right(end);

        let start = grid_handler.maybe_translate_point_from_original_to_displayed(start);
        let end = grid_handler.maybe_translate_point_from_original_to_displayed(end);

        SelectionRange {
            start,
            end,
            is_reversed,
        }
    }

    fn range_simple(
        &self,
        mut start: Anchor,
        mut end: Anchor,
        num_cols: usize,
        is_reversed: bool,
    ) -> SelectionRange {
        // Remove last cell if selection ends to the left of a cell.
        if end.side == Side::Left && start.point != end.point {
            // Special case when selection ends to left of first cell.
            if end.point.col == 0 {
                end.point = Point::new(end.point.row.saturating_sub(1), num_cols - 1);
            } else {
                end.point.col -= 1;
            }
        }

        // Remove first cell if selection starts at the right of a cell.
        if start.side == Side::Right && start.point != end.point {
            start.point.col += 1;

            // Wrap to next line when selection starts to the right of last column.
            if start.point.col == num_cols {
                start.point = Point::new(start.point.row + 1, 0);
            }
        }

        SelectionRange {
            start: start.point,
            end: end.point,
            is_reversed,
        }
    }
}

/// Returns whether or not the character should be considered a word boundary
/// character, based on the given semantic selection configuration.
fn is_word_boundary_char(selection: &SemanticSelection, c: char) -> bool {
    if c == cell::DEFAULT_CHAR {
        return true;
    }
    selection.is_word_boundary_char(c)
}

/// Tests for selection.
///
/// There are comments on all of the tests describing the selection. Pictograms
/// are used to avoid ambiguity. Grid cells are represented by a [  ]. Only
/// cells that are completely covered are counted in a selection. Ends are
/// represented by `B` and `E` for begin and end, respectively.  A selected cell
/// looks like [XX], [BX] (at the start), [XB] (at the end), [XE] (at the end),
/// and [EX] (at the start), or [BE] for a single cell. Partially selected cells
/// look like [ B] and [E ].
#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
