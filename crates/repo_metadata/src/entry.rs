#![cfg_attr(not(feature = "local_fs"), allow(dead_code))]

use std::collections::VecDeque;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(feature = "local_fs")]
use std::sync::Arc;

use ignore::gitignore::Gitignore;
#[cfg(feature = "local_fs")]
use notify_debouncer_full::notify::WatchFilter;
use thiserror::Error;
use warp_util::standardized_path::StandardizedPath;

use crate::standing_queries::{StandingQueryDefinitions, StandingQueryResults};

/// Maximum file size allowed for treesitter parsing (3MB).
const MAX_FILE_SIZE: usize = 3 * 1000 * 1000;

/// Maximum number of files to load when lazy-loading a directory
pub const LAZY_LOAD_FILE_LIMIT: usize = 5000;

#[derive(Debug, Error)]
pub enum BuildTreeError {
    #[error("Repo size exceeded max file limit")]
    ExceededMaxFileLimit,
    #[error("File is ignored")]
    Ignored,
    #[error("IO error reading path.")]
    IOError(#[from] io::Error),
    #[error("Symlink is not supported")]
    Symlink,
    #[error("Maximum directory depth exceeded")]
    MaxDepthExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IgnoredPathStrategy {
    /// Do not include any ignored files or folders
    Exclude,

    /// Lazy-load excluded directories
    IncludeLazy,

    /// Exclude all ignored files except for the ones in the given list
    IncludeOnly(Vec<String>),

    /// Add all of the ignored files into the tree
    Include,
}

/// What the tree builder does when the per-build file budget is exhausted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetExceededBehavior {
    /// Stop descending and leave the remaining directories as unloaded
    /// placeholders (lazy-loaded on demand). The build still succeeds with a
    /// partial, breadth-first tree. This is the default for the shared file
    /// tree, `@`-context, and skill discovery.
    StopAndLazyLoad,
    /// Abort the build and return [`BuildTreeError::ExceededMaxFileLimit`].
    /// Use this for consumers that must not operate on a partial tree — e.g.
    /// codebase embedding, where the file limit is an intentional cost cap.
    FailFast,
}

/// Filesystem entry.
#[derive(Debug, Clone)]
pub enum Entry {
    File(FileMetadata),
    Directory(DirectoryEntry),
}
#[derive(Clone, Copy)]
pub(crate) struct BuildTreeOptions<'a> {
    pub max_depth: usize,
    pub current_depth: usize,
    pub ignored_path_strategy: &'a IgnoredPathStrategy,
    pub force_included_paths: &'a [PathBuf],
    pub budget_exceeded_behavior: BudgetExceededBehavior,
}
struct StandingQueryBuildState<'a> {
    results: &'a mut StandingQueryResults,
    definitions: &'a StandingQueryDefinitions,
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
pub struct FileId(usize);

impl FileId {
    /// Constructs a new globally-unique file ID.
    #[allow(clippy::new_without_default)]
    pub(crate) fn new() -> FileId {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let raw = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        FileId(raw)
    }
}

impl Entry {
    pub fn path(&self) -> &StandardizedPath {
        match self {
            Self::File(file) => &file.path,
            Self::Directory(directory) => &directory.path,
        }
    }

    pub fn loaded(&self) -> bool {
        match self {
            Self::File(_) => true,
            Self::Directory(directory) => directory.loaded,
        }
    }

    pub fn ignored(&self) -> bool {
        match self {
            Self::File(file) => file.ignored,
            Self::Directory(directory) => directory.ignored,
        }
    }

