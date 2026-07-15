#![cfg_attr(not(feature = "local_fs"), allow(dead_code))]
//! Standalone, filename-filtered walker that evaluates standing repository
//! queries (project skills and rules) directly from the filesystem, without
//! materializing a file tree.
//!
//! This mirrors the traversal semantics of
//! [`Entry::build_tree_with_standing_queries`](crate::entry::Entry) so that
//! consumers switching between the eager tree-derived standing results and
//! this walk observe the same matches:
//! - gitignore handling starts from the root's `.gitignore` plus the global
//!   gitignore and accumulates nested `.gitignore` files as directories are
//!   visited; ignored directories are not descended into.
//! - `.git` internals are pruned.
//! - Force-included paths (e.g. skill provider directories such as
//!   `.agents/skills`) are descended into even when gitignored or beyond the
//!   depth limit.
//! - Directory symlinks are not followed, but eligible symlinked project
//!   skill directories are still surfaced via their lexical paths.
//!
//! Unlike the tree build, the walk has no per-build file budget: it only
//! enumerates directory entries and retains filename matches, so covering the
//! entire repository stays cheap even for repositories whose eager index is
//! truncated by the file budget.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use ignore::gitignore::Gitignore;

use crate::entry::{
    gitignores_for_directory, is_git_internal_path, matches_force_included_path, matches_gitignores,
};
use crate::standing_queries::{StandingQueryDefinitions, StandingQueryResults};

/// Maximum directory depth for standing-query walks of full repositories.
///
/// Matches the eager tree build's depth limit (`MAX_TREE_DEPTH` in
/// `local_model.rs`) so walk coverage stays consistent with the standing
/// results computed during eager indexing.
pub const STANDING_QUERY_WALK_MAX_DEPTH: usize = 200;

/// Options controlling a standing-query walk.
#[derive(Debug, Clone)]
pub struct StandingQueryWalkOptions {
    /// Directories deeper than this (relative to the walk root) are not
    /// descended into unless they lie on the way to a force-included path.
    /// Use `1` to mirror the first-level-only coverage of lazily-loaded
    /// non-repository paths.
    pub max_depth: usize,
    /// Repository-relative path suffixes (e.g. `.agents/skills`) that are
    /// always descended into, even when gitignored or beyond `max_depth`.
    pub force_included_paths: Vec<PathBuf>,
}

impl Default for StandingQueryWalkOptions {
    fn default() -> Self {
        Self {
            max_depth: STANDING_QUERY_WALK_MAX_DEPTH,
            force_included_paths: Vec::new(),
        }
    }
}

/// Walks `root_path` and returns every path matching a standing query.
///
/// This is a blocking filesystem traversal; callers should run it on a
/// background executor for large repositories.
pub fn evaluate_standing_queries(
    root_path: &Path,
    definitions: &StandingQueryDefinitions,
    options: &StandingQueryWalkOptions,
) -> StandingQueryResults {
    let mut results = StandingQueryResults::default();
    let mut gitignores = gitignores_for_directory(root_path);

    let root_is_dir = root_path.is_dir();
    results.record_path(root_path, root_is_dir, definitions);

    // Mirror the tree build: directory symlinks are never followed, including
    // at the root.
    if !root_is_dir || root_path.is_symlink() {
        return results;
    }

    let root_ignored = matches_gitignores(
        root_path,
        true,
        &gitignores,
        false, /* check_ancestors */
    );

    struct DirJob {
        path: PathBuf,
        depth: usize,
        ignored: bool,
    }

    let mut queue = VecDeque::new();
    queue.push_back(DirJob {
        path: root_path.to_path_buf(),
        depth: 0,
        ignored: root_ignored,
    });

    while let Some(job) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&job.path) else {
            // Unreadable directories are skipped, matching the tree build's
            // treatment of unreadable nested directories.
            continue;
        };

        let child_depth = job.depth + 1;
        for entry in entries {
            let Ok(entry) = entry else {
                continue;
            };
            let entry_path = entry.path();

            // Directory symlinks are not followed. Eligible symlinked project
            // skill directories are still retained through their lexical
            // paths, matching the tree build.
            let child_path = if entry_path.is_symlink() {
                if entry_path.is_dir() {
                    results.record_followed_project_skill_directory(&entry_path, definitions);
                    continue;
                }
                entry_path
            } else {
                match dunce::canonicalize(entry_path) {
                    Ok(path) => path,
                    Err(_) => continue,
                }
            };

            let is_dir = child_path.is_dir();
            results.record_path(&child_path, is_dir, definitions);
            if !is_dir {
                continue;
            }

            // Load this directory's `.gitignore` before deciding whether to
            // descend, matching `evaluate_entry` in the tree build.
            let gitignore_path = child_path.join(".gitignore");
            if gitignore_path.exists() {
                let (gitignore, _) = Gitignore::new(gitignore_path);
                gitignores.push(gitignore);
            }

            let ignored = job.ignored
                || is_git_internal_path(&child_path)
                || matches_gitignores(
                    &child_path,
                    true,
                    &gitignores,
                    false, /* check_ancestors */
                );
            let force_included =
                matches_force_included_path(&child_path, &options.force_included_paths);

            // Same descend rule as the tree build with
            // `IgnoredPathStrategy::IncludeLazy`: ignored directories and
            // directories past the depth limit stay unexplored unless they lie
            // on the way to a force-included path.
            let lazy = (ignored || child_depth >= options.max_depth) && !force_included;
            if !lazy {
                queue.push_back(DirJob {
                    path: child_path,
                    depth: child_depth,
                    ignored,
                });
            }
        }
    }

    results
}

#[cfg(test)]
#[path = "standing_query_walker_tests.rs"]
mod tests;
