use std::ops::{Index, IndexMut, Range, RangeFrom, RangeFull, RangeTo};

use warp_terminal::model::grid::cell::Cell;
use warp_terminal::model::grid::row::Row;

use super::grid_handler::GridHandler;
use super::GridStorage;
use crate::terminal::model::grid::Dimensions as _;
use crate::terminal::model::index::{Point, VisiblePoint, VisibleRow};

pub(in crate::terminal::model) trait ConvertToAbsolute {
    type Output;

    /// Transforms a value from the VisibleScreen coordinate space to the grid
    /// coordinate space.
    fn convert_to_absolute(&self, grid: &GridHandler) -> Self::Output;
}

impl ConvertToAbsolute for VisibleRow {
    type Output = usize;

    fn convert_to_absolute(&self, grid: &GridHandler) -> Self::Output {
        grid.history_size() + self.0
    }
}

impl ConvertToAbsolute for VisiblePoint {
    type Output = Point;

    fn convert_to_absolute(&self, grid: &GridHandler) -> Self::Output {
        Point::new(self.row.convert_to_absolute(grid), self.col)
    }
}

/// Index with buffer offset.
impl Index<usize> for GridStorage {
    type Output = Row;

    #[inline]
    fn index(&self, index: usize) -> &Row {
        &self.raw[index]
    }
}

impl IndexMut<usize> for GridStorage {
    #[inline]
    fn index_mut(&mut self, index: usize) -> &mut Row {
        &mut self.raw[index]
    }
}

impl Index<&Point> for GridStorage {
    type Output = Cell;

    #[inline]
    fn index(&self, point: &Point) -> &Cell {
        &self[point.row][point.col]
    }
}

impl IndexMut<&Point> for GridStorage {
    #[inline]
    fn index_mut(&mut self, point: &Point) -> &mut Cell {
        &mut self[point.row][point.col]
    }
}

impl Index<Point> for GridStorage {
    type Output = Cell;

    #[inline]
    fn index(&self, point: Point) -> &Cell {
        &self[point.row][point.col]
    }
}

impl IndexMut<Point> for GridStorage {
    #[inline]
    fn index_mut(&mut self, point: Point) -> &mut Cell {
        &mut self[point.row][point.col]
    }
}

impl Index<VisibleRow> for GridStorage {
    type Output = Row;

    #[inline]
    fn index(&self, index: VisibleRow) -> &Row {
        &self.raw[index]
    }
}

impl IndexMut<VisibleRow> for GridStorage {
    #[inline]
    fn index_mut(&mut self, index: VisibleRow) -> &mut Row {
        &mut self.raw[index]
    }
}

/// A subset of lines in the grid.
///
/// May be constructed using Grid::region(..).
pub struct Region<'a> {
    start: VisibleRow,
    end: VisibleRow,
    grid: &'a GridStorage,
}

/// A mutable subset of lines in the grid.
///
/// May be constructed using Grid::region_mut(..).
pub struct RegionMut<'a> {
    start: VisibleRow,
    end: VisibleRow,
    grid: &'a mut GridStorage,
}

impl RegionMut<'_> {
    /// Call the provided function for every item in this region.
    pub fn each<F: Fn(&mut Cell)>(self, func: F) {
        for row in self {
            for item in row {
                func(item)
            }
        }
    }
}

pub trait IndexRegion<I> {
    /// Get an immutable region of Self.
    fn region(&self, _: I) -> Region<'_>;

    /// Get a mutable region of Self.
    fn region_mut(&mut self, _: I) -> RegionMut<'_>;
}

impl IndexRegion<Range<VisibleRow>> for GridStorage {
    fn region(&self, index: Range<VisibleRow>) -> Region<'_> {
        assert!(index.start < VisibleRow(self.visible_rows()));
        assert!(index.end <= VisibleRow(self.visible_rows()));
        assert!(index.start <= index.end);
        Region {
            start: index.start,
            end: index.end,
            grid: self,
        }
    }

    fn region_mut(&mut self, index: Range<VisibleRow>) -> RegionMut<'_> {
        assert!(index.start < VisibleRow(self.visible_rows()));
        assert!(index.end <= VisibleRow(self.visible_rows()));
        assert!(index.start <= index.end);
        RegionMut {
            start: index.start,
            end: index.end,
            grid: self,
        }
    }
}

impl IndexRegion<RangeTo<VisibleRow>> for GridStorage {
    fn region(&self, index: RangeTo<VisibleRow>) -> Region<'_> {
        assert!(index.end <= VisibleRow(self.visible_rows()));
        Region {
            start: VisibleRow(0),
            end: index.end,
            grid: self,
        }
    }

    fn region_mut(&mut self, index: RangeTo<VisibleRow>) -> RegionMut<'_> {
        assert!(index.end <= VisibleRow(self.visible_rows()));
        RegionMut {
            start: VisibleRow(0),
            end: index.end,
            grid: self,
        }
    }
}

impl IndexRegion<RangeFrom<VisibleRow>> for GridStorage {
    fn region(&self, index: RangeFrom<VisibleRow>) -> Region<'_> {
        assert!(index.start < VisibleRow(self.visible_rows()));
        Region {
            start: index.start,
            end: VisibleRow(self.visible_rows()),
            grid: self,
        }
    }

    fn region_mut(&mut self, index: RangeFrom<VisibleRow>) -> RegionMut<'_> {
        assert!(index.start < VisibleRow(self.visible_rows()));
        RegionMut {
            start: index.start,
            end: VisibleRow(self.visible_rows()),
            grid: self,
        }
    }
}

impl IndexRegion<RangeFull> for GridStorage {
    fn region(&self, _: RangeFull) -> Region<'_> {
        Region {
            start: VisibleRow(0),
            end: VisibleRow(self.visible_rows()),
            grid: self,
        }
    }

    fn region_mut(&mut self, _: RangeFull) -> RegionMut<'_> {
        RegionMut {
            start: VisibleRow(0),
            end: VisibleRow(self.visible_rows()),
            grid: self,
        }
    }
}

pub struct RegionIter<'a> {
    end: VisibleRow,
    cur: VisibleRow,
    grid: &'a GridStorage,
}

pub struct RegionIterMut<'a> {
    end: VisibleRow,
    cur: VisibleRow,
    grid: &'a mut GridStorage,
}

impl<'a> IntoIterator for Region<'a> {
    type IntoIter = RegionIter<'a>;
    type Item = &'a Row;

    fn into_iter(self) -> Self::IntoIter {
        RegionIter {
            end: self.end,
            cur: self.start,
            grid: self.grid,
        }
    }
}

impl<'a> IntoIterator for RegionMut<'a> {
    type IntoIter = RegionIterMut<'a>;
    type Item = &'a mut Row;

    fn into_iter(self) -> Self::IntoIter {
        RegionIterMut {
            end: self.end,
            cur: self.start,
            grid: self.grid,
        }
    }
}

impl<'a> Iterator for RegionIter<'a> {
    type Item = &'a Row;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cur < self.end {
            let index = self.cur;
            self.cur += 1;
            Some(&self.grid[index])
        } else {
            None
        }
    }
}

impl<'a> Iterator for RegionIterMut<'a> {
    type Item = &'a mut Row;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cur < self.end {
            let index = self.cur;
            self.cur += 1;
            unsafe { Some(&mut *(&mut self.grid[index] as *mut _)) }
        } else {
            None
        }
    }
}