    /// Builds a tree of entries from a given path, handling gitignored files and directories.
    /// After max_depth is reached, children outside force-included paths are lazy-loaded to
    /// prevent deeply nested trees.
    /// IgnoredPathStrategy determines what happens when ignored files are encountered.
    /// `budget_exceeded_behavior` controls what happens once the file budget is
    /// exhausted (see [`BudgetExceededBehavior`]).
    #[allow(clippy::too_many_arguments)]
    pub fn build_tree(
        path: impl Into<PathBuf>,
        files: &mut Vec<FileMetadata>,
        gitignores: &mut Vec<Gitignore>,
        remaining_file_quota: Option<&mut usize>,
        max_depth: usize,
        current_depth: usize,
        ignored_path_strategy: &IgnoredPathStrategy,
        budget_exceeded_behavior: BudgetExceededBehavior,
    ) -> Result<Self, BuildTreeError> {
        Self::build_tree_with_force_included_paths_and_ancestor(
            path,
            files,
            gitignores,
            remaining_file_quota,
            BuildTreeOptions {
                max_depth,
                current_depth,
                ignored_path_strategy,
                force_included_paths: &[],
                budget_exceeded_behavior,
            },
            false,
            None,
        )
    }
    /// Builds the materialized tree and standing results during the same filesystem traversal.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_tree_with_standing_queries(
        path: impl Into<PathBuf>,
        files: &mut Vec<FileMetadata>,
        gitignores: &mut Vec<Gitignore>,
        remaining_file_quota: Option<&mut usize>,
        options: BuildTreeOptions<'_>,
        ancestor_is_ignored: bool,
        standing_results: &mut StandingQueryResults,
        definitions: &StandingQueryDefinitions,
    ) -> Result<Self, BuildTreeError> {
        let mut standing_queries = StandingQueryBuildState {
            results: standing_results,
            definitions,
        };
        Self::build_tree_with_force_included_paths_and_ancestor(
            path,
            files,
            gitignores,
            remaining_file_quota,
            options,
            ancestor_is_ignored,
            Some(&mut standing_queries),
        )
    }

    /// Builds a tree of entries from a given path, eagerly loading any path that
    /// matches one of the supplied force-included paths instead of leaving it
    /// lazy (see [`BuildTreeOptions::force_included_paths`]).
    #[cfg(test)]
    pub(crate) fn build_tree_with_force_included_paths(
        path: impl Into<PathBuf>,
        files: &mut Vec<FileMetadata>,
        gitignores: &mut Vec<Gitignore>,
        remaining_file_quota: Option<&mut usize>,
        options: BuildTreeOptions<'_>,
    ) -> Result<Self, BuildTreeError> {
        Self::build_tree_with_force_included_paths_and_ancestor(
            path,
            files,
            gitignores,
            remaining_file_quota,
            options,
            false,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_tree_with_ignored_ancestor(
        path: impl Into<PathBuf>,
        files: &mut Vec<FileMetadata>,
        gitignores: &mut Vec<Gitignore>,
        remaining_file_quota: Option<&mut usize>,
        max_depth: usize,
        current_depth: usize,
        ignored_path_strategy: &IgnoredPathStrategy,
        ancestor_is_ignored: bool,
    ) -> Result<Self, BuildTreeError> {
        Self::build_tree_with_force_included_paths_and_ancestor(
            path,
            files,
            gitignores,
            remaining_file_quota,
            BuildTreeOptions {
                max_depth,
                current_depth,
                ignored_path_strategy,
                force_included_paths: &[],
                budget_exceeded_behavior: BudgetExceededBehavior::StopAndLazyLoad,
            },
            ancestor_is_ignored,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build_tree_with_force_included_paths_and_ancestor(
        path: impl Into<PathBuf>,
        files: &mut Vec<FileMetadata>,
        gitignores: &mut Vec<Gitignore>,
        remaining_file_quota: Option<&mut usize>,
        options: BuildTreeOptions<'_>,
        ancestor_is_ignored: bool,
        mut standing_queries: Option<&mut StandingQueryBuildState<'_>>,
    ) -> Result<Self, BuildTreeError> {
        let root_path: PathBuf = path.into();

        // Local copy of the file budget. The builder spends it breadth-first;
        // once it is exhausted, any remaining directories are left as unloaded
        // placeholders (lazy-loaded on demand) instead of aborting the whole
        // build. This keeps coverage even and shallow-biased rather than
        // collapsing the entire tree to a single level.
        let mut quota: Option<usize> = remaining_file_quota.as_deref().copied();

        // Arena of partially-built nodes. A child is always discovered (and
        // pushed) while expanding its parent, so a child's index is always
        // greater than its parent's and the nested tree can be assembled
        // bottom-up at the end.
        let mut nodes: Vec<Option<NodeBuilder>> = Vec::new();

        // Classify the root. Unlike child entries (which are simply omitted when
        // ignored/symlinked), a classification failure at the root propagates to
        // the caller, preserving existing error behavior.
        if let Some(state) = standing_queries.as_deref_mut() {
            state
                .results
                .record_path(&root_path, root_path.is_dir(), state.definitions);
        }
        match evaluate_entry(
            &root_path,
            gitignores,
            &options,
            options.current_depth,
            ancestor_is_ignored,
        )? {
            EvaluatedEntry::File { ignored } => {
                if quota == Some(0)
                    && options.budget_exceeded_behavior == BudgetExceededBehavior::FailFast
                {
                    return Err(BuildTreeError::ExceededMaxFileLimit);
                }
                let metadata = consume_file(&root_path, ignored, files, &mut quota);
                write_back_quota(remaining_file_quota, quota);
                Ok(Self::File(metadata))
            }
            EvaluatedEntry::Directory { ignored, lazy } => {
                nodes.push(Some(NodeBuilder::Dir {
                    path: root_path.clone(),
                    ignored,
                    loaded: false,
                    children: Vec::new(),
                }));

                let mut queue: VecDeque<DirJob> = VecDeque::new();
                if !lazy {
                    queue.push_back(DirJob {
                        index: 0,
                        path: root_path,
                        depth: options.current_depth,
                        ignored,
                        is_root: true,
                    });
                }

                while let Some(job) = queue.pop_front() {
                    // Budget handling. With `StopAndLazyLoad` (the default), once
                    // the file quota is exhausted we stop expanding directories
                    // and leave them as unloaded placeholders; directories on the
                    // path to a force-included path (e.g. skill provider
                    // directories) are always expanded so discovery-critical
                    // files stay reachable. With `FailFast` we keep descending
                    // and abort below as soon as a file would exceed the budget.
                    let should_expand = match options.budget_exceeded_behavior {
                        BudgetExceededBehavior::FailFast => true,
                        BudgetExceededBehavior::StopAndLazyLoad => {
                            quota.is_none_or(|remaining| remaining > 0)
                                || matches_force_included_path(
                                    &job.path,
                                    options.force_included_paths,
                                )
                        }
                    };
                    if !should_expand {
                        continue;
                    }

                    let entries = match std::fs::read_dir(&job.path) {
                        Ok(entries) => entries,
                        Err(e) => {
                            // Preserve existing behavior: failing to read the
                            // root directory propagates, while an unreadable
                            // nested directory is left as an unloaded placeholder.
                            if job.is_root {
                                return Err(BuildTreeError::IOError(e));
                            }
                            continue;
                        }
                    };

                    if let Some(NodeBuilder::Dir { loaded, .. }) = nodes[job.index].as_mut() {
                        *loaded = true;
                    }

                    let child_depth = job.depth + 1;
                    for entry in entries {
                        let Ok(entry) = entry else {
                            continue;
                        };
                        let entry_path = entry.path();

                        // Do not materialize directory symlinks in the canonical tree. Standing
                        // project-skill queries still follow eligible provider children locally
                        // and retain their lexical paths in the result set.
                        let canonical_path = if entry_path.is_symlink() {
                            if entry_path.is_dir() {
                                if let Some(state) = standing_queries.as_deref_mut() {
                                    state.results.record_followed_project_skill_directory(
                                        &entry_path,
                                        state.definitions,
                                    );
                                }
                                None
                            } else {
                                Some(entry_path)
                            }
                        } else {
                            dunce::canonicalize(entry_path).ok()
                        };
                        let Some(child_path) = canonical_path else {
                            continue;
                        };
                        if let Some(state) = standing_queries.as_deref_mut() {
                            state.results.record_path(
                                &child_path,
                                child_path.is_dir(),
                                state.definitions,
                            );
                        }

                        match evaluate_entry(
                            &child_path,
                            gitignores,
                            &options,
                            child_depth,
                            job.ignored,
                        ) {
                            Ok(EvaluatedEntry::File { ignored }) => {
                                if quota == Some(0)
                                    && options.budget_exceeded_behavior
                                        == BudgetExceededBehavior::FailFast
                                {
                                    return Err(BuildTreeError::ExceededMaxFileLimit);
                                }
                                let metadata =
                                    consume_file(&child_path, ignored, files, &mut quota);
                                let child_index = nodes.len();
                                nodes.push(Some(NodeBuilder::File(metadata)));
                                push_child(&mut nodes, job.index, child_index);
                            }
                            Ok(EvaluatedEntry::Directory { ignored, lazy }) => {
                                let child_index = nodes.len();
                                nodes.push(Some(NodeBuilder::Dir {
                                    path: child_path.clone(),
                                    ignored,
                                    loaded: false,
                                    children: Vec::new(),
                                }));
                                push_child(&mut nodes, job.index, child_index);
                                // Lazy directories (past max depth, or ignored
                                // without a matching force-included path) stay
                                // unloaded. Everything else is queued for
                                // expansion, subject to the budget gate above.
                                if !lazy {
                                    queue.push_back(DirJob {
                                        index: child_index,
                                        path: child_path,
                                        depth: child_depth,
                                        ignored,
                                        is_root: false,
                                    });
                                }
                            }
                            Err(_) => {
                                // Ignored / excluded / symlinked-directory entries
                                // are omitted from the tree.
                            }
                        }
                    }
                }

                write_back_quota(remaining_file_quota, quota);
                Ok(assemble_node(&mut nodes, 0))
            }
        }
    }

    /// Finds an entry based on path
    pub fn find_mut(&mut self, path: &Path) -> Option<&mut Entry> {
        let std_path = StandardizedPath::try_from_local(path).ok()?;
        self.find_mut_by_std_path(&std_path)
    }

    fn find_mut_by_std_path(&mut self, path: &StandardizedPath) -> Option<&mut Entry> {
        if self.path() == path {
            return Some(self);
        }

        if let Self::Directory(directory) = self {
            if !path.starts_with(&directory.path) {
                // Target is not descendant of directory.
                return None;
            }

            for child in directory.children.iter_mut() {
                if let Some(entry) = child.find_mut_by_std_path(path) {
                    return Some(entry);
                }
            }
        }

        None
    }

    /// Loads an unloaded directory
    pub fn load(&mut self, gitignores: &mut Vec<Gitignore>) -> Result<(), BuildTreeError> {
        // TODO: Consider a similar `unload` method if we run into performance issues.
        let Self::Directory(directory) = self else {
            return Ok(());
        };

        let mut remaining_file_quota = LAZY_LOAD_FILE_LIMIT;
        let mut files = Vec::new();
        let ancestor_is_ignored = directory.ignored;

        let result = Entry::build_tree_with_ignored_ancestor(
            directory.path.to_local_path_lossy(),
            &mut files,
            gitignores,
            Some(&mut remaining_file_quota),
            1, /* max_depth */
            0, /* current_depth */
            &IgnoredPathStrategy::Include,
            ancestor_is_ignored,
        );

        result.map(|entry| match entry {
            Entry::Directory(entry) => {
                *directory = entry;
            }
            Entry::File(_) => {
                log::error!("Called load on a directory but a file entry was returned");
            }
        })
    }

    /// Removes the entry corresponding to the given target path, if any.
    pub fn remove(&mut self, target_path: &Path) -> Option<FileMetadata> {
        let std_path = StandardizedPath::try_from_local(target_path).ok()?;
        self.remove_by_std_path(&std_path)
    }

    fn remove_by_std_path(&mut self, target_path: &StandardizedPath) -> Option<FileMetadata> {
        let Self::Directory(directory) = self else {
            // We should never hit this condition - we only end up recursing into directories given
            // that recursion only occurs when `target_path` is a descendant of `directory.path`
            // but not a direct child.
            return None;
        };
        if !target_path.starts_with(&directory.path) {
            // Target is not descendant of directory.
            return None;
        }
        for (index, child) in directory.children.iter_mut().enumerate() {
            if child.path() == target_path {
                // If the child's path is the target path, remove the child.
                return match directory.children.remove(index) {
                    Entry::Directory(_) => None,
                    Entry::File(metadata) => Some(metadata),
                };
            } else if target_path.starts_with(child.path()) {
                // Child is a descendant of the target path, so recurse.
                return child.remove_by_std_path(target_path);
            }
        }

        log::debug!("target path not found under the current directory node");
        None
    }
}

/// A node in the breadth-first build arena. Directory children are referenced by
/// arena index so the nested [`Entry`] tree can be assembled bottom-up.
enum NodeBuilder {
    File(FileMetadata),
    Dir {
        path: PathBuf,
        ignored: bool,
        loaded: bool,
        children: Vec<usize>,
    },
}

/// A directory queued for expansion during the breadth-first build.
struct DirJob {
    index: usize,
    path: PathBuf,
    depth: usize,
    ignored: bool,
    is_root: bool,
}

/// Classification of a single filesystem entry.
enum EvaluatedEntry {
    File { ignored: bool },
    Directory { ignored: bool, lazy: bool },
}

/// Classifies a single path: rejects directory symlinks, loads any local
/// `.gitignore`, computes gitignore status, and applies the ignored-path
/// strategy. Returns `Err(Ignored)`/`Err(Symlink)` for entries that should be
/// omitted; callers decide whether that is fatal (root) or a skip (child).
fn evaluate_entry(
    curr_path: &Path,
    gitignores: &mut Vec<Gitignore>,
    options: &BuildTreeOptions<'_>,
    current_depth: usize,
    ancestor_is_ignored: bool,
) -> Result<EvaluatedEntry, BuildTreeError> {
    let is_dir = curr_path.is_dir();

    // Only ignore symlinks to directories. Symlinks to files are preserved (e.g. WARP.md).
    if curr_path.is_symlink() && is_dir {
        return Err(BuildTreeError::Symlink);
    }

    let gitignore_path = curr_path.join(".gitignore");
    if gitignore_path.exists() {
        let (gitignore, _) = Gitignore::new(gitignore_path);
        gitignores.push(gitignore);
    }

    let path_is_ignored = ancestor_is_ignored
        || is_git_internal_path(curr_path)
        || matches_gitignores(
            curr_path,
            is_dir,
            &*gitignores,
            false, /* check_ancestors */
        );

    let force_included = matches_force_included_path(curr_path, options.force_included_paths);

    // If we've reached the max depth, force lazy-loading even of non-ignored folders unless the
    // folder is on the path to a force-included subtree.
    let mut lazy = current_depth >= options.max_depth && !force_included;

    if path_is_ignored {
        match options.ignored_path_strategy {
            IgnoredPathStrategy::Exclude => return Err(BuildTreeError::Ignored),
            IgnoredPathStrategy::IncludeOnly(patterns) => {
                if let Some(file_name) = curr_path.file_name().and_then(|n| n.to_str()) {
                    if !patterns.iter().any(|pattern| file_name == pattern) {
                        return Err(BuildTreeError::Ignored);
                    }
                }
            }
            IgnoredPathStrategy::IncludeLazy => {
                lazy = !force_included;
            }
            IgnoredPathStrategy::Include => {}
        }
    }

    if is_dir {
        Ok(EvaluatedEntry::Directory {
            ignored: path_is_ignored,
            lazy,
        })
    } else if curr_path.is_file() {
        Ok(EvaluatedEntry::File {
            ignored: path_is_ignored,
        })
    } else {
        Err(BuildTreeError::Symlink)
    }
}

/// Records a file: decrements the budget (saturating), constructs metadata, and
/// appends it to the flat `files` list.
fn consume_file(
    path: &Path,
    ignored: bool,
    files: &mut Vec<FileMetadata>,
    quota: &mut Option<usize>,
) -> FileMetadata {
    if let Some(remaining) = quota.as_mut() {
        *remaining = remaining.saturating_sub(1);
    }
    let metadata = FileMetadata::new(path.to_path_buf(), ignored);
    files.push(metadata.clone());
    metadata
}

/// Appends `child` to `parent`'s child list in the build arena.
fn push_child(nodes: &mut [Option<NodeBuilder>], parent: usize, child: usize) {
    if let Some(NodeBuilder::Dir { children, .. }) = nodes[parent].as_mut() {
        children.push(child);
    }
}

/// Recursively assembles the nested [`Entry`] tree from the build arena.
/// Recursion depth is normally bounded by `BuildTreeOptions::max_depth`, except for force-included
/// subtrees.
fn assemble_node(nodes: &mut [Option<NodeBuilder>], index: usize) -> Entry {
    match nodes[index]
        .take()
        .expect("each arena node is assembled exactly once")
    {
        NodeBuilder::File(metadata) => Entry::File(metadata),
        NodeBuilder::Dir {
            path,
            ignored,
            loaded,
            children,
        } => {
            let children = children
                .into_iter()
                .map(|child| assemble_node(nodes, child))
                .collect();
            Entry::Directory(DirectoryEntry {
                path: StandardizedPath::from_local_absolute_unchecked(&path),
                children,
                ignored,
                loaded,
            })
        }
    }
}

/// Writes the remaining budget back into the caller-provided slot, if any.
fn write_back_quota(remaining_file_quota: Option<&mut usize>, quota: Option<usize>) {
    if let (Some(slot), Some(value)) = (remaining_file_quota, quota) {
        *slot = value;
    }
}

pub fn is_git_internal_path(path: &Path) -> bool {
    path.components().any(|component| {
        if let Component::Normal(name) = component {
            name == ".git"
        } else {
            false
        }
    })
}

/// Returns `true` when `path` is, contains, or lies on the way to one of the
/// `force_included_paths`. Each force-included path is a relative component
/// sequence (e.g. `.agents/skills`) matched against the tail of `path`, so a
/// match also holds for the ancestor prefixes leading to it.
pub(crate) fn matches_force_included_path(path: &Path, force_included_paths: &[PathBuf]) -> bool {
    let path_components: Vec<_> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name),
            Component::Prefix(_)
            | Component::RootDir
            | Component::CurDir
            | Component::ParentDir => None,
        })
        .collect();

    force_included_paths.iter().any(|force_included| {
        let force_included_components: Vec<_> = force_included
            .components()
            .filter_map(|component| match component {
                Component::Normal(name) => Some(name),
                Component::Prefix(_)
                | Component::RootDir
                | Component::CurDir
                | Component::ParentDir => None,
            })
            .collect();

        if force_included_components.is_empty() {
            return false;
        }

        if path_components
            .windows(force_included_components.len())
            .any(|window| window == force_included_components.as_slice())
        {
            return true;
        }

        (1..force_included_components.len()).any(|prefix_len| {
            path_components.len() >= prefix_len
                && path_components[path_components.len() - prefix_len..]
                    == force_included_components[..prefix_len]
        })
    })
}
/// Returns true if a path matches any of the gitignores.
///
/// For example, if the directory `/target` is ignored:
/// - If `check_ancestors` is true, then `/target/debug` will match.
/// - If `check_ancestors` is false, then `/target/debug` will not match.
pub fn matches_gitignores(
    path: &Path,
    is_dir: bool,
    gitignores: &[Gitignore],
    check_ancestors: bool,
) -> bool {
    gitignores.iter().any(|gitignore| {
        if let Ok(relative_path) = path.strip_prefix(gitignore.path()) {
            // `matched_path_or_any_parents` panics if the path has a root.
            // If not on windows, we allow paths with a root if the gitignore path is empty (since this denotes a global gitignore).
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
        } else {
            false
        }
    })
}

