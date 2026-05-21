// The code in this file is adapted from the alacritty_terminal crate under the
// Apache license; see: crates/warp_terminal/src/model/LICENSE-ALACRITTY.

//! Defines the Row type which makes up lines in the grid.
use std::cmp::{max, min};
use std::ops::{Index, IndexMut};
use std::ops::{Range, RangeFrom, RangeFull, RangeTo, RangeToInclusive};
use std::ptr;
use std::slice;

use serde::{Deserialize, Serialize};

use crate::model::grid::cell::{Cell, ResetDiscriminant};

/// A row in the grid.
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Row {
    inner: Vec<Cell>,

    /// Maximum number of occupied entries.
    ///
    /// This is the upper bound on the number of elements in the row, which have been modified
    /// since the last reset. All cells after this point are guaranteed to be equal.
    ///
    /// TODO(visibility): This should be changed to `pub(crate)` when possible.
    pub occ: usize,
}

impl PartialEq for Row {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl Row {
    /// Create a new terminal row.
    pub fn new(usizes: usize) -> Row {
        debug_assert!(usizes >= 1);

        let mut inner: Vec<Cell> = Vec::with_capacity(usizes);

        // This is a slightly optimized version of `std::vec::Vec::resize`.
        unsafe {
            let mut ptr = inner.as_mut_ptr();

            for _ in 1..usizes {
                ptr::write(ptr, Cell::default());
                ptr = ptr.offset(1);
            }
            ptr::write(ptr, Cell::default());

            inner.set_len(usizes);
        }

        Row { inner, occ: 0 }
    }

    /// Increase the number of usizes in the row.
    #[inline]
    pub fn grow(&mut self, cols: usize) {
        if self.inner.len() >= cols {
            return;
        }

        self.inner.resize_with(cols, Cell::default);
    }

    /// Reduce the number of usizes in the row.
    ///
    /// This will return all non-empty cells that were removed.
    pub fn shrink(&mut self, cols: usize) -> Option<Vec<Cell>> {
        if self.inner.len() <= cols {
            return None;
        }

        // Split off cells for a new row.
        let mut new_row = self.inner.split_off(cols);
        let index = new_row
            .iter()
            // NOTE: We do NOT want to treat the "end of prompt" cell as empty in this case - we
            // want to preserve the marker in the Cell's `extra` field and carry it over for the row shrinking
            // in the context of a resize.
            .rposition(|c| !c.is_empty() || c.is_end_of_prompt())
            .map(|i| i + 1)
            .unwrap_or(0);
        new_row.truncate(index);

        self.occ = min(self.occ, cols);

        if new_row.is_empty() {
            None
        } else {
            Some(new_row)
        }
    }

    /// Truncate row to desired number of columns.
    pub fn truncate(&mut self, cols: usize) {
        self.inner.truncate(cols);
    }

    /// Reset all cells in the row to the `template` cell.
    #[inline]
    pub fn reset(&mut self, template: &Cell) {
        debug_assert!(!self.inner.is_empty());

        // Mark all cells as dirty if template cell changed.
        let len = self.inner.len();
        if self.inner[len - 1].discriminant() != template.discriminant() {
            self.occ = len;
        }

        // Reset every dirty cell in the row.
        for item in &mut self.inner[0..self.occ] {
            item.reset(template);
        }

        self.occ = 0;
    }

