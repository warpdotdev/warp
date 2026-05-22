//! Iterator implementations for FileTreeSnapshot.

use std::path::Path;
use std::sync::Arc;

use sum_tree::{Cursor, SeekBias};

use super::{FileEntry, FileTreeSnapshot, PathKey};

/// Iterator over direct children of a directory.
///
/// # How it works
///
/// Entries in the SumTree are sorted lexicographically by path. This means all entries
/// within a directory's subtree are **contiguous** in the sorted order:
///
/// ```text
/// Sorted entries for child_entries("/project/src/"):
/// ┌─────────────┬─────────────────────┬──────────────────────┬─────────────────────────────┬─────────────┐
/// │ /project/   │ /project/src/       │ /project/src/lib.rs  │ /project/src/utils/         │ /project/z  │
/// │             │ (skip: parent)      │ ✓ yield (1 comp)     │ ✓ yield (1 comp)            │ (stop)      │
/// │             │         ↓           │                      │                             │             │
/// │             │  cursor starts here │                      │ /project/src/utils/helper.rs│             │
/// │             │                     │                      │ (skip: 2 components)        │             │
/// └─────────────┴─────────────────────┴──────────────────────┴─────────────────────────────┴─────────────┘
/// ```
///
/// The iterator:
/// 1. Seeks to the parent path in O(log n)
/// 2. Skips the parent directory entry itself
/// 3. Iterates forward, yielding entries with exactly 1 path component after the parent prefix
/// 4. Skips deeper descendants (2+ components) — they'll be visited when their parent is expanded
/// 5. Stops when reaching an entry outside the parent's subtree
pub struct ChildEntriesIter<'a> {
    cursor: Cursor<'a, FileEntry, PathKey, ()>,
    parent_path: &'a Path,
    done: bool,
}

impl<'a> ChildEntriesIter<'a> {
    pub(super) fn new(snapshot: &'a FileTreeSnapshot, parent_path: &'a Path) -> Self {
        let mut cursor = snapshot.entries_by_path.cursor::<PathKey, ()>();
        let key = PathKey::new(Arc::from(parent_path));
        cursor.seek(&key, SeekBias::Left);

        // Skip past the parent directory itself
        if cursor
            .item()
            .is_some_and(|e| e.path.as_ref() == parent_path)
        {
            cursor.next();
        }

        Self {
            cursor,
            parent_path,
            done: false,
        }
    }
}

impl<'a> Iterator for ChildEntriesIter<'a> {
    type Item = &'a FileEntry;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        loop {
            let entry = self.cursor.item()?;

            // Stop if we've moved past the parent's subtree (lexicographically)
            if !entry.path.starts_with(self.parent_path) {
                self.done = true;
                return None;
            }

            // Count path components after the parent prefix to determine depth
            let relative = entry.path.strip_prefix(self.parent_path).ok()?;
            let components: Vec<_> = relative.components().collect();

            self.cursor.next();

            // Yield only direct children (exactly 1 component after parent)
            // Skip grandchildren and deeper (2+ components)
            if components.len() == 1 {
                return Some(entry);
            }
        }
    }
}
