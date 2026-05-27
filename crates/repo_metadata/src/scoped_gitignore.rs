use std::collections::{hash_map::DefaultHasher, HashMap};
use std::hash::{Hash as _, Hasher as _};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use ignore::Match;

const CODEBASE_INDEX_IGNORE_FILES: &[&str] = &[
    ".warpindexingignore",
    ".cursorignore",
    ".cursorindexingignore",
    ".codeiumignore",
];

#[derive(Debug, Clone)]
pub struct GitignoreRuleCache {
    root_path: PathBuf,
    entries: HashMap<GitignoreRuleKey, GitignoreRule>,
    root_scoped_ignore_files: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
struct GitignoreRule {
    matcher: Gitignore,
    metadata: Option<GitignoreFileMetadata>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum GitignoreRuleKey {
    Global,
    File(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GitignoreFileMetadata {
    modified: Option<SystemTime>,
    len: u64,
    content_hash: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitignoreMatch {
    Ignore,
    Whitelist,
}

pub struct ScopedGitignoreTraversal<'a> {
    cache: &'a mut GitignoreRuleCache,
    active_keys: Vec<GitignoreRuleKey>,
}

impl GitignoreRuleCache {
    /// Creates an empty cache for tests and special cases that want to control
    /// refresh timing explicitly.
    pub fn empty_for_root(root_path: impl Into<PathBuf>) -> Self {
        Self {
            root_path: root_path.into(),
            entries: HashMap::new(),
            root_scoped_ignore_files: Vec::new(),
        }
    }

    /// Creates a cache for generic repository file tree behavior.
    ///
    /// This policy loads global gitignore rules and repository `.gitignore`
    /// files, but intentionally skips codebase-index-specific ignore files.
    pub fn for_repo_tree(root_path: &Path) -> Self {
        let mut cache = Self::empty_for_root(root_path);
        cache.refresh_global();
        cache.refresh_gitignore_for_directory(root_path);
        cache
    }

    /// Creates a cache for codebase indexing behavior.
    ///
    /// This policy includes generic repository rules plus root-scoped indexing
    /// ignore files such as `.warpindexingignore` and `.cursorignore`.
    pub fn for_codebase_index(root_path: &Path) -> Self {
        let mut cache = Self::for_repo_tree(root_path);
        cache.root_scoped_ignore_files = CODEBASE_INDEX_IGNORE_FILES
            .iter()
            .map(|file_name| root_path.join(file_name))
            .collect();
        cache.refresh_root_scoped_ignore_files();
        cache
    }

    pub fn matcher_count(&self) -> usize {
        self.entries.len()
    }

    pub fn refresh_for_path(&mut self, path: &Path) {
        self.refresh_ancestor_gitignores(path);
        self.refresh_root_scoped_ignore_files();
    }

    pub fn scoped_traversal_for_path(&mut self, path: &Path) -> ScopedGitignoreTraversal<'_> {
        let active_keys = self.ordered_active_keys_for_path(path);
        ScopedGitignoreTraversal {
            cache: self,
            active_keys,
        }
    }

    pub fn refreshed_traversal_for_path(&mut self, path: &Path) -> ScopedGitignoreTraversal<'_> {
        self.refresh_for_path(path);
        self.scoped_traversal_for_path(path)
    }

    pub fn is_ignored(&self, path: &Path, is_dir: bool, check_ancestors: bool) -> bool {
        self.evaluate_keys(
            self.ordered_active_keys_for_path(path).iter(),
            path,
            is_dir,
            check_ancestors,
        )
    }

    pub fn is_ignored_with_refresh(
        &mut self,
        path: &Path,
        is_dir: bool,
        check_ancestors: bool,
    ) -> bool {
        self.refresh_for_path(path);
        self.is_ignored(path, is_dir, check_ancestors)
    }

    fn refresh_global(&mut self) {
        let (gitignore, _) = GitignoreBuilder::new(&self.root_path).build_global();
        if !gitignore.is_empty() {
            self.entries.insert(
                GitignoreRuleKey::Global,
                GitignoreRule {
                    matcher: gitignore,
                    metadata: None,
                },
            );
        }
    }

    fn refresh_ancestor_gitignores(&mut self, path: &Path) {
        for ancestor in self.ancestor_directories_for_path(path) {
            self.refresh_gitignore_for_directory(&ancestor);
        }
    }

    fn refresh_root_scoped_ignore_files(&mut self) {
        for ignore_file_path in self.root_scoped_ignore_files.clone() {
            self.refresh_ignore_file(ignore_file_path);
        }
    }

    fn refresh_gitignore_for_directory(&mut self, directory_path: &Path) {
        self.refresh_ignore_file(directory_path.join(".gitignore"));
    }

    fn refresh_ignore_file(&mut self, ignore_file_path: PathBuf) {
        let key = GitignoreRuleKey::File(ignore_file_path.clone());
        let metadata = gitignore_file_metadata(&ignore_file_path);

        let Some(metadata) = metadata else {
            self.entries.remove(&key);
            return;
        };

        if self
            .entries
            .get(&key)
            .is_some_and(|entry| entry.metadata == Some(metadata))
        {
            return;
        }

        let (matcher, _) = Gitignore::new(&ignore_file_path);
        self.entries.insert(
            key,
            GitignoreRule {
                matcher,
                metadata: Some(metadata),
            },
        );
    }

    fn ordered_active_keys_for_path(&self, path: &Path) -> Vec<GitignoreRuleKey> {
        let mut active_keys = Vec::new();
        if self.entries.contains_key(&GitignoreRuleKey::Global) {
            active_keys.push(GitignoreRuleKey::Global);
        }

        for ancestor in self.ancestor_directories_for_path(path) {
            let key = GitignoreRuleKey::File(ancestor.join(".gitignore"));
            if self.entries.contains_key(&key) {
                active_keys.push(key);
            }
        }

        // Codebase-index-specific ignore files are evaluated after all `.gitignore`
        // files. That gives `.warpindexingignore`, `.cursorignore`, and similar
        // root-scoped indexing policies the final say for codebase indexing without
        // changing generic repository tree behavior.
        for ignore_file_path in &self.root_scoped_ignore_files {
            let key = GitignoreRuleKey::File(ignore_file_path.clone());
            if self.entries.contains_key(&key) && !active_keys.contains(&key) {
                active_keys.push(key);
            }
        }

        active_keys
    }

    fn ancestor_directories_for_path(&self, path: &Path) -> Vec<PathBuf> {
        let directory_path = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        };

        if !directory_path.starts_with(&self.root_path) {
            return Vec::new();
        }

        let mut ancestors = directory_path
            .ancestors()
            .take_while(|ancestor| ancestor.starts_with(&self.root_path))
            .map(Path::to_path_buf)
            .collect::<Vec<_>>();
        ancestors.reverse();
        ancestors
    }

