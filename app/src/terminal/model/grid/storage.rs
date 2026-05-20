// The code in this file is adapted from the alacritty_terminal crate under the
// Apache license; see: crates/warp_terminal/src/model/LICENSE-ALACRITTY.

use serde::{Deserialize, Serialize};
use std::cmp::{max, PartialEq};
use std::mem;
use std::ops::{Index, IndexMut};

use crate::terminal::model::grid::row::Row;
use crate::terminal::model::index::VisibleRow;

/// A circular buffer for optimizing indexing and rotation. In other words, a performant implementation of
/// a grid API.
///
/// The data is stored in a vector of rows where top_row tracks the very first row of the terminal (think, top row
/// of the visible grid) and bottom_row is the last visible grid row. The direction of the data can either be
/// forwards or backwards, depending on the feature flag SequentialStorage.
///
/// Rezeroing is the process of restoring the topmost line back to raw storage index 0. This allows us then
/// to extend or reduce the capacity of the underlying vector. The field "len" tracks the number of active
/// rows in the grid, which is different from the currently allocated capacity of the vector (self.inner.len()).
/// A more detailed explanation: when the grid grows in length, it does so in chunks of MAX_CACHE_SIZE - that way,
/// we don't extend the vector every newline (which constitutes a scroll_up event) but rather every 1000 rows. Because of
/// this, the value storage.len (number of active rows) will often be less than the value storage.inner.len()
/// (the actual allocated capacity).
///
/// The [`Storage::rotate`] and [`Storage::rotate_down`] functions are fast modular additions on
/// the internal [`bottom_row`] field. As compared with [`slice::rotate_left`] which must rearrange items
/// in memory.
///
/// As a consequence, both [`Index`] and [`IndexMut`] are reimplemented for this type to account
/// for the zeroth element not always being at the start of the allocation.
///
/// Because certain [`Vec`] operations are no longer valid on this type, no [`Deref`]
/// implementation is provided. Anything from [`Vec`] that should be exposed must be done so
/// manually.
///
/// [`slice::rotate_left`]: https://doc.rust-lang.org/std/primitive.slice.html#method.rotate_left
/// [`Deref`]: std::ops::Deref
/// [`bottom_row`]: #structfield.bottom_row
///
/// --------------------
///
/// Imagine we want to print Shakespeare plays in alphabetical order. Let's assume sequential storage.
///
/// 1) The first write takes place at index 0.
///    ┌────────────────────────────────┐
/// 0: │All's Well That Ends Well       │
/// 1: │                                │
/// 2: │                                │
/// 3: │                                │
/// 4: │                                │
/// 5: │                                │
/// 6: │                                │
/// 7: │                                │
/// 8: │                                │
/// 9: │                                │ <-- bottom_row (this is the bottom row because the grid
///    └────────────────────────────────┘                 was initialized with 10 active rows)
/// 2) The grid fills up at 10 rows.
///    ┌────────────────────────────────┐
/// 0: │All's Well That Ends Well (1602)│
/// 1: │Antony and Cleopatra (1606)     │
/// 2: │As You Like It (1599)           │
/// 3: │Comedy of Errors (1589)         │
/// 4: │Coriolanus (1607)               │
/// 5: │Cymbeline (1609)                │
/// 6: │Hamlet (1600)                   │
/// 7: │Henry IV, Part I (1597)         │
/// 8: │Henry IV, Part II (1597)        │
/// 9: │Henry V (1598)                  │ <-- bottom_row
///    └────────────────────────────────┘
///
/// 3) Adding 3 more rows means the oldest 3 rows are overwritten.
///    ┌────────────────────────────────┐
/// 0: │Henry VI, Part I (1591)         |
/// 1: │Henry VI, Part II (1590)        │
/// 2: │Henry VI, Part III (1590)       │ <-- bottom_row
/// 3: │Comedy of Errors (1589)         │ <-- self.top_row()
/// 4: │Coriolanus (1607)               │
/// 5: │Cymbeline (1609)                │
/// 6: │Hamlet (1600)                   │
/// 7: │Henry IV, Part I (1597)         │
/// 8: │Henry IV, Part II (1597)        │
/// 9: │Henry V (1598)                  │
///    └────────────────────────────────┘
#[derive(Serialize, Deserialize, Clone, Debug)]
pub(super) struct Storage {
    inner: Vec<Row>,

    /// The bottommost row of the visible grid. This is the furthest the grid extends and the maximum grid index.
    /// In an unrotated buffer, bottom_row = len - 1.
    bottom_row: usize,

    /// Number of visible lines. This is important because it's the initial grid size.
    visible_lines: usize,

    /// Total number of lines currently active in the terminal
    ///
    /// Shrinking this length allows reducing the number of lines in the scrollback buffer without
    /// having to truncate the raw `inner` buffer.
    /// As long as `len` is bigger than `inner`, it is also possible to grow the scrollback buffer
    /// without any additional insertions.
    len: usize,

