//! Glue between the file-search data sources and the on-the-fly
//! [`streaming_file_search`] engine (gated by
//! [`FeatureFlag::StreamingFileSearch`]).
//!
//! A [`StreamingFileSearchSession`] is created when a file picker opens (cmd-O
//! command palette or the `@` context menu), owns a streaming engine for the
//! active local repository, and is dropped when the picker closes. The engine
//! provides fast *recall* (streamed candidates, nucleo-ranked); callers keep
//! their existing precision re-ranking ([`FileSearchModel::fuzzy_match_path`]
//! plus git-changed / recently-opened bonuses), so ordering and highlighting
//! match the non-streaming implementation.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fuzzy_match::contains_wildcards;
use instant::Instant;
use streaming_file_search::{FileSearchEngine, StreamingFileSearchEngine};
use warp_core::features::FeatureFlag;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{AppContext, SingletonEntity};

use crate::search::files::model::FileSearchModel;
use crate::search::files::search_item::FileSearchResult;

/// How long a single query is willing to wait for the initial filesystem scan
/// to stream in more candidates before returning what has been found so far.
/// Later queries (or keystrokes) pick up the newly streamed candidates.
const SCAN_WAIT_BUDGET: Duration = Duration::from_millis(150);

/// Safety valve for waiting on the matcher itself once the scan is complete.
/// Matching is parallel and cheap, so this is practically never hit.
const MATCH_WAIT_BUDGET: Duration = Duration::from_secs(2);

/// How long each poll blocks on matcher progress before yielding back to the
/// executor so the query future stays cancellable.
const POLL_TIMEOUT_MS: u64 = 10;

/// A per-picker-open streaming search session for the active local repo.
pub struct StreamingFileSearchSession {
    engine: Mutex<StreamingFileSearchEngine>,
    /// Canonicalized repo root, formatted like the eager index's
    /// `project_directory`.
    project_directory: String,
    /// Repo-relative paths reported by `git status --porcelain` at session
    /// creation, used for zero-state prioritization.
    git_changed_files: HashSet<String>,
}

impl StreamingFileSearchSession {
    /// Creates a session for the active window's repository. Returns `None`
    /// when the streaming search flag is disabled, there is no active repo, or
    /// the repo is remote (remote sessions stay on the eager-index path until
    /// the remote search RPC exists).
    pub fn for_active_local_repo(app: &AppContext) -> Option<Arc<Self>> {
        if !FeatureFlag::StreamingFileSearch.is_enabled() {
            return None;
        }
        let file_search_model = FileSearchModel::as_ref(app);
        let repo_root = match file_search_model.repo_root_location(app)? {
            LocalOrRemotePath::Local(root) => root,
            LocalOrRemotePath::Remote(_) => return None,
        };
        let canonical_root = dunce::canonicalize(&repo_root).ok()?;
        let git_changed_files = file_search_model
            .get_git_changed_files(&canonical_root)
            .unwrap_or_default();
        let engine = StreamingFileSearchEngine::new(canonical_root.clone());
        Some(Arc::new(Self {
            engine: Mutex::new(engine),
            project_directory: canonical_root.to_string_lossy().to_string(),
            git_changed_files,
        }))
    }

    pub fn git_changed_files(&self) -> &HashSet<String> {
        &self.git_changed_files
    }

    /// Collects up to `max_results` candidates for `query`, best match first.
    ///
    /// Empty and wildcard queries collect *all* candidates in discovery order
    /// (wildcard semantics are applied by the caller's re-ranking pass, and
    /// zero state applies its own ordering), matching the contract of
    /// `FileSearchModel::get_repo_contents`. Non-empty fuzzy queries return a
    /// nucleo-ranked superset for the caller to re-rank.
    ///
    /// The engine lock is only held between yield points, so concurrent
    /// queries (e.g. a stale in-flight query racing a new keystroke) serialize
    /// safely.
    pub async fn collect_candidates(
        &self,
        query: &str,
        max_results: usize,
    ) -> Vec<FileSearchResult> {
        let engine_query = if contains_wildcards(query) { "" } else { query };
        {
            let mut engine = self.lock_engine();
            engine.refresh_if_stale();
            engine.update_query(engine_query);
        }

        let start = Instant::now();
        loop {
            let (running, scanning) = {
                let mut engine = self.lock_engine();
                let status = engine.poll(POLL_TIMEOUT_MS);
                (status.running, engine.is_scanning())
            };
            if !running && !scanning {
                break;
            }
            let elapsed = start.elapsed();
            if scanning && elapsed > SCAN_WAIT_BUDGET {
                // Return what has streamed in so far; subsequent queries will
                // see the rest of the scan.
                break;
            }
            if elapsed > MATCH_WAIT_BUDGET {
                break;
            }
            futures_lite::future::yield_now().await;
        }

        let candidates = self.lock_engine().matched(max_results);
        candidates
            .into_iter()
            .map(|candidate| FileSearchResult {
                path: candidate.relative_path,
                project_directory: self.project_directory.clone(),
                is_directory: candidate.is_directory,
            })
            .collect()
    }

    fn lock_engine(&self) -> std::sync::MutexGuard<'_, StreamingFileSearchEngine> {
        // The engine has no lock-poisoning-sensitive state; recover the guard
        // if a previous holder panicked.
        match self.engine.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

#[cfg(test)]
#[path = "streaming_tests.rs"]
mod tests;