    fn evaluate_keys<'a>(
        &self,
        keys: impl IntoIterator<Item = &'a GitignoreRuleKey>,
        path: &Path,
        is_dir: bool,
        check_ancestors: bool,
    ) -> bool
    where
        GitignoreRuleKey: 'a,
    {
        let mut ignored = false;
        for key in keys {
            let Some(entry) = self.entries.get(key) else {
                continue;
            };
            match gitignore_match_path(&entry.matcher, path, is_dir, check_ancestors) {
                Some(GitignoreMatch::Ignore) => ignored = true,
                Some(GitignoreMatch::Whitelist) => ignored = false,
                None => {}
            }
        }
        ignored
    }
}

impl ScopedGitignoreTraversal<'_> {
    pub fn enter_directory(&mut self, path: &Path) -> usize {
        let active_len = self.active_keys.len();
        self.cache.refresh_gitignore_for_directory(path);

        let key = GitignoreRuleKey::File(path.join(".gitignore"));
        if self.cache.entries.contains_key(&key) && !self.active_keys.contains(&key) {
            self.active_keys.push(key);
        }

        active_len
    }

    pub fn truncate_active(&mut self, active_len: usize) {
        self.active_keys.truncate(active_len);
    }

    pub fn matches(&self, path: &Path, is_dir: bool, check_ancestors: bool) -> bool {
        self.cache
            .evaluate_keys(self.active_keys.iter(), path, is_dir, check_ancestors)
    }
}

fn gitignore_file_metadata(path: &Path) -> Option<GitignoreFileMetadata> {
    let metadata = std::fs::metadata(path).ok()?;
    let content = std::fs::read(path).ok()?;
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    Some(GitignoreFileMetadata {
        modified: metadata.modified().ok(),
        len: metadata.len(),
        content_hash: hasher.finish(),
    })
}

pub(crate) fn gitignore_matches_path(
    gitignore: &Gitignore,
    path: &Path,
    is_dir: bool,
    check_ancestors: bool,
) -> bool {
    gitignore_match_path(gitignore, path, is_dir, check_ancestors) == Some(GitignoreMatch::Ignore)
}

fn gitignore_match_path(
    gitignore: &Gitignore,
    path: &Path,
    is_dir: bool,
    check_ancestors: bool,
) -> Option<GitignoreMatch> {
    if let Ok(relative_path) = path.strip_prefix(gitignore.path()) {
        // `matched_path_or_any_parents` panics if the path has a root.
        // If not on windows, we allow paths with a root if the gitignore path is empty (since this denotes a global gitignore).
        if relative_path.has_root() && (cfg!(windows) || gitignore.path() != Path::new("")) {
            return None;
        }

        let match_result = if check_ancestors {
            gitignore.matched_path_or_any_parents(relative_path, is_dir)
        } else {
            gitignore.matched(relative_path, is_dir)
        };

        match match_result {
            Match::Ignore(_) => Some(GitignoreMatch::Ignore),
            Match::Whitelist(_) => Some(GitignoreMatch::Whitelist),
            Match::None => None,
        }
    } else {
        None
    }
}

#[cfg(test)]
#[path = "scoped_gitignore_tests.rs"]
mod tests;
