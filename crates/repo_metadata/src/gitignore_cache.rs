//! A small process-wide cache of parsed `.gitignore` files.
//!
//! Constructing a [`Gitignore`] compiles a fresh `regex_automata` regex, and
//! that regex owns its own thread-safe `Pool` of per-thread search caches
//! (see `regex_automata::util::pool::Pool`). Re-parsing the same
//! `.gitignore` file on every file-tree traversal — which happens on every
//! watcher-triggered rebuild — creates a fresh pool each time. This cache
//! reuses a parsed, `Arc`-shared [`Gitignore`] across traversals as long as
//! the file's content is unchanged, so a given `.gitignore` path compiles
//! its regex (and allocates its pool) at most once until the file is
//! actually edited.
//!
//! Invalidation is keyed by a hash of the file's content rather than its
//! (mtime, length): a same-length edit within the filesystem's timestamp
//! granularity would otherwise be indistinguishable from an unchanged file,
//! serving stale ignore rules. Computing the hash means reading the file on
//! every call (in addition to the read `Gitignore::new` itself does when the
//! hash misses), but `.gitignore` files are small and, after the first
//! traversal, page-cached — this trades a cheap read for correctness rather
//! than reintroducing the compile cost this cache exists to avoid.
//!
//! The cache is bounded by estimated retained bytes, not entry count: a
//! `.gitignore` with many largely-distinct glob patterns can retain
//! megabytes in its compiled matcher (see [`RETAINED_BYTES_PER_SOURCE_BYTE`]
//! for the measurement backing that estimate), so a fixed entry-count cap
//! could not bound total memory. Entries beyond the byte budget are evicted
//! least-recently-used first.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use ignore::gitignore::Gitignore;

/// Estimated multiplier from a `.gitignore` file's source byte length to the
/// heap retained by its compiled matcher (regex_automata NFA/DFA tables,
/// aho-corasick tables, and the first accessing thread's `regex_automata`
/// `Pool` cache, once the matcher has actually been used). Measured with an
/// ad hoc RSS-delta harness: a 10-line, 78-byte gitignore retained ~5 KiB
/// (~65x), while a pathological gitignore of 1,000 largely-distinct
/// doublestar globs (29.5 KB) retained ~4.8 MiB (~163x) after one match.
/// Rounded up from the worse-observed ratio for margin.
const RETAINED_BYTES_PER_SOURCE_BYTE: u64 = 200;

/// Total estimated retained heap the cache may hold before it evicts
/// least-recently-used entries, independent of how many distinct
/// `.gitignore` paths that represents. At [`RETAINED_BYTES_PER_SOURCE_BYTE`],
/// this bounds worst-case retention to roughly thirteen pathological,
/// megabyte-scale gitignores, or many thousands of ordinary ones — generous
/// for the distinct `.gitignore` paths touched in a real session, while
/// remaining a small, fixed fraction of typical available memory.
#[cfg(not(test))]
const MAX_CACHE_WEIGHT_BYTES: u64 = 64 * 1024 * 1024;
/// Small in tests so eviction can be exercised without huge fixtures.
#[cfg(test)]
const MAX_CACHE_WEIGHT_BYTES: u64 = 5_000;

struct CacheEntry {
    content_digest: u64,
    gitignore: Arc<Gitignore>,
    /// Estimated retained bytes, per [`RETAINED_BYTES_PER_SOURCE_BYTE`].
    weight: u64,
    /// Tick from [`next_tick`] as of the last hit or insert. The entry with
    /// the smallest tick is evicted first once the cache is over budget.
    last_used: u64,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<PathBuf, CacheEntry>,
    total_weight: u64,
}

static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(Cache::default()));
static NEXT_TICK: AtomicU64 = AtomicU64::new(0);

fn next_tick() -> u64 {
    NEXT_TICK.fetch_add(1, Ordering::Relaxed)
}

/// A panic elsewhere while the lock is held must not permanently disable
/// every subsequent traversal; the cache is best-effort, so recovering the
/// (possibly inconsistent) inner state is preferable to poisoning it forever.
fn lock_cache() -> std::sync::MutexGuard<'static, Cache> {
    CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn content_digest(content: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

/// Returns a cached, parsed `.gitignore` matcher for `gitignore_path`,
/// reusing the previous parse when the file's content is unchanged and
/// re-parsing (and caching the fresh result) otherwise.
///
/// Never caches a result that doesn't reflect the file's current, complete
/// contents: a transient read failure or a parse error is returned directly
/// without touching the cache, so a stale or partial result can never shadow
/// a later, successful parse (e.g. after a permissions fix or an edit that
/// corrects a malformed glob line).
pub(crate) fn get_or_parse(gitignore_path: &Path) -> Arc<Gitignore> {
    let Ok(content) = std::fs::read(gitignore_path) else {
        // Can't fingerprint a file we can't read right now. Parse directly —
        // `Gitignore::new` fails open the same way on an unreadable file —
        // without disturbing any existing cache entry, so a later, readable
        // call still finds (or repopulates) a correct entry.
        let (gitignore, _) = Gitignore::new(gitignore_path);
        return Arc::new(gitignore);
    };
    let content_digest = content_digest(&content);

    {
        let mut cache = lock_cache();
        if let Some(entry) = cache.entries.get_mut(gitignore_path)
            && entry.content_digest == content_digest
        {
            entry.last_used = next_tick();
            return entry.gitignore.clone();
        }
    }

    // Parse outside the lock (`Gitignore::new` does its own blocking file
    // I/O) so a slow parse doesn't block unrelated cache lookups. A
    // concurrent caller parsing the same path at the same time is a
    // harmless, rare race: the last insert wins and both callers still get a
    // valid, usable matcher.
    let (gitignore, error) = Gitignore::new(gitignore_path);
    if error.is_some() {
        // A parse error (e.g. one malformed glob line) means this instance
        // doesn't fully represent the file. Don't cache it, so a later call
        // — after the file is fixed, but with the same content otherwise —
        // isn't shadowed by this partial result.
        return Arc::new(gitignore);
    }
    let gitignore = Arc::new(gitignore);
    let weight = content.len() as u64 * RETAINED_BYTES_PER_SOURCE_BYTE;

    let mut cache = lock_cache();
    if let Some(previous) = cache.entries.insert(
        gitignore_path.to_path_buf(),
        CacheEntry {
            content_digest,
            gitignore: gitignore.clone(),
            weight,
            last_used: next_tick(),
        },
    ) {
        cache.total_weight -= previous.weight;
    }
    cache.total_weight += weight;
    evict_if_over_budget(&mut cache);
    gitignore
}

/// Evicts least-recently-used entries until the cache is back under
/// [`MAX_CACHE_WEIGHT_BYTES`].
fn evict_if_over_budget(cache: &mut Cache) {
    if cache.total_weight <= MAX_CACHE_WEIGHT_BYTES {
        return;
    }
    let mut by_last_used: Vec<(PathBuf, u64, u64)> = cache
        .entries
        .iter()
        .map(|(path, entry)| (path.clone(), entry.last_used, entry.weight))
        .collect();
    by_last_used.sort_by_key(|(_, last_used, _)| *last_used);
    for (path, _, weight) in by_last_used {
        if cache.total_weight <= MAX_CACHE_WEIGHT_BYTES {
            break;
        }
        cache.entries.remove(&path);
        cache.total_weight -= weight;
    }
}

#[cfg(test)]
pub(crate) fn clear_for_test() {
    let mut cache = lock_cache();
    cache.entries.clear();
    cache.total_weight = 0;
}

#[cfg(test)]
#[path = "gitignore_cache_tests.rs"]
mod tests;
