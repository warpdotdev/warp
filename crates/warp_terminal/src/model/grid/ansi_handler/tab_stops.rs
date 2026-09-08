use std::ops::{Index, IndexMut};

use crate::model::index::IndexRange;

/// Default tab interval, corresponding to terminfo `it` value.
const INITIAL_TABSTOPS: usize = 8;

#[derive(Clone)]
pub struct TabStops {
    tabs: Vec<bool>,
}

impl TabStops {
    #[inline]
    pub fn new(num_cols: usize) -> TabStops {
        TabStops {
            tabs: IndexRange::from(0..num_cols)
                .map(|i| i % INITIAL_TABSTOPS == 0)
                .collect::<Vec<bool>>(),
        }
    }

    /// Remove all tabstops.
    #[inline]
    pub fn clear_all(&mut self) {
        unsafe {
            std::ptr::write_bytes(self.tabs.as_mut_ptr(), 0, self.tabs.len());
        }
    }

    /// Increase tabstop capacity.
    #[inline]
    pub fn resize(&mut self, num_cols: usize) {
        let mut index = self.tabs.len();
        self.tabs.resize_with(num_cols, || {
            let is_tabstop = index.is_multiple_of(INITIAL_TABSTOPS);
            index += 1;
            is_tabstop
        });
    }
}

impl Index<usize> for TabStops {
    type Output = bool;

    fn index(&self, index: usize) -> &bool {
        &self.tabs[index]
    }
}

impl IndexMut<usize> for TabStops {
    fn index_mut(&mut self, index: usize) -> &mut bool {
        self.tabs.index_mut(index)
    }
}