/// Returns the path components after `.git` in a git-internal path,
/// skipping the worktree indirection (`.git/worktrees/<name>/…`) if present.
/// Returns `None` if the path has no `.git` component or nothing follows it.
fn git_suffix_components(path: &Path) -> Option<Vec<Component<'_>>> {
    let components: Vec<_> = path.components().collect();
    let git_index = components.iter().position(|c| c.as_os_str() == ".git")?;

    let after_git = &components[git_index + 1..];
    if after_git.is_empty() {
        return None;
    }

    // For worktrees the layout is `.git/worktrees/<name>/…`.
    // Skip the `worktrees/<name>` prefix so callers see the same
    // logical structure as a normal repo.
    if after_git.first().map(|c| c.as_os_str()) == Some(std::ffi::OsStr::new("worktrees"))
        && after_git.len() >= 3
    {
        // after_git[0] = "worktrees", [1] = <name>, [2..] = actual content
        return Some(after_git[2..].to_vec());
    }

    Some(after_git.to_vec())
}

/// Given a path like `.../repo/.git/worktrees/foo/HEAD`, returns
/// `.../repo/.git/worktrees/foo`. Returns `None` for non-worktree paths.
pub(crate) fn extract_worktree_git_dir(path: &Path) -> Option<PathBuf> {
    let components: Vec<_> = path.components().collect();
    let git_index = components.iter().position(|c| c.as_os_str() == ".git")?;
    let after_git = &components[git_index + 1..];
    if after_git.len() >= 3
        && after_git
            .first()
            .map(|c| c.as_os_str() == "worktrees")
            .unwrap_or(false)
    {
        // Rebuild: everything up to and including .git/worktrees/<name>
        Some(components[..git_index + 3].iter().collect())
    } else {
        None
    }
}

