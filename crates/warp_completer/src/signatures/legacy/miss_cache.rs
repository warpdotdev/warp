use std::collections::{HashSet, VecDeque};
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

const MAX_CACHED_MISSES: usize = 256;

/// A bounded set of (lowercased) command names that recently failed to resolve to a signature,
/// used by `SignatureCache::misses`.
///
/// This is a plain FIFO, not an LRU: once at capacity, inserting a new entry always evicts the
/// *oldest-inserted* one, regardless of how recently any entry (including the one about to be
/// evicted) was looked up again. `CommandRegistry` is a shared `Arc` behind a single global
/// instance (see `CommandRegistry::global_instance`) that's called from multiple terminal
/// sessions/panes concurrently, each generating completions on a background thread pool -- so
/// this genuinely needs cross-thread synchronization, not just single-task interior mutability.
/// But a negative cache is inherently approximate: a false negative (a miss this forgot) only
/// costs one extra, cheap `lookup_fn` call, never a wrong answer. That's a much weaker
/// requirement than an LRU implies, and dropping recency tracking is what makes a `RwLock` (an
/// LRU would need every *hit* to also take a write lock, to move the entry to the front) the
/// natural fit here, since a hit against `contains` becomes a pure read that never mutates
/// anything -- the write lock is only ever needed for a genuinely new miss.
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
