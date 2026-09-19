//! A process-wide cache of parsed `.gitignore` matchers with scoped access.
//!
//! Each source is read at most once per matching operation, outside the cache mutex.
//! Cache-owned matchers never escape as `Arc`s, and a slot is reserved before a matcher is
//! compiled. Consequently, the cache and all concurrent source-backed operations own at most
//! [`MAX_LIVE_MATCHERS`] compiled matchers.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use ignore::gitignore::{Gitignore, GitignoreBuilder, gitconfig_excludes_path};
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

#[derive(Debug, Clone)]
struct SourceSnapshot {
    key: CacheKey,
    source_path: PathBuf,
    matcher_root: PathBuf,
    content: Option<Arc<[u8]>>,
    content_digest: u64,
}

impl SourceSnapshot {
    fn file(path: PathBuf) -> Self {
        let matcher_root = path.parent().unwrap_or(Path::new("/")).to_path_buf();
        Self::read(CacheKey::File(path.clone()), path, matcher_root)
    }

    fn global(
        #[cfg(test)] path_override: Option<&Path>,
        #[cfg(test)] root_override: Option<&Path>,
    ) -> Self {
        let source_path = {
            #[cfg(test)]
            if let Some(path) = path_override {
                path.to_path_buf()
            } else {
                gitconfig_excludes_path().unwrap_or_default()
            }
            #[cfg(not(test))]
            {
                gitconfig_excludes_path().unwrap_or_default()
            }
        };
        let matcher_root = {
            #[cfg(test)]
            if let Some(root) = root_override {
                root.to_path_buf()
            } else {
                std::env::current_dir().unwrap_or_default()
            }
            #[cfg(not(test))]
            {
                std::env::current_dir().unwrap_or_default()
            }
        };
        Self::read(CacheKey::Global, source_path, matcher_root)
    }

    fn read(key: CacheKey, source_path: PathBuf, matcher_root: PathBuf) -> Self {
        let content = std::fs::read(&source_path).ok().map(Arc::from);
        let content_digest = source_digest(&source_path, &matcher_root, content.as_deref());
        Self {
            key,
            source_path,
            matcher_root,
            content,
            content_digest,
        }
    }

    fn source_len(&self) -> u64 {
        self.content
            .as_ref()
            .map_or(0, |content| content.len() as u64)
    }
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
        source: &SourceSnapshot,
        path: &Path,
        is_dir: bool,
        check_ancestors: bool,
    ) -> Option<bool> {
        if self.entries.get(&source.key)?.content_digest != source.content_digest {
            return None;
        }
        let tick = self.next_tick();
        let entry = self.entries.get_mut(&source.key)?;
        entry.last_used = tick;
        Some(matcher_ignores(
            &entry.gitignore,
            path,
            is_dir,
            check_ancestors,
        ))
    }

    fn match_source(
        &mut self,
        source: &SourceSnapshot,
        path: &Path,
        is_dir: bool,
        check_ancestors: bool,
    ) -> bool {
        let Some(content) = source.content.as_deref() else {
            self.remove(&source.key);
            return false;
        };
        if let Some(is_ignored) = self.cached_match(source, path, is_dir, check_ancestors) {
            return is_ignored;
        }

        self.remove(&source.key);
        let source_len = source.source_len();
        self.reserve_for(source_len);
        self.record_live_matchers(1);
        self.record_parse();
        let (gitignore, has_error) = compile_source(source, content);
        if has_error || source_len > MAX_CACHED_SOURCE_BYTES {
            return matcher_ignores(&gitignore, path, is_dir, check_ancestors);
        }

        let last_used = self.next_tick();
        self.entries.insert(
            source.key.clone(),
            CacheEntry {
                content_digest: source.content_digest,
                gitignore,
                source_len,
                last_used,
            },
        );
        self.total_source_bytes += source_len;
        self.record_live_matchers(0);
        self.cached_match(source, path, is_dir, check_ancestors)
            .unwrap_or(false)
    }
}

static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(Cache::default()));

fn source_digest(source_path: &Path, matcher_root: &Path, content: Option<&[u8]>) -> u64 {
    let mut hasher = DefaultHasher::new();
    source_path.hash(&mut hasher);
    matcher_root.hash(&mut hasher);
    content.hash(&mut hasher);
    hasher.finish()
}

