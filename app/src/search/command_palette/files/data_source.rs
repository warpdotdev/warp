#![cfg_attr(not(feature = "local_fs"), allow(dead_code))]
use std::collections::HashSet;
#[cfg(feature = "local_fs")]
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use futures_lite::FutureExt;
use fuzzy_match::FuzzyMatchResult;
use instant::Instant;
use itertools::Itertools;
use warp_util::path::CleanPathResult;
use warpui::{AppContext, Entity, SingletonEntity};

use super::search_item::{CreateFileSearchItem, FileSearchItem};
use crate::code::opened_files::{OpenedFilesInRepo, OpenedFilesModel};
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::data_source::{Query, QueryFilter, QueryResult};
use crate::search::files::model::FileSearchModel;
use crate::search::files::search_item::FileSearchResult;
use crate::search::mixer::{AsyncDataSource, BoxFuture, DataSourceRunErrorWrapper};

const MAX_RESULTS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum FileRanking {
    None,
    ChangedInGit,
    OpenedInWarp { timestamp: Instant },
}

pub struct FileDataSource {
    mode: FileDataSourceMode,
}

enum FileDataSourceMode {
    /// Search across the repository (existing behavior)
    Repo,
    /// Search within the current folder only, using cached contents computed at creation time
    CurrentFolder {
        cached_contents: Vec<FileSearchResult>,
    },
}

impl FileDataSource {
    pub fn new() -> Self {
        // Default to repo search to preserve existing call sites
        Self {
            mode: FileDataSourceMode::Repo,
        }
    }

    /// Create a data source that searches only within the current folder.
    /// This will read folder contents once at creation and reuse them for subsequent queries.
    pub fn new_current_folder(app: &AppContext) -> Self {
        let file_search_model = FileSearchModel::as_ref(app);
        let contents = file_search_model.get_folder_contents(app);
        Self {
            mode: FileDataSourceMode::CurrentFolder {
                cached_contents: contents,
            },
        }
    }
}

impl AsyncDataSource for FileDataSource {
    type Action = CommandPaletteItemAction;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> BoxFuture<'static, Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper>> {
        // Get the search query text
        let query_text = &query.text;

        // Early exit for very broad wildcard patterns that would match everything
        if FileSearchModel::should_skip_overly_broad_query(query_text) {
            return futures::future::ready(Ok(vec![])).boxed();
        }

        // Zero state: fetch git-changed files and prioritize them
        if query_text.is_empty() {
            self.run_zero_state_query(app)
        } else {
            // Non-empty query: use fuzzy matching
            self.run_fuzzy_search_query(
                app,
                query_text,
                query.filters.contains(&QueryFilter::Files),
            )
        }
    }
}

impl FileDataSource {
    fn contents_with_git_changes(
        &self,
        app: &AppContext,
    ) -> (Arc<Vec<FileSearchResult>>, HashSet<String>) {
        match &self.mode {
            FileDataSourceMode::Repo => {
                let file_search_model = FileSearchModel::as_ref(app);
                file_search_model.get_repo_contents_with_git_status(app)
            }
            FileDataSourceMode::CurrentFolder { cached_contents } => {
                (Arc::new(cached_contents.clone()), HashSet::new())
            }
        }
    }

    fn contents(&self, query: &str, app: &AppContext) -> Arc<Vec<FileSearchResult>> {
        match &self.mode {
            FileDataSourceMode::Repo => {
                let file_search_model = FileSearchModel::as_ref(app);
                file_search_model.get_repo_file_contents(query, app)
            }
            FileDataSourceMode::CurrentFolder { cached_contents } => {
                Arc::new(cached_contents.clone())
            }
        }
    }

    /// Handle zero state query - prioritize git-changed files without fuzzy matching
    fn run_zero_state_query(
        &self,
        app: &AppContext,
    ) -> BoxFuture<
        'static,
        Result<Vec<QueryResult<CommandPaletteItemAction>>, DataSourceRunErrorWrapper>,
    > {
        let file_search_model = FileSearchModel::as_ref(app);

        let (contents, git_changed_files) = self.contents_with_git_changes(app);

        let opened_files = OpenedFilesModel::as_ref(app);

        let repo_root = file_search_model.repo_root_location(app);
        let opened_files = repo_root
            .and_then(|repo_root| opened_files.opened_files_for_repo(&repo_root))
            .cloned();

        Box::pin(async move {
            let mut results = Vec::new();

            for chunk in contents.chunks(50) {
                for item in chunk {
                    if item.is_directory {
                        continue;
                    }

                    let mut file_ranking = if git_changed_files.contains(&item.path) {
                        FileRanking::ChangedInGit
                    } else {
                        FileRanking::None
                    };

                    if let Some(last_opened_timestamp) = opened_files
                        .as_ref()
                        .and_then(|of: &OpenedFilesInRepo| of.get(&item.path))
                    {
                        file_ranking = FileRanking::OpenedInWarp {
                            timestamp: *last_opened_timestamp,
                        };
                    }

                    let match_result = FuzzyMatchResult {
                        score: 0,
                        matched_indices: vec![], // No highlighting needed for zero state
                    };

                    let search_item = FileSearchItem {
                        path: PathBuf::from(&item.path),
                        project_directory: item.project_directory.clone(),
                        match_result,
                        line_and_column_arg: None,
                        is_directory: item.is_directory,
                    };
                    results.push((file_ranking, QueryResult::from(search_item)));
                }
                futures_lite::future::yield_now().await;
            }

            results.sort_by_key(|(ranking, _)| *ranking);

            Ok(results.into_iter().map(|(_, ranking)| ranking).collect())
        })
    }