/// Returns `true` for shared ref paths that live directly in the common
/// `.git` directory and should be broadcast to all repos sharing it.
/// Currently this means `.git/refs/heads/*` (not under `.git/worktrees/`).
pub(crate) fn is_shared_git_ref(path: &Path) -> bool {
    if extract_worktree_git_dir(path).is_some() {
        return false;
    }
    let components: Vec<_> = path.components().collect();
    let Some(git_index) = components.iter().position(|c| c.as_os_str() == ".git") else {
        return false;
    };
    let after_git = &components[git_index + 1..];
    after_git
        .first()
        .map(|c| c.as_os_str() == "refs")
        .unwrap_or(false)
        && after_git
            .get(1)
            .map(|c| c.as_os_str() == "heads")
            .unwrap_or(false)
}

/// Returns `true` for loose remote-tracking refs under the shared `.git`
/// directory, e.g. `.git/refs/remotes/origin/main`.
pub(crate) fn is_remote_tracking_ref(path: &Path) -> bool {
    if extract_worktree_git_dir(path).is_some() {
        return false;
    }
    let components: Vec<_> = path.components().collect();
    let Some(git_index) = components.iter().position(|c| c.as_os_str() == ".git") else {
        return false;
    };
    let after_git = &components[git_index + 1..];
    after_git.len() >= 4
        && after_git[0].as_os_str() == "refs"
        && after_git[1].as_os_str() == "remotes"
}