fn compile_source(source: &SourceSnapshot, content: &[u8]) -> (Gitignore, bool) {
    let mut builder = GitignoreBuilder::new(&source.matcher_root);
    let mut has_error = false;
    for (index, line) in content.split(|byte| *byte == b'\n').enumerate() {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Ok(line) = std::str::from_utf8(line) else {
            has_error = true;
            break;
        };
        let line = if index == 0 {
            line.trim_start_matches('\u{feff}')
        } else {
            line
        };
        has_error |= builder
            .add_line(Some(source.source_path.clone()), line)
            .is_err();
    }
    match builder.build() {
        Ok(gitignore) => (gitignore, has_error),
        Err(_) => (Gitignore::empty(), true),
    }
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

/// Source-backed Gitignore rules that do not retain compiled matchers between operations.
#[derive(Debug, Clone, Default)]
pub struct GitignoreRules {
    cached_paths: Arc<Vec<PathBuf>>,
    #[cfg(test)]
    persistent_matchers: Arc<Vec<Arc<Gitignore>>>,
    include_global: bool,
    #[cfg(test)]
    global_path_override: Option<PathBuf>,
    #[cfg(test)]
    global_root_override: Option<PathBuf>,
}

impl GitignoreRules {
    /// Creates rules that consult the configured global Gitignore.
    pub fn global() -> Self {
        Self {
            include_global: true,
            ..Self::default()
        }
    }

    pub(crate) fn for_directory(directory: &Path) -> Self {
        let mut rules = Self::global();
        rules.add_cached_path(directory.join(".gitignore"));
        rules
    }

    /// Reads and fingerprints each source once for a matching operation.
    pub fn operation(&self) -> GitignoreOperation {
        GitignoreOperation::new(self.clone())
    }

    #[cfg(test)]
    fn with_cached_paths(self, cached_paths: Vec<PathBuf>) -> Self {
        Self {
            cached_paths: Arc::new(cached_paths),
            ..self
        }
    }

    #[cfg(test)]
    fn with_global_source(self, source_path: PathBuf, matcher_root: PathBuf) -> Self {
        Self {
            include_global: true,
            global_path_override: Some(source_path),
            global_root_override: Some(matcher_root),
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
        self.operation().matches(path, is_dir, check_ancestors)
    }
}

/// Source snapshots reused for one tree build, watcher classification scope, or outline update.
#[derive(Debug)]
pub struct GitignoreOperation {
    rules: GitignoreRules,
    sources: Vec<SourceSnapshot>,
    global_source: Option<SourceSnapshot>,
    #[cfg(test)]
    source_read_count: usize,
}

impl GitignoreOperation {
    fn new(rules: GitignoreRules) -> Self {
        let sources = rules
            .cached_paths
            .iter()
            .cloned()
            .map(SourceSnapshot::file)
            .collect::<Vec<_>>();
        let global_source = rules.include_global.then(|| {
            SourceSnapshot::global(
                #[cfg(test)]
                rules.global_path_override.as_deref(),
                #[cfg(test)]
                rules.global_root_override.as_deref(),
            )
        });
        #[cfg(test)]
        let source_read_count = sources.len() + usize::from(global_source.is_some());
        Self {
            rules,
            sources,
            global_source,
            #[cfg(test)]
            source_read_count,
        }
    }

    pub(crate) fn add_cached_path(&mut self, path: PathBuf) {
        if !self.rules.cached_paths.contains(&path) {
            self.rules.add_cached_path(path.clone());
            self.sources.push(SourceSnapshot::file(path));
            #[cfg(test)]
            {
                self.source_read_count += 1;
            }
        }
    }

    /// Returns whether any source captured for this operation ignores `path`.
    pub fn matches(&self, path: &Path, is_dir: bool, check_ancestors: bool) -> bool {
        #[cfg(test)]
        if self
            .rules
            .persistent_matchers
            .iter()
            .any(|matcher| matcher_ignores(matcher, path, is_dir, check_ancestors))
        {
            return true;
        }

        if self.global_source.as_ref().is_some_and(|source| {
            CACHE
                .lock()
                .match_source(source, path, is_dir, check_ancestors)
        }) {
            return true;
        }
        self.sources.iter().any(|source| {
            let applies = source
                .source_path
                .parent()
                .is_some_and(|root| path.starts_with(root));
            applies
                && CACHE
                    .lock()
                    .match_source(source, path, is_dir, check_ancestors)
        })
    }

    pub(crate) fn into_rules(self) -> GitignoreRules {
        self.rules
    }

    #[cfg(test)]
    fn matches_with_cache(
        &self,
        cache: &Mutex<Cache>,
        path: &Path,
        is_dir: bool,
        check_ancestors: bool,
    ) -> bool {
        self.sources.iter().any(|source| {
            cache
                .lock()
                .match_source(source, path, is_dir, check_ancestors)
        })
    }

    #[cfg(test)]
    fn source_read_count(&self) -> usize {
        self.source_read_count
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
pub(crate) fn cache_live_matcher_counts() -> (usize, usize) {
    let cache = CACHE.lock();
    (cache.entries.len(), cache.peak_live_matchers)
}

#[cfg(test)]
#[path = "gitignore_cache_tests.rs"]
mod tests;
