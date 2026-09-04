//! A process-wide cache of parsed `.gitignore` matchers with scoped access.
//!
//! Cache-owned matchers never escape as `Arc`s. Parsing and matching are serialized through the
//! cache mutex, and a slot is reserved before a matcher is compiled. Consequently, the cache and
//! all concurrent source-backed operations own at most [`MAX_LIVE_MATCHERS`] compiled matchers.
//! Callers retain source paths between operations and materialize only one applicable matcher at a
//! time while evaluating a path.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use ignore::gitignore::Gitignore;
use parking_lot::Mutex;

#[cfg(not(test))]
const MAX_CACHED_SOURCE_BYTES: u64 = 384 * 1024;
#[cfg(test)]
const MAX_CACHED_SOURCE_BYTES: u64 = 24;

/// Maximum number of concurrently live matchers owned by the scoped source-backed cache.
pub const MAX_LIVE_MATCHERS: usize = 64;
#[cfg(test)]
const EFFECTIVE_MAX_LIVE_MATCHERS: usize = 3;
#[cfg(not(test))]
const EFFECTIVE_MAX_LIVE_MATCHERS: usize = MAX_LIVE_MATCHERS;

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum CacheKey {
    File(PathBuf),
    Global,
}

struct CacheEntry {
    content_digest: u64,
    gitignore: Gitignore,
    source_len: u64,
    last_used: u64,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<CacheKey, CacheEntry>,
    total_source_bytes: u64,
    next_tick: u64,
    #[cfg(test)]
    peak_live_matchers: usize,
    #[cfg(test)]
    parse_count: usize,
}

impl Cache {
    fn next_tick(&mut self) -> u64 {
        let tick = self.next_tick;
        self.next_tick += 1;
        tick
    }

    fn remove(&mut self, key: &CacheKey) {
        if let Some(entry) = self.entries.remove(key) {
            self.total_source_bytes -= entry.source_len;
        }
    }

    fn evict_lru(&mut self) -> bool {
        let Some(key) = self
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_used)
            .map(|(key, _)| key.clone())
        else {
            return false;
        };
        self.remove(&key);
        true
    }

    fn reserve_for(&mut self, source_len: u64) {
        while self.entries.len() >= EFFECTIVE_MAX_LIVE_MATCHERS
            || self.total_source_bytes.saturating_add(source_len) > MAX_CACHED_SOURCE_BYTES
        {
            if !self.evict_lru() {
                break;
            }
        }
    }

    #[cfg(test)]
    fn record_live_matchers(&mut self, transient_matchers: usize) {
        self.peak_live_matchers = self
            .peak_live_matchers
            .max(self.entries.len() + transient_matchers);
    }

    #[cfg(not(test))]
    fn record_live_matchers(&mut self, _transient_matchers: usize) {}

    #[cfg(test)]
    fn record_parse(&mut self) {
        self.parse_count += 1;
    }

    #[cfg(not(test))]
    fn record_parse(&mut self) {}

    fn cached_match(
        &mut self,
        key: &CacheKey,
        content_digest: u64,
        path: &Path,
        is_dir: bool,
        check_ancestors: bool,
    ) -> Option<bool> {
        if self.entries.get(key)?.content_digest != content_digest {
            return None;
        }
        let tick = self.next_tick();
        let entry = self.entries.get_mut(key)?;
        entry.last_used = tick;
        Some(matcher_ignores(
            &entry.gitignore,
            path,
            is_dir,
            check_ancestors,
        ))
    }

    fn match_file(
        &mut self,
        gitignore_path: &Path,
        content: Option<&[u8]>,
        path: &Path,
        is_dir: bool,
        check_ancestors: bool,
    ) -> bool {
        let key = CacheKey::File(gitignore_path.to_path_buf());
        let Some(content) = content else {
            self.remove(&key);
            self.reserve_for(0);
            self.record_live_matchers(1);
            self.record_parse();
            let (gitignore, _) = Gitignore::new(gitignore_path);
            return matcher_ignores(&gitignore, path, is_dir, check_ancestors);
        };
        let content_digest = content_digest(content);
        if let Some(is_ignored) =
            self.cached_match(&key, content_digest, path, is_dir, check_ancestors)
        {
            return is_ignored;
        }

        self.remove(&key);
        let source_len = content.len() as u64;
        self.reserve_for(source_len);
        self.record_live_matchers(1);
        self.record_parse();
        let (gitignore, error) = Gitignore::new(gitignore_path);
        if error.is_some() || source_len > MAX_CACHED_SOURCE_BYTES {
            return matcher_ignores(&gitignore, path, is_dir, check_ancestors);
        }

        let last_used = self.next_tick();
        self.entries.insert(
            key.clone(),
            CacheEntry {
                content_digest,
                gitignore,
                source_len,
                last_used,
            },
        );
        self.total_source_bytes += source_len;
        self.record_live_matchers(0);
        self.cached_match(&key, content_digest, path, is_dir, check_ancestors)
            .unwrap_or(false)
    }

    fn refresh_global(&mut self) {
        self.refresh_global_with(|| Gitignore::global().0);
    }

    fn refresh_global_with(&mut self, load: impl FnOnce() -> Gitignore) {
        self.remove(&CacheKey::Global);
        self.reserve_for(0);
        self.record_live_matchers(1);
        self.record_parse();
        let gitignore = load();
        if gitignore.is_empty() {
            return;
        }
        let last_used = self.next_tick();
        self.entries.insert(
            CacheKey::Global,
            CacheEntry {
                content_digest: 0,
                gitignore,
                source_len: 0,
                last_used,
            },
        );
        self.record_live_matchers(0);
    }

    fn match_global(&mut self, path: &Path, is_dir: bool, check_ancestors: bool) -> bool {
        if !self.entries.contains_key(&CacheKey::Global) {
            self.refresh_global();
        }
        self.cached_match(&CacheKey::Global, 0, path, is_dir, check_ancestors)
            .unwrap_or(false)
    }
}