    pub fn get(&self, index: usize) -> Option<&Cell> {
        if index < self.len() {
            Some(&self[index])
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut Cell> {
        if index < self.len() {
            Some(&mut self[index])
        } else {
            None
        }
    }

    pub fn estimated_heap_usage_bytes(&self) -> usize {
        self.inner.capacity() * std::mem::size_of::<Cell>()
    }

    pub fn estimated_memory_usage_bytes(&self) -> usize {
        // size of struct on the stack
        std::mem::size_of::<Self>()
            // size of heap-allocated data in self.inner
            + self.estimated_heap_usage_bytes()
    }
}

#[allow(clippy::len_without_is_empty)]
impl Row {
    #[inline]
    pub fn from_vec(vec: Vec<Cell>, occ: usize) -> Row {
        Row { inner: vec, occ }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[inline]
    pub fn last(&self) -> Option<&Cell> {
        self.inner.last()
    }

    #[inline]
    pub fn last_mut(&mut self) -> Option<&mut Cell> {
        self.occ = self.inner.len();
        self.inner.last_mut()
    }

    #[inline]
    pub fn append(&mut self, vec: &mut Vec<Cell>) {
        self.occ += vec.len();
        self.inner.append(vec);
    }

    #[inline]
    pub fn append_front(&mut self, mut vec: Vec<Cell>) {
        self.occ += vec.len();

        vec.append(&mut self.inner);
        self.inner = vec;
    }

    /// Check if all cells in the row are empty.
    #[inline]
    pub fn is_clear(&self) -> bool {
        self.inner.iter().all(Cell::is_empty)
    }

    /// Returns `true` if no cells in the row contain an end of prompt marker
    /// and `false` otherwise.
    #[inline]
    pub fn has_no_end_of_prompt_marker(&self) -> bool {
        self.inner.iter().all(|cell| !Cell::is_end_of_prompt(cell))
    }

    #[inline]
    pub fn front_split_off(&mut self, at: usize) -> Vec<Cell> {
        self.occ = self.occ.saturating_sub(at);

        let mut split = self.inner.split_off(at);
        std::mem::swap(&mut split, &mut self.inner);
        split
    }

    /// Returns the set of cells that have been dirtied since the row was last
    /// reset.
    ///
    /// This is guaranteed to return all cells that contain content that should
    /// be rendered, but may also return some additional cells after the last
    /// actually-relevant cell.
    ///
    /// This returns a slice instead of an [`Iterator`] as a small performance
    /// optimization.
    pub(crate) fn dirty_cells(&self) -> &[Cell] {
        &self.inner[0..self.occ]
    }
}

impl<'a> IntoIterator for &'a mut Row {
    type Item = &'a mut Cell;
    type IntoIter = slice::IterMut<'a, Cell>;

    #[inline]
    fn into_iter(self) -> slice::IterMut<'a, Cell> {
        self.occ = self.len();
        self.inner.iter_mut()
    }
}

impl Index<usize> for Row {
    type Output = Cell;

    #[inline]
    fn index(&self, index: usize) -> &Cell {
        &self.inner[index]
    }
}

impl IndexMut<usize> for Row {
    #[inline]
    fn index_mut(&mut self, index: usize) -> &mut Cell {
        self.occ = max(self.occ, index + 1);
        &mut self.inner[index]
    }
}

impl Index<Range<usize>> for Row {
    type Output = [Cell];

    #[inline]
    fn index(&self, index: Range<usize>) -> &[Cell] {
        &self.inner[(index.start)..(index.end)]
    }
}

impl IndexMut<Range<usize>> for Row {
    #[inline]
    fn index_mut(&mut self, index: Range<usize>) -> &mut [Cell] {
        self.occ = max(self.occ, index.end);
        &mut self.inner[(index.start)..(index.end)]
    }
}

impl Index<RangeTo<usize>> for Row {
    type Output = [Cell];

    #[inline]
    fn index(&self, index: RangeTo<usize>) -> &[Cell] {
        &self.inner[..(index.end)]
    }
}

impl IndexMut<RangeTo<usize>> for Row {
    #[inline]
    fn index_mut(&mut self, index: RangeTo<usize>) -> &mut [Cell] {
        self.occ = max(self.occ, index.end);
        &mut self.inner[..(index.end)]
    }
}

impl Index<RangeFrom<usize>> for Row {
    type Output = [Cell];

    #[inline]
    fn index(&self, index: RangeFrom<usize>) -> &[Cell] {
        &self.inner[(index.start)..]
    }
}

impl IndexMut<RangeFrom<usize>> for Row {
    #[inline]
    fn index_mut(&mut self, index: RangeFrom<usize>) -> &mut [Cell] {
        self.occ = self.len();
        &mut self.inner[(index.start)..]
    }
}

impl Index<RangeFull> for Row {
    type Output = [Cell];

    #[inline]
    fn index(&self, _: RangeFull) -> &[Cell] {
        &self.inner[..]
    }
}

impl IndexMut<RangeFull> for Row {
    #[inline]
    fn index_mut(&mut self, _: RangeFull) -> &mut [Cell] {
        self.occ = self.len();
        &mut self.inner[..]
    }
}

impl Index<RangeToInclusive<usize>> for Row {
    type Output = [Cell];

    #[inline]
    fn index(&self, index: RangeToInclusive<usize>) -> &[Cell] {
        &self.inner[..=(index.end)]
    }
}

impl IndexMut<RangeToInclusive<usize>> for Row {
    #[inline]
    fn index_mut(&mut self, index: RangeToInclusive<usize>) -> &mut [Cell] {
        self.occ = max(self.occ, index.end);
        &mut self.inner[..=(index.end)]
    }
}