/// Returns true for Git files that can change the current branch's tracked
/// upstream ref.
pub(crate) fn is_tracking_state_git_file(path: &Path) -> bool {
    let Some(suffix) = git_suffix_components(path) else {
        return false;
    };
    suffix.len() == 1
        && matches!(
            suffix[0].as_os_str().to_str(),
            Some("HEAD" | "config" | "config.worktree")
        )
}

/// Returns true for `.git/config` in the shared Git directory.
pub(crate) fn is_common_git_config(path: &Path) -> bool {
    if extract_worktree_git_dir(path).is_some() {
        return false;
    }
    let components: Vec<_> = path.components().collect();
    let Some(git_index) = components.iter().position(|c| c.as_os_str() == ".git") else {
        return false;
    };
    let after_git = &components[git_index + 1..];
    after_git.len() == 1 && after_git[0].as_os_str() == "config"
}

/// Returns true for `.git/HEAD` and `.git/refs/heads/*`
/// (and their worktree equivalents `.git/worktrees/*/HEAD`, etc.).
pub(crate) fn is_commit_related_git_file(path: &Path) -> bool {
    let Some(suffix) = git_suffix_components(path) else {
        return false;
    };
    match suffix.first().map(|c| c.as_os_str()) {
        Some(name) if name == "HEAD" => true,
        Some(name) if name == "refs" => {
            suffix.get(1).map(|c| c.as_os_str()) == Some(std::ffi::OsStr::new("heads"))
        }
        _ => false,
    }
}

