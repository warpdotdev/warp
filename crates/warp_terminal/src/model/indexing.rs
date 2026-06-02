//! Types relating to indexing into a terminal grid.

use std::cmp::Ordering;
use std::fmt;
use std::ops::{Add, AddAssign, Range, Sub, SubAssign};

use serde::{Deserialize, Serialize};
use warpui_core::units::Lines;

use super::grid::Dimensions;

/// Behavior for handling grid boundaries.
pub enum Boundary {
    /// Clamp to grid boundaries.
    ///
    /// When an operation exceeds the grid boundaries, the last point will be returned no matter
    /// how far the boundaries were exceeded.
    Clamp,

    /// Wrap around grid bondaries.
    ///
    /// When an operation exceeds the grid boundaries, the point will wrap around the entire grid
    /// history and continue at the other side.
    Wrap,
}

/// An integral index representing a row or column in a grid.
///
/// This exists to encapsulate logic needed to account for floating point error
/// when converting from a floating-point grid position to an integral one.
/// Accumulation of small errors over time can cause a simple truncation from an
/// f32 to a usize to produce an off-by-one error, so when constructing an
/// `Index` from `Lines`, we adjust the value upwards by a small amount before
/// truncating.
pub struct Index(usize);

impl Index {
    /// The amount by which we are willing to round up when converting from the
    /// floating-point `Lines` to an integral grid row/column index.  This helps
    /// account for small errors that accumulate as arithmetic operations are
    /// performed on floating-point values.
    const FLOATING_POINT_ERROR_ADJUSTMENT: f64 = 0.0001;
}

impl From<usize> for Index {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl From<Lines> for Index {
    fn from(value: Lines) -> Self {
        // Adjust the value upwards slightly before truncating, to round up
        // when the value is sufficiently close to the next integer boundary.
        let error_adjusted = value.as_f64() + Self::FLOATING_POINT_ERROR_ADJUSTMENT;
        Self(error_adjusted as usize)
    }
}

/// Index in the grid using row, column notation.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, Eq, PartialEq, Hash)]
pub struct Point {
    pub row: usize,
    pub col: usize,
}

impl Point {
    pub fn new(line: impl Into<Index>, col: impl Into<Index>) -> Point {
        Point {
            row: line.into().0,
            col: col.into().0,
        }
    }

    pub const fn zero() -> Self {
        Self { row: 0, col: 0 }
    }

    #[inline]
    #[must_use = "this returns the result of the operation, without modifying the original"]
    /// Increments the `Point` by `distance` cells, wrapping around if the column value exceeds
    /// `num_cols`.
    pub fn wrapping_add(mut self, num_cols: usize, distance: usize) -> Point {
        self.row += (distance + self.col) / num_cols;
        self.col = (self.col + distance) % num_cols;
        self
    }

    #[inline]
    #[must_use = "this returns the result of the operation, without modifying the original"]
    /// Decrements the `Point` by `distance` cells, wrapping around if the column value would drop
    /// below zero.
    ///
    /// Note: This will also saturate at (0, 0) as a minimum value.
    pub fn wrapping_sub(mut self, num_cols: usize, distance: usize) -> Point {
        let line_changes = (distance + num_cols - 1).saturating_sub(self.col) / num_cols;
        if self.row >= line_changes {
            self.row -= line_changes;
            self.col = (num_cols + self.col - distance % num_cols) % num_cols;
            self
        } else {
            Point::new(0, 0)
        }
    }

    /// Returns 1D representation of point, as an index into a 1D array of cells (assumes left-to-right, top-to-bottom order).
    fn as_one_dimensional_index(&self, num_cols: usize) -> usize {
        self.row * num_cols + self.col
    }

    /// Compares Point against another Point, returning the one that is maximal, given the number of
    /// columns in the grid being considered.
    pub fn max_point<'a>(&'a self, other: &'a Point, num_cols: usize) -> &'a Point {
        if self.as_one_dimensional_index(num_cols) >= other.as_one_dimensional_index(num_cols) {
            self
        } else {
            other
        }
    }

