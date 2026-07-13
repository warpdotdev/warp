#![cfg_attr(not(feature = "local_fs"), allow(dead_code))]
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use futures_lite::future::yield_now;
use fuzzy_match::FuzzyMatchResult;
use itertools::Itertools;
#[cfg(feature = "local_fs")]
use repo_metadata::repositories::DetectedRepositories;
use warpui::{AppContext, SingletonEntity};

use super::search_item::FileSearchItem;
#[cfg(feature = "local_fs")]
use crate::code::opened_files::OpenedFilesModel;
use crate::search::ai_context_menu::mixer::AIContextMenuSearchableAction;
use crate::search::async_snapshot_data_source::AsyncSnapshotDataSource;
use crate::search::data_source::{Query, QueryResult};
use crate::search::files::model::FileSearchModel;
use crate::search::files::search_item::FileSearchResult;
#[cfg(feature = "local_fs")]
use crate::search::files::streaming::StreamingFileSearchSession;
use crate::search::mixer::{BoxFuture, DataSourceRunErrorWrapper};
#[cfg(feature = "local_fs")]
use crate::workspace::ActiveSession;

const MAX_RESULTS: usize = 200;

/// How many nucleo-ranked candidates to overfetch from the streaming engine
/// before the precision re-ranking pass reduces them to `MAX_RESULTS`.
#[cfg(feature = "local_fs")]
const STREAMING_OVERFETCH_FACTOR: usize = 5;

pub(crate) struct FileSnapshot {
    pub(crate) contents: Arc<Vec<FileSearchResult>>,
    pub(crate) git_changed_files: HashSet<String>,
    pub(crate) query_text: String,
    /// Last-opened timestamps for files, keyed by path. Populated from
    /// `OpenedFilesModel` at snapshot time. Used as a secondary recency
    /// signal within each scoring tier.
    pub(crate) last_opened: HashMap<String, instant::Instant>,
    /// When set, `contents` is empty at snapshot time and is collected from
    /// the streaming engine during the async match phase (gated by
    /// `FeatureFlag::StreamingFileSearch`).
    #[cfg(feature = "local_fs")]
    pub(crate) streaming_session: Option<Arc<StreamingFileSearchSession>>,
}

impl FileSnapshot {
    fn empty(query_text: String) -> Self {
        FileSnapshot {
            contents: Arc::new(Vec::new()),
            git_changed_files: HashSet::new(),
            query_text,
            last_opened: HashMap::new(),
            #[cfg(feature = "local_fs")]
            streaming_session: None,
        }
    }
}

/// Builds the repository-backed file search source used by the AI context menu.
/// For empty queries, snapshots repo contents with git-change status to prioritize modified files,
/// and for non-empty queries snapshots repo contents only for faster fuzzy matching.
///
/// When `FeatureFlag::StreamingFileSearch` is enabled and the active repo is
/// local, a streaming search session is created instead (per menu open) and
/// candidates are collected on the fly during matching.
pub fn file_data_source_for_current_repo(
    app: &AppContext,
) -> AsyncSnapshotDataSource<FileSnapshot, AIContextMenuSearchableAction> {
    #[cfg(feature = "local_fs")]
    let streaming_session = StreamingFileSearchSession::for_active_local_repo(app);
    #[cfg(not(feature = "local_fs"))]
    let _ = app;
    AsyncSnapshotDataSource::new(
        move |query: &Query, app: &AppContext| {
            if FileSearchModel::should_skip_overly_broad_query(&query.text) {
                return FileSnapshot::empty(query.text.clone());
            }

            let last_opened = snapshot_last_opened(app);

            #[cfg(feature = "local_fs")]
            if let Some(session) = &streaming_session {
                let git_changed_files = if query.text.is_empty() {
                    session.git_changed_files().clone()
                } else {
                    HashSet::new()
                };
                return FileSnapshot {
                    contents: Arc::new(Vec::new()),
                    git_changed_files,
                    query_text: query.text.clone(),
                    last_opened,
                    streaming_session: Some(session.clone()),
                };
            }

            let file_search_model = FileSearchModel::as_ref(app);
            if query.text.is_empty() {
                let (contents, git_changed_files) =
                    file_search_model.get_repo_contents_with_git_status(app);
                FileSnapshot {
                    contents,
                    git_changed_files,
                    query_text: query.text.clone(),
                    last_opened,
                    #[cfg(feature = "local_fs")]
                    streaming_session: None,
                }
            } else {
                let contents = file_search_model.get_repo_contents(&query.text, app);
                FileSnapshot {
                    contents,
                    git_changed_files: HashSet::new(),
                    query_text: query.text.clone(),
                    last_opened,
                    #[cfg(feature = "local_fs")]
                    streaming_session: None,
                }
            }
        },
        fuzzy_match_files,
    )
}