/// Returns true for `.git/index.lock`
/// (and its worktree equivalent `.git/worktrees/*/index.lock`).
pub(crate) fn is_index_lock_file(path: &Path) -> bool {
    let Some(suffix) = git_suffix_components(path) else {
        return false;
    };
    suffix.len() == 1 && suffix[0].as_os_str() == "index.lock"
}

/// Determines if a git-related path should be ignored by the filesystem watcher.
///
/// Uses an allowlist approach: only commit-related files (HEAD, refs/heads/*),
/// loose remote-tracking refs, tracked-upstream state files, and the index lock
/// file are allowed through. Everything else inside `.git/` is ignored.
pub fn should_ignore_git_path(path: &Path) -> bool {
    if !is_git_internal_path(path) {
        return false; // Not a git path, don't ignore
    }
    // Ignore everything inside .git/ except the allowlisted patterns.
    !is_commit_related_git_file(path)
        && !is_index_lock_file(path)
        && !is_remote_tracking_ref(path)
        && !is_tracking_state_git_file(path)
}

/// Returns `true` when the directory at `path` should be registered for watching.
/// Specifically for prefixes that lead to an allowlisted file and `false` for everything else inside `.git/`.
pub fn should_watch_directory_in_git_path(path: &Path) -> bool {
    if !is_git_internal_path(path) {
        return true;
    }

    // Worktree paths: `.git/worktrees/<name>/...` only descends along the
    // path needed to reach the allowlisted children (HEAD, index.lock,
    // config.worktree, refs/heads/*, refs/remotes/<r>/*).
    if let Some(worktree_dir) = extract_worktree_git_dir(path) {
        // `path` is either the worktree gitdir itself or something under it.
        // Anything up to and including `.git/worktrees/<name>` must
        // be descended into so we can reach children.
        if path == worktree_dir || worktree_dir.starts_with(path) {
            return true;
        }
        // Inside `.git/worktrees/<name>/...`. Apply the same allowlist logic as for the shared `.git/`.
        let Some(suffix) = git_suffix_components(path) else {
            return false;
        };
        return descend_allowlist_matches(&suffix);
    }

    // Common `.git/` directory: allow descending along the path to
    // `.git/`, `.git/refs/heads/`, `.git/refs/remotes/<remote>/`, and
    // `.git/worktrees/<name>/`.
    let Some(suffix) = git_suffix_components(path) else {
        // Path is `.git/` itself — needed so we can reach allowlisted children.
        return true;
    };
    descend_allowlist_matches(&suffix)
}