    /// Computes left-to-right distance between two points. The result is an absolute value.
    pub fn distance(&self, num_cols: usize, other: &Point) -> usize {
        let this_index = self.as_one_dimensional_index(num_cols);
        let other_index = other.as_one_dimensional_index(num_cols);

        this_index.abs_diff(other_index)
    }

    pub fn to_visible_point(&self, history_size: usize) -> VisiblePoint {
        VisiblePoint {
            row: VisibleRow(self.row.saturating_sub(history_size)),
            col: self.col,
        }
    }
}

impl Point {
    #[inline]
    #[must_use = "this returns the result of the operation, without modifying the original"]
    pub fn sub_absolute<D>(mut self, dimensions: &D, boundary: Boundary, rhs: usize) -> Point
    where
        D: Dimensions,
    {
        let total_lines = dimensions.total_rows();
        let num_cols = dimensions.columns();

        self.row += (rhs + num_cols - 1).saturating_sub(self.col) / num_cols;
        self.col = (num_cols + self.col - rhs % num_cols) % num_cols;

        if self.row >= total_lines {
            match boundary {
                Boundary::Clamp => Point::new(total_lines - 1, 0),
                Boundary::Wrap => Point::new(self.row - total_lines, self.col),
            }
        } else {
            self
        }
    }

