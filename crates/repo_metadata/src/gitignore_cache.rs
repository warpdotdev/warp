//! A small process-wide cache of parsed `.gitignore` files.
//!
//! Constructing a [`Gitignore`] compiles a fresh `regex_automata` regex, and that regex owns
//! its own thread-safe `Pool` of per-thread search caches (see
//! `regex_automata::util::pool::Pool`). Re-parsing the same `.gitignore` file on every
//! file-tree traversal — which happens on every watcher-triggered rebuild — creates a fresh
//! pool each time. This cache reuses a parsed, `Arc`-shared [`Gitignore`] across traversals as
//! long as the file's content is unchanged, so a given `.gitignore` path compiles its regex
//! (and allocates its pool) at most once until the file is actually edited.
//!
//! Invalidation is keyed by a hash of the file's content, so an edit is detected even when it
//! preserves the file's mtime and byte length. Hashing means an extra read on every call (in
//! addition to the read `Gitignore::new` itself does on a miss), but `.gitignore` files are
//! small and page-cached after the first traversal, so this stays cheap relative to the parse
//! it guards.
//!
//! Eviction is bounded by both source bytes and matcher count (see
//! [`MAX_CACHED_SOURCE_BYTES`] and [`MAX_CACHED_MATCHERS`]).

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use ignore::gitignore::Gitignore;
use parking_lot::Mutex;

/// Total `.gitignore` source bytes the cache may hold before evicting least-recently-used
/// entries, bounding memory regardless of `.gitignore` count. A compiled matcher can retain up
/// to ~163x its source size in the worst case observed, so this bounds worst-case retained
/// heap to roughly 60 MiB.
#[cfg(not(test))]
const MAX_CACHED_SOURCE_BYTES: u64 = 384 * 1024;
/// Small in tests so eviction can be exercised without huge fixtures.
#[cfg(test)]
const MAX_CACHED_SOURCE_BYTES: u64 = 24;
/// Maximum number of compiled matchers retained process-wide.
pub const MAX_CACHED_MATCHERS: usize = 64;
/// Small in tests so count-based eviction can be exercised with small fixtures.
#[cfg(test)]
pub(crate) const EFFECTIVE_MAX_CACHED_MATCHERS: usize = 3;
#[cfg(not(test))]
const EFFECTIVE_MAX_CACHED_MATCHERS: usize = MAX_CACHED_MATCHERS;

/// Gitignore sources that can materialize compiled matchers for an operation without retaining
/// cache-managed matchers between operations.
#[derive(Debug, Clone, Default)]
pub struct GitignoreRules {
    cached_paths: Arc<Vec<PathBuf>>,
    persistent_matchers: Arc<Vec<Arc<Gitignore>>>,
}
static GLOBAL_GITIGNORE: LazyLock<Option<Arc<Gitignore>>> = LazyLock::new(|| {
    let (global_gitignore, _) = Gitignore::global();
    (!global_gitignore.is_empty()).then(|| Arc::new(global_gitignore))
});

impl GitignoreRules {
    /// Returns rules containing the global gitignore when one is configured.
    pub fn global() -> Self {
        let persistent_matchers = GLOBAL_GITIGNORE.iter().cloned().collect();
        Self {
            cached_paths: Arc::default(),
            persistent_matchers: Arc::new(persistent_matchers),
        }
    }

    /// Replaces the cache-managed source paths while preserving persistent matchers.
    pub fn with_cached_paths(self, cached_paths: Vec<PathBuf>) -> Self {
        Self {
            cached_paths: Arc::new(cached_paths),
            ..self
        }
    }

    /// Materializes matchers for one operation.
    pub fn matchers(&self) -> Vec<Arc<Gitignore>> {
        self.persistent_matchers
            .iter()
            .cloned()
            .chain(self.cached_paths.iter().map(|path| get_or_parse(path)))
            .collect()
    }
}

impl From<Vec<Arc<Gitignore>>> for GitignoreRules {
    fn from(persistent_matchers: Vec<Arc<Gitignore>>) -> Self {
        Self {
            cached_paths: Arc::default(),
            persistent_matchers: Arc::new(persistent_matchers),
        }
    }
}

struct CacheEntry {
    content_digest: u64,
    gitignore: Arc<Gitignore>,
    source_len: u64,
    /// Tick from [`Cache::next_tick`] as of the last hit or insert; the LRU eviction key (the
    /// entry with the smallest tick is evicted first).
    last_used: u64,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<PathBuf, CacheEntry>,
    total_source_bytes: u64,
    /// Recency counter, only ever accessed while `CACHE`'s mutex is held.
    next_tick: u64,
}