    /// Handle non-empty query with fuzzy matching (no git status needed)
    fn run_fuzzy_search_query(
        &self,
        app: &AppContext,
        query_text: &str,
        allow_create_file: bool,
    ) -> BoxFuture<
        'static,
        Result<Vec<QueryResult<CommandPaletteItemAction>>, DataSourceRunErrorWrapper>,
    > {
        let file_search_model = FileSearchModel::as_ref(app);

        // Strip any trailing : in case user is in the middle of typing a line / column arg.
        let query_text = query_text.strip_suffix(':').unwrap_or(query_text);

        let text = CleanPathResult::with_line_and_column_number(query_text);
        let query_file_content = text.path;

        let opened_files = OpenedFilesModel::as_ref(app);

        #[cfg(feature = "local_fs")]
        let repo_root = file_search_model.repo_root(app);
        let repo_root_location = file_search_model.repo_root_location(app);

        // For the "Create file" fallback, use the expanded (but not repo-root-stripped)
        // path so that absolute paths work correctly with Path::join.
        let query_file_name = shellexpand::tilde(&query_file_content).into_owned();

        // Get the current directory for the "Create file" option and for path stripping.
        #[cfg(feature = "local_fs")]
        let current_directory = {
            use crate::workspace::ActiveSession;
            let active_window_id = app.windows().state().active_window;
            active_window_id
                .and_then(|window_id| ActiveSession::as_ref(app).path_if_local(window_id))
                .map(|path| path.to_string_lossy().to_string())
        };
        #[cfg(not(feature = "local_fs"))]
        let current_directory: Option<String> = None;

        // If the query looks like an absolute path, strip the common prefix with the
        // repo root (first) or working directory (second) so it can match against the
        // relative paths stored in the file index.  This allows users to paste absolute
        // paths — e.g. copied via "Copy file path" in the Code Review pane — directly
        // into the Command-Palette file picker.  We pass the tilde-expanded
        // `query_file_name` so that `~/...` paths are also handled.
        #[cfg(feature = "local_fs")]
        let query_file_content = FileSearchModel::strip_absolute_path_prefix(
            &query_file_name,
            repo_root.as_deref(),
            current_directory.as_deref().map(Path::new),
        )
        .unwrap_or(query_file_content);

        let opened_files = repo_root_location
            .and_then(|repo_root| opened_files.opened_files_for_repo(&repo_root))
            .cloned();

        // Fetch contents using the finalized query so it is pushed down into
        // the repo-metadata traversal as a filter (matching files are not
        // truncated away before fuzzy matching).
        let contents = self.contents(&query_file_content, app);

        const CHUNK_SIZE: usize = 50;

        Box::pin(async move {
            let mut results = Vec::with_capacity(contents.len());

            // Iterate in chunks of 50, yielding at the end of each chunk to
            // allow the main thread to abort the search if needed.
            for chunk in contents.chunks(CHUNK_SIZE) {
                for item in chunk {
                    if item.is_directory {
                        continue;
                    }

                    let Some(mut match_result) =
                        FileSearchModel::fuzzy_match_path(&item.path, &query_file_content)
                    else {
                        continue;
                    };

                    if opened_files
                        .as_ref()
                        .and_then(|of: &OpenedFilesInRepo| of.get(&item.path))
                        .is_some()
                    {
                        // Apply a boost to opened files to rank them above non-opened files.
                        match_result.score += 100;
                    };

                    let search_item = FileSearchItem {
                        path: PathBuf::from(&item.path),
                        project_directory: item.project_directory.clone(),
                        line_and_column_arg: text.line_and_column_num,
                        match_result,
                        is_directory: item.is_directory,
                    };
                    results.push(search_item);
                }
                futures_lite::future::yield_now().await;
            }

            let mut results: Vec<QueryResult<CommandPaletteItemAction>> = results
                .into_iter()
                .k_largest_relaxed_by_key(MAX_RESULTS, |item| item.match_result.score)
                .map(QueryResult::from)
                .collect();

            // If no files matched and we have a valid query and current directory,
            // add a "Create a file named <filename>..." option
            if allow_create_file && results.is_empty() && !query_file_name.trim().is_empty() {
                if let Some(current_dir) = current_directory {
                    let create_item = CreateFileSearchItem {
                        file_name: query_file_name,
                        current_directory: current_dir,
                    };
                    results.push(QueryResult::from(create_item));
                }
            }

            Ok(results)
        })
    }
}

impl Entity for FileDataSource {
    type Event = ();
}

#[cfg(test)]
#[path = "data_source_tests.rs"]
mod tests;