    #[inline]
    #[must_use = "this returns the result of the operation, without modifying the original"]
    pub fn add_absolute<D>(mut self, dimensions: &D, boundary: Boundary, rhs: usize) -> Point
    where
        D: Dimensions,
    {
        let num_cols = dimensions.columns();

        let line_delta = (rhs + self.col) / num_cols;

        if self.row >= line_delta {
            self.row -= line_delta;
            self.col = (self.col + rhs) % num_cols;
            self
        } else {
            match boundary {
                Boundary::Clamp => Point::new(0, num_cols - 1),
                Boundary::Wrap => {
                    let col = (self.col + rhs) % num_cols;
                    let line = dimensions.total_rows() + self.row - line_delta;
                    Point::new(line, col)
                }
            }
        }
    }
}

impl PartialOrd for Point {
    fn partial_cmp(&self, other: &Point) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Point {
    fn cmp(&self, other: &Point) -> Ordering {
        match (self.row.cmp(&other.row), self.col.cmp(&other.col)) {
            (Ordering::Equal, ord) | (ord, _) => ord,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct VisibleRow(pub usize);

impl Sub<usize> for VisibleRow {
    type Output = VisibleRow;

    fn sub(self, rhs: usize) -> Self::Output {
        Self(self.0 - rhs)
    }
}

impl SubAssign<Self> for VisibleRow {
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}

impl Sub<Self> for VisibleRow {
    type Output = usize;

    fn sub(self, rhs: Self) -> Self::Output {
        self.0 - rhs.0
    }
}

impl Add<Self> for VisibleRow {
    type Output = VisibleRow;

    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Add<usize> for VisibleRow {
    type Output = VisibleRow;

    fn add(self, rhs: usize) -> Self::Output {
        Self(self.0 + rhs)
    }
}

impl AddAssign<usize> for VisibleRow {
    fn add_assign(&mut self, rhs: usize) {
        self.0 += rhs;
    }
}

impl VisibleRow {
    pub fn saturating_sub(&self, rhs: usize) -> VisibleRow {
        VisibleRow(self.0.saturating_sub(rhs))
    }

    #[inline]
    pub fn steps_between(start: VisibleRow, end: VisibleRow, by: VisibleRow) -> Option<usize> {
        if by == VisibleRow(0) {
            return None;
        }
        if start < end {
            // Note: We assume $t <= usize here.
            let diff = end - start;
            let by = by.0;
            if !diff.is_multiple_of(by) {
                Some(diff / by + 1)
            } else {
                Some(diff / by)
            }
        } else {
            Some(0)
        }
    }

    #[inline]
    pub fn steps_between_by_one(start: VisibleRow, end: VisibleRow) -> Option<usize> {
        Self::steps_between(start, end, VisibleRow(1))
    }
}

impl fmt::Display for VisibleRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub struct VisiblePoint {
    pub row: VisibleRow,
    pub col: usize,
}

impl VisiblePoint {
    pub fn zero() -> Self {
        Self {
            row: VisibleRow(0),
            col: 0,
        }
    }

    #[inline]
    #[must_use = "this returns the result of the operation, without modifying the original"]
    /// Increments the `VisiblePoint` by `distance` cells, wrapping around if the column value
    /// exceeds `num_cols`.
    pub fn wrapping_add(mut self, num_cols: usize, distance: usize) -> VisiblePoint {
        self.row += (distance + self.col) / num_cols;
        self.col = (self.col + distance) % num_cols;
        self
    }

    #[inline]
    #[must_use = "this returns the result of the operation, without modifying the original"]
    /// Decrements the `VisiblePoint` by `distance` cells, wrapping around if the column value
    /// would drop below zero.
    ///
    /// Note: This will also saturate at (0, 0) as a minimum value.
    pub fn wrapping_sub(mut self, num_cols: usize, distance: usize) -> VisiblePoint {
        let line_changes = (distance + num_cols - 1).saturating_sub(self.col) / num_cols;
        if self.row >= VisibleRow(line_changes) {
            self.row = self.row - line_changes;
            self.col = (num_cols + self.col - distance % num_cols) % num_cols;
            self
        } else {
            VisiblePoint {
                row: VisibleRow(0),
                col: 0,
            }
        }
    }

    /// Wrap this point to a maximum width of `num_cols`, incrementing the row if it is beyond
    /// that value
    pub fn wrap(self, num_cols: usize) -> VisiblePoint {
        self.wrapping_add(num_cols, 0)
    }
}

impl PartialOrd for VisiblePoint {
    fn partial_cmp(&self, other: &VisiblePoint) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for VisiblePoint {
    fn cmp(&self, other: &VisiblePoint) -> Ordering {
        match (self.row.cmp(&other.row), self.col.cmp(&other.col)) {
            (Ordering::Equal, ord) | (ord, _) => ord,
        }
    }
}

/// This exists because we can't implement Iterator on Range
/// and the existing impl needs the unstable Step trait
/// This should be removed and replaced with a Step impl
/// in the ops macro when `step_by` is stabilized.
pub struct IndexRange<T>(pub Range<T>);

impl<T> From<Range<T>> for IndexRange<T> {
    fn from(from: Range<T>) -> Self {
        IndexRange(from)
    }
}

impl Iterator for IndexRange<usize> {
    type Item = usize;
    #[inline]
    fn next(&mut self) -> Option<usize> {
        if self.0.start < self.0.end {
            let old = self.0.start;
            self.0.start = old + 1;
            Some(old)
        } else {
            None
        }
    }
}

impl DoubleEndedIterator for IndexRange<usize> {
    #[inline]
    fn next_back(&mut self) -> Option<usize> {
        if self.0.start < self.0.end {
            let new = self.0.end - 1;
            self.0.end = new;
            Some(new)
        } else {
            None
        }
    }
}

impl Iterator for IndexRange<VisibleRow> {
    type Item = VisibleRow;
    #[inline]
    fn next(&mut self) -> Option<VisibleRow> {
        if self.0.start < self.0.end {
            let old = self.0.start;
            self.0.start = old + 1;
            Some(old)
        } else {
            None
        }
    }
    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        match Self::Item::steps_between_by_one(self.0.start, self.0.end) {
            Some(hint) => (hint, Some(hint)),
            None => (0, None),
        }
    }
}

impl DoubleEndedIterator for IndexRange<VisibleRow> {
    #[inline]
    fn next_back(&mut self) -> Option<VisibleRow> {
        if self.0.start < self.0.end {
            let new = self.0.end - 1;
            self.0.end = new;
            Some(new)
        } else {
            None
        }
    }
}

#[cfg(test)]
#[path = "indexing_tests.rs"]
mod tests;