    /// Whether or not the storage mechanism is reversed or sequential.
    #[serde(skip)]
    is_sequential: bool,

    /// Maximum number of buffered lines outside of the grid for performance optimization.
    /// Every time we extend the size of the grid, we do so in chunks of this size.
    #[serde(skip)]
    max_cache_size: usize,
}

impl PartialEq for Storage {
    fn eq(&self, other: &Self) -> bool {
        // Both storage buffers need to be truncated and zeroed.
        assert_eq!(self.bottom_row, 0);
        assert_eq!(other.bottom_row, 0);

        self.inner == other.inner && self.len == other.len
    }
}

impl Storage {
    #[inline]
    pub fn with_capacity(visible_lines: usize, cols: usize, is_sequential: bool) -> Storage {
        // Initialize visible lines; the scrollback buffer is initialized dynamically.
        let mut inner = Vec::with_capacity(visible_lines);
        inner.resize_with(visible_lines, || Row::new(cols));

        Self::with_rows(inner, is_sequential, visible_lines)
    }

    /// Initialize Storage with given vector of rows.
    #[inline]
    pub fn with_rows(rows: Vec<Row>, is_sequential: bool, visible_lines: usize) -> Storage {
        let len = rows.len();
        debug_assert!(
            visible_lines <= len,
            "Size of the visible grid cannot be bigger than the actual size of the grid."
        );
        let inner = rows;

        // We set this to 1 when using flat storage because there is no scrollback buffer
        // that we'll need to grow.
        let max_cache_size = 1;

        Storage {
            inner,
            bottom_row: 0,
            visible_lines,
            len,
            is_sequential,
            max_cache_size,
        }
    }

    pub fn is_sequential(&self) -> bool {
        self.is_sequential
    }

    /// Increase the number of lines in the buffer.
    #[inline]
    pub fn grow_visible_lines(&mut self, next: usize) {
        // Number of lines the buffer needs to grow.
        let growage = next - self.visible_lines;

        let cols = self[0].len();
        self.initialize(growage, cols);

        // Update visible lines.
        self.visible_lines = next;
    }

    /// Decrease the number of lines in the buffer.
    #[inline]
    pub fn shrink_visible_lines(&mut self, next: usize) {
        // Shrink the size without removing any lines.
        let shrinkage = self.visible_lines - next;
        self.shrink_lines(shrinkage);

        // Update visible lines.
        self.visible_lines = next;
    }

    /// Shrink the number of lines in the buffer.
    #[inline]
    pub fn shrink_lines(&mut self, shrinkage: usize) {
        self.len -= shrinkage;

        // Free memory.
        if self.inner.len() > self.len + self.max_cache_size {
            self.truncate_unused_rows();
        }
    }

    /// Truncate the invisible elements from the raw buffer.
    #[inline]
    pub fn truncate_unused_rows(&mut self) {
        self.rezero();
        self.inner.truncate(self.len);
        self.inner.shrink_to_fit();
    }

    /// Truncate columns in Storage to specified target # of columns.
    #[inline]
    pub fn truncate_columns(&mut self, col_to_truncate_to: usize) {
        for row in &mut self.inner {
            row.truncate(col_to_truncate_to);
        }
    }

    /// Truncate the ring buffer so that it only contains the `target_len` number of rows. Rows are
    /// filled from bottom to top (bottom_row), so elements are deleted from the beginning of the buffer.
    #[inline]
    pub fn truncate_to(&mut self, target_len: usize) {
        if target_len < self.len() {
            self.rezero();
            self.inner.drain(0..self.len - target_len);
            self.len = target_len;
            self.visible_lines = self.visible_lines.min(target_len);
        }

        // Now that we've shortened the vector, remove unused rows, if any.
        self.truncate_unused_rows();
    }

    pub fn push_from_scrollback(&mut self, mut rows: Vec<Row>) {
        let num_rows = rows.len();

        if !self.is_sequential {
            rows.reverse();
        }

        // Append the rows to the internal storage.
        self.rezero();
        self.inner.splice(self.len.., rows);

        // Increase the length to account for the additional rows we added.
        self.len += num_rows;
    }

