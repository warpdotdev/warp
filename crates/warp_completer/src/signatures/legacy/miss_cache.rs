use std::collections::{HashSet, VecDeque};
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

const MAX_CACHED_MISSES: usize = 256;

/// A bounded, `RwLock`-guarded FIFO set of (lowercased) command names that recently failed to
/// resolve to a signature. FIFO rather than LRU because a lookup here is a pure read that never
/// needs to reorder anything, so the write lock is only needed for a genuinely new miss.
pub(super) struct MissCache {
    capacity: usize,
    entries: RwLock<MissCacheEntries>,
}

#[derive(Default)]
struct MissCacheEntries {
    /// Insertion order, oldest first, used to find the next entry to evict once at capacity.
    order: VecDeque<String>,
    /// The actual set of currently-remembered misses, for O(1) membership checks in `contains`.
    set: HashSet<String>,
}

impl Default for MissCache {
    fn default() -> Self {
        Self::new(MAX_CACHED_MISSES)
    }
}

impl MissCache {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: RwLock::default(),
        }
    }

    /// Returns `true` if `command` was recently recorded as a miss. A pure read: does not
    /// affect eviction order.
    pub(super) fn contains(&self, command: &str) -> bool {
        self.read().set.contains(command)
    }

    /// Returns the number of misses currently recorded, for tests.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.read().set.len()
    }

    /// Records `command` as a miss, evicting the oldest-recorded miss first if already at
    /// capacity.
    pub(super) fn insert(&self, command: String) {
        let mut entries = self.write();
        if entries.set.contains(&command) {
            return;
        }
        if entries.order.len() >= self.capacity
            && let Some(oldest) = entries.order.pop_front()
        {
            entries.set.remove(&oldest);
        }
        entries.order.push_back(command.clone());
        entries.set.insert(command);
    }

    fn read(&self) -> RwLockReadGuard<'_, MissCacheEntries> {
        self.entries
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(&self) -> RwLockWriteGuard<'_, MissCacheEntries> {
        self.entries
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
#[path = "miss_cache_tests.rs"]
mod tests;