pub fn file_data_source_for_pwd(
    app: &AppContext,
) -> AsyncSnapshotDataSource<FileSnapshot, AIContextMenuSearchableAction> {
    let file_search_model = FileSearchModel::as_ref(app);
    let mut cached_contents = file_search_model.get_folder_contents(app);
    // Reverse sort to put what you'd expect at the top for zero-state
    cached_contents.sort_by(|a, b| b.path.cmp(&a.path));
    let cached_contents = Arc::new(cached_contents);

    AsyncSnapshotDataSource::new(
        move |query: &Query, _app: &AppContext| {
            if FileSearchModel::should_skip_overly_broad_query(&query.text) {
                return FileSnapshot::empty(query.text.clone());
            }

            FileSnapshot {
                contents: cached_contents.clone(),
                git_changed_files: HashSet::new(),
                query_text: query.text.clone(),
                last_opened: HashMap::new(),
                #[cfg(feature = "local_fs")]
                streaming_session: None,
            }
        },
        fuzzy_match_files,
    )
}

/// Captures last-opened timestamps from `OpenedFilesModel` for the active
/// repo at snapshot time. Returns an empty map when no repo is active.
#[cfg(feature = "local_fs")]
fn snapshot_last_opened(app: &AppContext) -> HashMap<String, instant::Instant> {
    let repo_root = app
        .windows()
        .state()
        .active_window
        .and_then(|window_id| ActiveSession::as_ref(app).working_directory(window_id))
        .and_then(|working_dir| DetectedRepositories::as_ref(app).get_root_for_path(working_dir));

    let Some(repo_root) = repo_root else {
        return HashMap::new();
    };

    let opened_files_model = OpenedFilesModel::as_ref(app);
    let Some(opened_in_repo) = opened_files_model.opened_files_for_repo(&repo_root) else {
        return HashMap::new();
    };

    opened_in_repo
        .iter()
        .map(|(path, ts)| (path.clone(), *ts))
        .collect()
}

/// File-open recency is unavailable without a local filesystem.
#[cfg(not(feature = "local_fs"))]
fn snapshot_last_opened(_app: &AppContext) -> HashMap<String, instant::Instant> {
    HashMap::new()
}

/// Routes file matching to zero-state ranking or query-based fuzzy scoring.
pub(crate) fn fuzzy_match_files(
    snapshot: FileSnapshot,
) -> BoxFuture<
    'static,
    Result<Vec<QueryResult<AIContextMenuSearchableAction>>, DataSourceRunErrorWrapper>,
> {
    Box::pin(async move {
        let snapshot = resolve_streaming_contents(snapshot).await;
        if snapshot.query_text.is_empty() {
            Ok(fuzzy_match_files_zero_state(snapshot).await)
        } else {
            Ok(fuzzy_match_files_query(snapshot).await)
        }
    })
}

/// Collects candidates from the streaming engine when the snapshot was taken
/// on the streaming path; otherwise returns the snapshot unchanged. Empty and
/// wildcard queries need every candidate (zero-state ordering and wildcard
/// matching happen in the scoring passes below); fuzzy queries re-rank a
/// nucleo-ranked overfetch.
#[cfg(feature = "local_fs")]
async fn resolve_streaming_contents(mut snapshot: FileSnapshot) -> FileSnapshot {
    if let Some(session) = &snapshot.streaming_session {
        let max_results = if snapshot.query_text.is_empty()
            || fuzzy_match::contains_wildcards(&snapshot.query_text)
        {
            usize::MAX
        } else {
            MAX_RESULTS * STREAMING_OVERFETCH_FACTOR
        };
        snapshot.contents = Arc::new(
            session
                .collect_candidates(&snapshot.query_text, max_results)
                .await,
        );
    }
    snapshot
}