    /// Dynamically grow the storage buffer at runtime.
    #[inline]
    pub fn initialize(&mut self, additional_rows: usize, cols: usize) {
        if self.len + additional_rows > self.inner.len() {
            self.rezero();

            let realloc_size = self.inner.len() + max(additional_rows, self.max_cache_size);
            self.inner.resize_with(realloc_size, || Row::new(cols));
        }

        self.len += additional_rows;
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    /// Swap two visible lines. This function takes visible rows and converts them
    /// to grid indices and then raw storage indices.
    pub fn swap_lines(&mut self, a: VisibleRow, b: VisibleRow) {
        let a = self.from_grid_index(self.to_grid_index(a));
        let b = self.from_grid_index(self.to_grid_index(b));

        self.inner.swap(a, b);
    }

    /// Swap implementation for Row.
    ///
    /// Exploits the known size of Row to produce a slightly more efficient
    /// swap than going through slice::swap.
    ///
    /// The default implementation from swap generates 8 movups and 4 movaps
    /// instructions. This implementation achieves the swap in only 8 movups
    /// instructions.
    ///
    /// This function accepts grid indices.
    pub fn swap(&mut self, a: usize, b: usize) {
        debug_assert_eq!(mem::size_of::<Row>(), mem::size_of::<usize>() * 4);

        let a = self.from_grid_index(a);
        let b = self.from_grid_index(b);

        unsafe {
            // Cast to a qword array to opt out of copy restrictions and avoid
            // drop hazards. Byte array is no good here since for whatever
            // reason LLVM won't optimized it.
            let a_ptr = self.inner.as_mut_ptr().add(a) as *mut usize;
            let b_ptr = self.inner.as_mut_ptr().add(b) as *mut usize;

            // Copy 1 qword at a time.
            //
            // The optimizer unrolls this loop and vectorizes it.
            let mut tmp: usize;
            for i in 0..4 {
                tmp = *a_ptr.offset(i);
                *a_ptr.offset(i) = *b_ptr.offset(i);
                *b_ptr.offset(i) = tmp;
            }
        }
    }

    #[inline]
    pub fn rotate(&mut self, count: isize) {
        debug_assert!(count.unsigned_abs() <= self.inner.len());

        let len = self.inner.len();
        self.bottom_row = (self.bottom_row as isize + count + len as isize) as usize % len;
    }

    /// Move the bottommost row to indicate a lesser row value.
    #[inline]
    pub fn retract(&mut self, mut count: isize) {
        debug_assert!(count.unsigned_abs() <= self.inner.len());

        if self.is_sequential {
            count *= -1;
        }

        let len = self.inner.len();
        self.bottom_row = (self.bottom_row as isize + count + len as isize) as usize % len;
    }

    /// Move the bottommost row to indicate a greater row value.
    #[inline]
    pub fn extend(&mut self, mut count: isize) {
        debug_assert!(count.unsigned_abs() <= self.inner.len());

        if !self.is_sequential {
            count *= -1;
        }

        let len = self.inner.len();
        self.bottom_row = (self.bottom_row as isize + count + len as isize) as usize % len;
    }

    /// Update the raw storage buffer.
    #[inline]
    pub fn replace_inner(&mut self, vec: Vec<Row>) {
        self.len = vec.len();
        self.inner = vec;
        self.bottom_row = 0;
    }

    /// Remove all rows from storage.
    #[inline]
    pub fn take_all(&mut self) -> Vec<Row> {
        self.truncate_unused_rows();

        let mut buffer = Vec::new();

        mem::swap(&mut buffer, &mut self.inner);
        self.len = 0;

        buffer
    }

    /// Rotate the ringbuffer to reset `self.bottom_row` back to index `0`.
    #[inline]
    fn rezero(&mut self) {
        if self.bottom_row == 0 {
            return;
        }

        self.inner.rotate_left(self.bottom_row);
        self.bottom_row = 0;
    }

    /// Convert from grid index to raw storage index
    #[inline]
    #[allow(clippy::wrong_self_convention)]
    pub fn from_grid_index(&self, mut requested: usize) -> usize {
        debug_assert!(requested < self.len);

        // If our storage is not sequential... reverse the index
        if !self.is_sequential {
            requested = self.len - requested - 1;
        }

        let zeroed = self.bottom_row + requested;

        // Use if/else instead of remainder here to improve performance.
        //
        // Requires `zeroed` to be smaller than `self.inner.len() * 2`,
        // but both `self.bottom_row` and `requested` are always smaller than `self.inner.len()`.
        if zeroed >= self.inner.len() {
            zeroed - self.inner.len()
        } else {
            zeroed
        }
    }

    /// Convert from visible row to grid index
    #[inline]
    pub fn to_grid_index(&self, row: VisibleRow) -> usize {
        self.len - self.visible_lines + row.0
    }

    pub fn estimated_heap_usage_bytes(&self) -> usize {
        // We expect all rows to have the same internal capacity, so we can
        // avoid summing up the heap usage from every row.
        self.inner.capacity() * self.inner[0].estimated_memory_usage_bytes()
    }
}

impl Index<usize> for Storage {
    type Output = Row;

    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        &self.inner[self.from_grid_index(index)]
    }
}

impl IndexMut<usize> for Storage {
    #[inline]
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        let index = self.from_grid_index(index); // borrowck
        &mut self.inner[index]
    }
}

impl Index<VisibleRow> for Storage {
    type Output = Row;

    #[inline]
    fn index(&self, row: VisibleRow) -> &Self::Output {
        &self[self.to_grid_index(row)]
    }
}

impl IndexMut<VisibleRow> for Storage {
    #[inline]
    fn index_mut(&mut self, row: VisibleRow) -> &mut Self::Output {
        let grid_index = self.to_grid_index(row);
        &mut self[grid_index]
    }
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