static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(Cache::default()));

fn content_digest(content: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

fn matcher_ignores(
    gitignore: &Gitignore,
    path: &Path,
    is_dir: bool,
    check_ancestors: bool,
) -> bool {
    let Ok(relative_path) = path.strip_prefix(gitignore.path()) else {
        return false;
    };
    if relative_path.has_root() && (cfg!(windows) || gitignore.path() != Path::new("")) {
        return false;
    }
    if check_ancestors {
        gitignore
            .matched_path_or_any_parents(relative_path, is_dir)
            .is_ignore()
    } else {
        gitignore.matched(relative_path, is_dir).is_ignore()
    }
}

/// Source-backed Gitignore rules that do not retain compiled matchers between evaluations.
#[derive(Debug, Clone, Default)]
pub struct GitignoreRules {
    cached_paths: Arc<Vec<PathBuf>>,
    #[cfg(test)]
    persistent_matchers: Arc<Vec<Arc<Gitignore>>>,
    include_global: bool,
}

impl GitignoreRules {
    /// Creates rules that consult the current configured global Gitignore.
    pub fn global() -> Self {
        CACHE.lock().refresh_global();
        Self {
            include_global: true,
            ..Self::default()
        }
    }

    #[cfg(test)]
    fn with_cached_paths(self, cached_paths: Vec<PathBuf>) -> Self {
        Self {
            cached_paths: Arc::new(cached_paths),
            ..self
        }
    }

    pub(crate) fn add_cached_path(&mut self, path: PathBuf) {
        if !self.cached_paths.contains(&path) {
            Arc::make_mut(&mut self.cached_paths).push(path);
        }
    }

    /// Returns whether any applicable rule ignores `path`.
    pub fn matches(&self, path: &Path, is_dir: bool, check_ancestors: bool) -> bool {
        #[cfg(test)]
        if self
            .persistent_matchers
            .iter()
            .any(|matcher| matcher_ignores(matcher, path, is_dir, check_ancestors))
        {
            return true;
        }

        let mut cache = CACHE.lock();
        if self.include_global && cache.match_global(path, is_dir, check_ancestors) {
            return true;
        }
        self.cached_paths.iter().any(|gitignore_path| {
            let applies = gitignore_path
                .parent()
                .is_some_and(|root| path.starts_with(root));
            applies
                && cache.match_file(
                    gitignore_path,
                    std::fs::read(gitignore_path).ok().as_deref(),
                    path,
                    is_dir,
                    check_ancestors,
                )
        })
    }

    #[cfg(test)]
    fn matches_with_cache(
        &self,
        cache: &Mutex<Cache>,
        path: &Path,
        is_dir: bool,
        check_ancestors: bool,
    ) -> bool {
        let mut cache = cache.lock();
        self.cached_paths.iter().any(|gitignore_path| {
            cache.match_file(
                gitignore_path,
                std::fs::read(gitignore_path).ok().as_deref(),
                path,
                is_dir,
                check_ancestors,
            )
        })
    }
}

#[cfg(test)]
impl From<Vec<Arc<Gitignore>>> for GitignoreRules {
    fn from(persistent_matchers: Vec<Arc<Gitignore>>) -> Self {
        Self {
            persistent_matchers: Arc::new(persistent_matchers),
            ..Self::default()
        }
    }
}

#[cfg(test)]
#[path = "gitignore_cache_tests.rs"]
mod tests;
