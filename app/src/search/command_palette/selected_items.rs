use bounded_vec_deque::BoundedVecDeque;
use warpui::{Entity, SingletonEntity};

use crate::search::command_palette::mixer::ItemSummary;

/// Maximum number of elements to store. Per the [`BoundedVecDeque`] docs, it is recommended that
/// this is one less than the power of two to avoid unnecessary allocations.
///
/// Only a small set of selected items are stored (15). However, we store more items than we render
/// in the command palette since it's not guaranteed that all of the items are available at a
/// given time (available bindings are dependent on which view is focused, sessions could have been
/// closed, workflows could have been deleted).
const MAX_SIZE: usize = 15;

/// Store of all of recently selected items within the command palette. Only one item of any given
/// [`ItemSummary`] type is stored.
pub struct SelectedItems {
    items: BoundedVecDeque<ItemSummary>,
}

impl Default for SelectedItems {
    fn default() -> Self {
        Self::new()
    }
}

impl SelectedItems {
    pub fn new() -> Self {
        Self {
            items: BoundedVecDeque::new(MAX_SIZE),
        }
    }

    /// Enqueue a new `summary` into the list of [`SelectedItems`]. If the item is already in the
    /// list, it is removed and reinserted at the end.
    ///
    /// Upon insertion, if the max number of items exceeds that of [`MAX_SIZE`], items from the
    /// beginning of the list are removed.
    pub fn enqueue(&mut self, summary: ItemSummary) {
        if let Some(index) = self.items.iter().position(|item| item == &summary) {
            self.items.remove(index);
        }

        self.items.push_back(summary);
    }

    /// Returns an iterator of the recently selected items in reverse order of when they were
    /// selected (newly selected items are returned first).
    pub fn iter(&self) -> impl Iterator<Item = &ItemSummary> {
        self.items.iter().rev()
    }
}

impl Entity for SelectedItems {
    type Event = ();
}

impl SingletonEntity for SelectedItems {}

#[cfg(test)]
#[path = "selected_items_tests.rs"]
mod tests;