/// Returns `true` for an in-`.git/` directory suffix that lies on the way to an allowlisted file.
/// `suffix` is the component sequence after the `.git` component (worktree indirection already stripped),
/// so e.g. `.git/worktrees/<name>/refs/heads` is seen here as just `["refs", "heads"]`.
///
/// Only the first two components are inspected:
/// - `top_level_dir` is the directory immediately under `.git/` (e.g. `refs`, `objects`, `worktrees`)
///   and decides which subtree we're descending into.
/// - `refs_subdir` is meaningful only when `top_level_dir == "refs"`, where it distinguishes
///   the watched ref subtrees (`heads`, `remotes`) from pruned ones (`tags`, etc.).
fn descend_allowlist_matches(suffix: &[Component<'_>]) -> bool {
    let top_level_dir = suffix.first().and_then(|c| c.as_os_str().to_str());
    let refs_subdir = suffix.get(1).and_then(|c| c.as_os_str().to_str());
    match top_level_dir {
        // `.git/refs`, `.git/refs/heads[/...]`, `.git/refs/remotes[/<r>[/...]]`.
        // `.git/refs/tags/*` and other refs subtrees stay pruned.
        Some("refs") => matches!(refs_subdir, None | Some("heads") | Some("remotes")),
        // Worktree dispatcher — needed to reach `.git/worktrees/<name>/...`.
        Some("worktrees") => true,
        // All other `.git/` subdirectories (objects, hooks, logs, info, lfs, …) are pruned.
        Some(_) => false,
        // `.git/` itself — descend so allowlisted children stay reachable.
        None => true,
    }
}

/// Returns whether a repository file watcher should descend into (and register
/// a watch on) the directory at `path`.
///
/// Directories inside `.git/` follow the watcher allowlist, force-included
/// paths are always watched even when gitignored, and any other gitignored
/// directory is pruned so we don't register watches on `node_modules`, build
/// output, vendored deps, etc.
pub fn should_watch_repo_directory(
    path: &Path,
    gitignores: &[Gitignore],
    force_included_paths: &[PathBuf],
) -> bool {
    if is_git_internal_path(path) {
        return should_watch_directory_in_git_path(path);
    }

    if matches_force_included_path(path, force_included_paths) {
        return true;
    }

    !matches_gitignores(
        path,
        path.is_dir(),
        gitignores,
        /* check_ancestors */ true,
    )
}

/// Returns the [`WatchFilter`] used by repository file watchers.
///
/// Emit predicate: forwards events for everything outside `.git/` plus the
/// allowlisted files inside `.git/` (HEAD, refs/heads/*, index.lock,
/// config, config.worktree, refs/remotes/<r>/*, and worktree equivalents).
/// Gitignored files that live directly in a watched (non-ignored) directory
/// are still emitted here and tagged `is_ignored` downstream, preserving
/// existing behavior.
///
/// Descend predicate: see [`should_watch_repo_directory`]. In addition to the
/// `.git/` allowlist, it prunes gitignored directories (honoring registered
/// force-included paths) so the recursive walk does not register watches on
/// gitignored subtrees.
///
/// `gitignores` should be the repo's root + global gitignores (as produced by
/// [`gitignores_for_directory`]), matching `Repository::check_gitignore_status`
/// so descend decisions and the downstream `is_ignored` tagging stay
/// consistent. Nested per-directory `.gitignore` files are not consulted here
/// (same limitation as the existing tagging), which can only cause us to
/// over-watch, never to miss events.
#[cfg(feature = "local_fs")]
pub fn repo_watch_filter(
    gitignores: Vec<Gitignore>,
    force_included_paths: Vec<PathBuf>,
) -> WatchFilter {
    let should_watch =
        move |path: &Path| should_watch_repo_directory(path, &gitignores, &force_included_paths);
    WatchFilter::with_filter(
        Arc::new(should_watch),
        Arc::new(|path: &Path| !should_ignore_git_path(path)),
    )
}

/// Determines whether a file should be parsed by a treesitter query. For now the main criteria is it shouldn't
/// exceed the given file size limit.
pub fn is_file_parsable(path: &Path) -> Result<bool, io::Error> {
    std::fs::metadata(path).map(|metadata| (metadata.len() as usize) < MAX_FILE_SIZE)
}

pub fn gitignores_for_directory(directory_path: &Path) -> Vec<Gitignore> {
    let mut gitignores = Vec::new();
    let gitignore_path = directory_path.join(".gitignore");
    if gitignore_path.exists() {
        let (gitignore, _) = Gitignore::new(&gitignore_path);
        gitignores.push(gitignore);
    }
    let (global_gitignore, _) = Gitignore::global();
    if !global_gitignore.is_empty() {
        gitignores.push(global_gitignore);
    }
    gitignores
}

#[derive(Debug, Clone)]
pub struct FileMetadata {
    /// Absolute path to the file.
    pub path: StandardizedPath,
    pub file_id: FileId,
    pub extension: Option<String>,
    pub ignored: bool,
}

impl FileMetadata {
    pub fn new(path: PathBuf, ignored: bool) -> Self {
        let path_extension = path.extension().and_then(|extension| extension.to_str());
        let file_id = FileId::new();
        let std_path = StandardizedPath::from_local_absolute_unchecked(&path);
        Self {
            file_id,
            extension: path_extension.map(str::to_string),
            path: std_path,
            ignored,
        }
    }

    /// Construct from a [`StandardizedPath`] directly, without filesystem I/O.
    pub fn from_standardized(path: StandardizedPath, ignored: bool) -> Self {
        let file_id = FileId::new();
        let extension = path.extension().map(|s| s.to_owned());
        Self {
            file_id,
            extension,
            path,
            ignored,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DirectoryEntry {
    /// Absolute path to the directory.
    pub path: StandardizedPath,
    pub children: Vec<Entry>,
    pub ignored: bool,
    pub loaded: bool,
}

impl DirectoryEntry {
    pub fn find_or_insert_child(&mut self, target_path: &Path) -> Option<&mut Entry> {
        let std_path = StandardizedPath::try_from_local(target_path).ok()?;

        // First, try to find the child's position
        if let Some(index) = self
            .children
            .iter()
            .position(|child| *child.path() == std_path)
        {
            // Child exists, return a mutable reference to it
            return Some(&mut self.children[index]);
        }

        // Child not found, create new entry if the path is valid
        let new_entry = if target_path.is_dir() {
            Entry::Directory(DirectoryEntry {
                children: vec![],
                path: std_path,
                loaded: false,
                ignored: false,
            })
        } else if target_path.is_file() {
            Entry::File(FileMetadata {
                path: std_path.clone(),
                file_id: FileId::new(),
                extension: std_path.extension().map(|s| s.to_owned()),
                ignored: false,
            })
        } else {
            // Cannot insert child since target_path is neither a file or a directory.
            return None;
        };

        // Insert the new entry and return a mutable reference to it
        self.children.push(new_entry);
        self.children.last_mut()
    }

    /// Similar to find_or_insert_child but specifically for creating directory entries.
    /// This is used when we know the path should be a directory (e.g., when ensuring parent directories exist).
    pub fn find_or_insert_directory(&mut self, target_path: &Path) -> Option<&mut Entry> {
        let std_path = StandardizedPath::try_from_local(target_path).ok()?;

        // First, try to find the child's position
        if let Some(index) = self
            .children
            .iter()
            .position(|child| *child.path() == std_path)
        {
            // Child exists, return a mutable reference to it
            return Some(&mut self.children[index]);
        }

        // Child not found, create new directory entry
        let new_entry = Entry::Directory(DirectoryEntry {
            children: vec![],
            path: std_path,
            ignored: false,
            loaded: false,
        });

        // Insert the new entry and return a mutable reference to it
        self.children.push(new_entry);
        self.children.last_mut()
    }
}

#[cfg(test)]
#[path = "entry_tests.rs"]
mod tests;