#[cfg(not(feature = "local_fs"))]
async fn resolve_streaming_contents(snapshot: FileSnapshot) -> FileSnapshot {
    snapshot
}

/// Build a recency index: sort files by last-opened timestamp (ascending,
/// `None` first) and return a map from path to sort position.
fn build_recency_index(
    contents: &[FileSearchResult],
    last_opened: &HashMap<String, instant::Instant>,
) -> HashMap<String, usize> {
    let mut opened: Vec<_> = contents
        .iter()
        .filter_map(|item| last_opened.get(&item.path).map(|ts| (&item.path, ts)))
        .collect();
    opened.sort_by_key(|(_, ts)| *ts);
    opened
        .into_iter()
        .enumerate()
        .map(|(rank, (path, _))| (path.clone(), rank + 1))
        .collect()
}

/// Returns zero-state file results with two scoring tiers and recency
/// as a secondary sort within each tier.
async fn fuzzy_match_files_zero_state(
    snapshot: FileSnapshot,
) -> Vec<QueryResult<AIContextMenuSearchableAction>> {
    let recency_index = build_recency_index(&snapshot.contents, &snapshot.last_opened);
    let max_recency = recency_index.len();
    let mut results: Vec<QueryResult<AIContextMenuSearchableAction>> = Vec::new();

    // Pass 1: git-changed or recently-opened files (guaranteed inclusion)
    for chunk in snapshot.contents.chunks(512) {
        for item in chunk {
            let is_git_changed = snapshot.git_changed_files.contains(&item.path);
            let is_recently_opened = snapshot.last_opened.contains_key(&item.path);

            if is_git_changed || is_recently_opened {
                let rank = recency_index.get(&item.path).copied().unwrap_or(0);
                let recency_bonus = if max_recency > 0 {
                    (30 * rank / max_recency) as i64
                } else {
                    0
                };
                let base_score = if is_git_changed { 10000 } else { 0 };
                let match_result = FuzzyMatchResult {
                    score: base_score + recency_bonus,
                    matched_indices: vec![],
                };
                let search_item = FileSearchItem {
                    path: PathBuf::from(&item.path),
                    match_result,
                    is_directory: item.is_directory,
                };
                results.push(QueryResult::from(search_item));
            }
        }
        yield_now().await;
    }

    // Pass 2: fill remaining capacity with untouched files
    for chunk in snapshot.contents.chunks(512) {
        for item in chunk {
            if !snapshot.git_changed_files.contains(&item.path)
                && !snapshot.last_opened.contains_key(&item.path)
                && results.len() < MAX_RESULTS
            {
                let match_result = FuzzyMatchResult {
                    score: 0,
                    matched_indices: vec![],
                };
                let search_item = FileSearchItem {
                    path: PathBuf::from(&item.path),
                    match_result,
                    is_directory: item.is_directory,
                };
                results.push(QueryResult::from(search_item));
            }
        }
        yield_now().await;
    }

    results
}

/// Returns fuzzy-ranked file results for non-empty queries.
async fn fuzzy_match_files_query(
    snapshot: FileSnapshot,
) -> Vec<QueryResult<AIContextMenuSearchableAction>> {
    let recency_index = build_recency_index(&snapshot.contents, &snapshot.last_opened);
    let max_recency = recency_index.len();
    let mut results = Vec::new();

    for chunk in snapshot.contents.chunks(512) {
        for item in chunk {
            if let Some(mut match_result) =
                FileSearchModel::fuzzy_match_path(&item.path, &snapshot.query_text)
            {
                // Give files a slight boost over directories to prioritize them when names are similar
                if !item.is_directory {
                    match_result.score += 100;
                }

                // Add a recency bonus, capped at 30.
                let rank = recency_index.get(&item.path).copied().unwrap_or(0);
                let recency_bonus = if max_recency > 0 {
                    (30 * rank / max_recency) as i64
                } else {
                    0
                };

                match_result.score += recency_bonus;

                let search_item = FileSearchItem {
                    path: PathBuf::from(&item.path),
                    match_result,
                    is_directory: item.is_directory,
                };
                results.push(QueryResult::from(search_item));
            }
        }
        yield_now().await;
    }

    results
        .into_iter()
        .k_largest_relaxed_by_key(MAX_RESULTS, |item| item.score())
        .collect()
}

#[cfg(test)]
#[path = "data_source_tests.rs"]
mod tests;