impl Cache {
    fn next_tick(&mut self) -> u64 {
        let tick = self.next_tick;
        self.next_tick += 1;
        tick
    }

    /// Returns the cached matcher for `path` if its content digest matches `content_digest`.
    fn lookup(&mut self, path: &Path, content_digest: u64) -> Option<Arc<Gitignore>> {
        if self.entries.get(path)?.content_digest != content_digest {
            return None;
        }
        let tick = self.next_tick();
        let entry = self.entries.get_mut(path)?;
        entry.last_used = tick;
        Some(entry.gitignore.clone())
    }

    /// Inserts a freshly parsed matcher for `path`, replacing any previous entry.
    fn insert(
        &mut self,
        path: PathBuf,
        content_digest: u64,
        gitignore: Arc<Gitignore>,
        source_len: u64,
    ) {
        let last_used = self.next_tick();
        if let Some(previous) = self.entries.insert(
            path,
            CacheEntry {
                content_digest,
                gitignore,
                source_len,
                last_used,
            },
        ) {
            self.total_source_bytes -= previous.source_len;
        }
        self.total_source_bytes += source_len;
        self.evict_if_over_budget();
    }

    /// Evicts least-recently-used entries until the cache is under both retention limits.
    fn evict_if_over_budget(&mut self) {
        if self.total_source_bytes <= MAX_CACHED_SOURCE_BYTES
            && self.entries.len() <= EFFECTIVE_MAX_CACHED_MATCHERS
        {
            return;
        }
        let mut by_last_used: Vec<(PathBuf, u64, u64)> = self
            .entries
            .iter()
            .map(|(path, entry)| (path.clone(), entry.last_used, entry.source_len))
            .collect();
        by_last_used.sort_by_key(|(_, last_used, _)| *last_used);
        for (path, _, source_len) in by_last_used {
            if self.total_source_bytes <= MAX_CACHED_SOURCE_BYTES
                && self.entries.len() <= EFFECTIVE_MAX_CACHED_MATCHERS
            {
                break;
            }
            self.entries.remove(&path);
            self.total_source_bytes -= source_len;
        }
    }
}

static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(Cache::default()));

fn content_digest(content: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

/// Returns a cached, parsed `.gitignore` matcher for `gitignore_path`, reusing the previous
/// parse when the file's content is unchanged and re-parsing (and caching the fresh result)
/// otherwise.
///
/// Never caches a result that doesn't reflect the file's current, complete contents: a
/// transient read failure or a parse error is returned directly without touching the cache, so
/// a stale or partial result can never shadow a later, successful parse (e.g. after a
/// permissions fix or an edit that corrects a malformed glob line).
pub fn get_or_parse(gitignore_path: &Path) -> Arc<Gitignore> {
    let Ok(content) = std::fs::read(gitignore_path) else {
        // Can't fingerprint a file we can't read right now. Parse directly — `Gitignore::new`
        // fails open the same way on an unreadable file — without disturbing any existing
        // cache entry, so a later, readable call still finds (or repopulates) a correct entry.
        let (gitignore, _) = Gitignore::new(gitignore_path);
        return Arc::new(gitignore);
    };
    let content_digest = content_digest(&content);

    if let Some(gitignore) = CACHE.lock().lookup(gitignore_path, content_digest) {
        return gitignore;
    }

    // Parse outside the lock (`Gitignore::new` does its own blocking file I/O) so a slow parse
    // doesn't block unrelated cache lookups. A concurrent caller parsing the same path at the
    // same time is a harmless, rare race: the last insert wins and both callers still get a
    // valid, usable matcher.
    let (gitignore, error) = Gitignore::new(gitignore_path);
    if error.is_some() {
        // A parse error (e.g. one malformed glob line) means this instance doesn't fully
        // represent the file. Don't cache it, so a later call — after the file is fixed, but
        // with the same content otherwise — isn't shadowed by this partial result.
        return Arc::new(gitignore);
    }
    let gitignore = Arc::new(gitignore);
    let source_len = content.len() as u64;

    CACHE.lock().insert(
        gitignore_path.to_path_buf(),
        content_digest,
        gitignore.clone(),
        source_len,
    );
    gitignore
}

#[cfg(test)]
pub(crate) fn clear_for_test() {
    let mut cache = CACHE.lock();
    cache.entries.clear();
    cache.total_source_bytes = 0;
    cache.next_tick = 0;
}

#[cfg(test)]
#[path = "gitignore_cache_tests.rs"]
mod tests;
