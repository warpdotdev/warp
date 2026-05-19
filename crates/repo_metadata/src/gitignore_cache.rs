/// Global gitignore cache to share compiled regex pools across concurrent traversals.
///
/// The `ignore` crate's `Gitignore` type internally compiles glob patterns into
/// `regex_automata::meta::Regex` instances backed by `PikeVM` caches. Each cache
/// can be tens of megabytes for complex `.gitignore` patterns. When multiple
/// concurrent tasks (repo indexing, outline computation, codebase index diffing)
/// each create independent `Gitignore` instances for the same `.gitignore` files,
/// the duplicated caches can consume 10+ GB of memory.
///
/// This module provides a process-global cache keyed by (path, content_hash) so
/// that concurrent traversals share the same `Gitignore` instances and their
/// internal regex cache pools.
use ignore::gitignore::Gitignore;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

struct CachedEntry {
    gitignore: Gitignore,
    modified_time: Option<SystemTime>,
    content_hash: u64,
}

static GITIGNORE_CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedEntry>>> = OnceLock::new();
static GLOBAL_GITIGNORE: OnceLock<Gitignore> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<PathBuf, CachedEntry>> {
    GITIGNORE_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Fast hash of file content for cache invalidation.
/// `.gitignore` files are tiny so this is cheap compared to regex compilation.
fn hash_file_content(path: &Path) -> u64 {
    match std::fs::read(path) {
        Ok(bytes) => {
            let mut hasher = DefaultHasher::new();
            bytes.hash(&mut hasher);
            hasher.finish()
        }
        Err(_) => 0,
    }
}

/// Returns a `Gitignore` for the given path, reusing a cached instance when the
/// file hasn't been modified since it was last parsed. Cloning a `Gitignore`
/// shares the underlying compiled regex via `Arc`, so concurrent searches reuse
/// the same `PikeVM` cache pool instead of each allocating their own.
pub fn cached_gitignore_new(path: impl AsRef<Path>) -> Gitignore {
    let path = path.as_ref();
    let current_mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());

    let mut map = cache().lock().unwrap_or_else(|e| e.into_inner());

    if let Some(entry) = map.get(path) {
        // Fast path: if mtime changed, we know we need to re-parse.
        // If mtime is the same, also check content hash to catch same-second writes.
        if entry.modified_time == current_mtime {
            let content_hash = hash_file_content(path);
            if entry.content_hash == content_hash {
                return entry.gitignore.clone();
            }
        }
    }

    let content_hash = hash_file_content(path);
    let (gitignore, _err) = Gitignore::new(path);
    map.insert(
        path.to_path_buf(),
        CachedEntry {
            gitignore: gitignore.clone(),
            modified_time: current_mtime,
            content_hash,
        },
    );
    gitignore
}

/// Returns the user's global gitignore, cached for the lifetime of the process.
/// The global gitignore rarely changes, so a single instance is sufficient.
pub fn cached_gitignore_global() -> Gitignore {
    GLOBAL_GITIGNORE
        .get_or_init(|| {
            let (gitignore, _) = Gitignore::global();
            gitignore
        })
        .clone()
}

/// Evicts all entries from the cache. Useful in tests or after detecting that
/// `.gitignore` files may have changed on disk.
#[cfg(any(test, feature = "test-util"))]
pub fn clear_cache() {
    if let Some(map) = GITIGNORE_CACHE.get() {
        map.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}
