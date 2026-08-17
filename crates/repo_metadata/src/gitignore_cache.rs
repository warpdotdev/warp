//! A small process-wide cache of parsed `.gitignore` files.
//!
//! Constructing a [`Gitignore`] compiles a fresh `regex_automata` regex, and
//! that regex owns its own thread-safe `Pool` of per-thread search caches
//! (see `regex_automata::util::pool::Pool`). Re-parsing the same
//! `.gitignore` file on every file-tree traversal — which happens on every
//! watcher-triggered rebuild — creates a fresh pool each time. This cache
//! reuses a parsed, `Arc`-shared [`Gitignore`] across traversals as long as
//! the file's mtime and length haven't changed, so a given `.gitignore` path
//! compiles its regex (and allocates its pool) at most once until the file
//! is actually edited.
//!
//! The cache is bounded so a long-running process that has ever touched many
//! distinct `.gitignore` paths (e.g. many short-lived workspaces over a long
//! session) can't grow it without limit; entries beyond the cap are evicted
//! least-recently-used first.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::SystemTime;

use ignore::gitignore::Gitignore;

/// Maximum number of parsed `.gitignore` files to retain. Bounds the cache's
/// memory to a fixed number of compiled matchers regardless of how many
/// distinct `.gitignore` paths a long-running process has ever seen.
#[cfg(not(test))]
const MAX_CACHED_GITIGNORES: usize = 4096;
/// Small in tests so eviction can be exercised without creating thousands of
/// temporary files.
#[cfg(test)]
const MAX_CACHED_GITIGNORES: usize = 3;

/// On-disk fingerprint used to detect changes without re-reading file content.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Fingerprint {
    modified: Option<SystemTime>,
    len: u64,
}

impl Fingerprint {
    fn for_path(path: &Path) -> Self {
        let metadata = std::fs::metadata(path).ok();
        Self {
            modified: metadata
                .as_ref()
                .and_then(|metadata| metadata.modified().ok()),
            len: metadata
                .as_ref()
                .map(|metadata| metadata.len())
                .unwrap_or(0),
        }
    }
}

struct CacheEntry {
    fingerprint: Fingerprint,
    gitignore: Arc<Gitignore>,
    /// Tick from [`next_tick`] as of the last hit or insert. The entry with
    /// the smallest tick is evicted first once the cache is over capacity.
    last_used: u64,
}

static CACHE: LazyLock<Mutex<HashMap<PathBuf, CacheEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_TICK: AtomicU64 = AtomicU64::new(0);

fn next_tick() -> u64 {
    NEXT_TICK.fetch_add(1, Ordering::Relaxed)
}

/// Returns a cached, parsed `.gitignore` matcher for `gitignore_path`,
/// reusing the previous parse when the file's mtime and length are
/// unchanged and re-parsing (and caching the fresh result) otherwise.
///
/// Like [`Gitignore::new`], this tolerates a missing or unreadable file by
/// treating it as an empty fingerprint rather than erroring; callers
/// typically only call this after confirming the path exists.
pub(crate) fn get_or_parse(gitignore_path: &Path) -> Arc<Gitignore> {
    let fingerprint = Fingerprint::for_path(gitignore_path);

    {
        let mut cache = CACHE.lock().unwrap();
        if let Some(entry) = cache.get_mut(gitignore_path)
            && entry.fingerprint == fingerprint
        {
            entry.last_used = next_tick();
            return entry.gitignore.clone();
        }
    }

    // Parse outside the lock (`Gitignore::new` does blocking file I/O) so a
    // slow parse doesn't block unrelated cache lookups. A concurrent caller
    // parsing the same path at the same time is a harmless, rare race: the
    // last insert wins and both callers still get a valid, usable matcher.
    let (gitignore, _) = Gitignore::new(gitignore_path);
    let gitignore = Arc::new(gitignore);

    let mut cache = CACHE.lock().unwrap();
    cache.insert(
        gitignore_path.to_path_buf(),
        CacheEntry {
            fingerprint,
            gitignore: gitignore.clone(),
            last_used: next_tick(),
        },
    );
    evict_if_over_capacity(&mut cache);
    gitignore
}

/// Evicts the least-recently-used entries once the cache exceeds
/// [`MAX_CACHED_GITIGNORES`].
fn evict_if_over_capacity(cache: &mut HashMap<PathBuf, CacheEntry>) {
    if cache.len() <= MAX_CACHED_GITIGNORES {
        return;
    }
    let excess = cache.len() - MAX_CACHED_GITIGNORES;
    let mut by_last_used: Vec<(PathBuf, u64)> = cache
        .iter()
        .map(|(path, entry)| (path.clone(), entry.last_used))
        .collect();
    by_last_used.sort_by_key(|(_, last_used)| *last_used);
    for (path, _) in by_last_used.into_iter().take(excess) {
        cache.remove(&path);
    }
}

#[cfg(test)]
pub(crate) fn clear_for_test() {
    CACHE.lock().unwrap().clear();
}

#[cfg(test)]
#[path = "gitignore_cache_tests.rs"]
mod tests;
