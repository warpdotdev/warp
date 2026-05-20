#![cfg_attr(not(feature = "local_fs"), allow(dead_code))]
#[cfg(not(target_family = "wasm"))]
use super::search_item::CodeSearchItem;
#[cfg(not(target_family = "wasm"))]
use crate::search::ai_context_menu::mixer::AIContextMenuSearchableAction;
#[cfg(not(target_family = "wasm"))]
use crate::search::data_source::{Query, QueryResult};
#[cfg(not(target_family = "wasm"))]
use crate::search::files::model::FileSearchModel;
#[cfg(not(target_family = "wasm"))]
use crate::search::mixer::{
    AsyncDataSource, BoxFuture, DataSourceRunError, DataSourceRunErrorWrapper,
};
use ai::index::Symbol;
use fuzzy_match::FuzzyMatchResult;
#[cfg(not(target_family = "wasm"))]
use instant::Instant;
#[cfg(not(target_family = "wasm"))]
use itertools::Itertools;
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
#[cfg(not(target_family = "wasm"))]
use std::time::Duration;
use warpui::AppContext;
#[cfg(not(target_family = "wasm"))]
use warpui::ModelSpawner;

#[cfg(not(target_family = "wasm"))]
use crate::ai::outline::{OutlineStatus, RepoOutlines, RepoOutlinesEvent};
#[cfg(not(target_family = "wasm"))]
use crate::workspace::ActiveSession;
#[cfg(not(target_family = "wasm"))]
use repo_metadata::repositories::DetectedRepositories;
#[cfg(not(target_family = "wasm"))]
use std::path::Path;
#[cfg(not(target_family = "wasm"))]
use warp_util::local_or_remote_path::LocalOrRemotePath;
#[cfg(not(target_family = "wasm"))]
use warpui::SingletonEntity;

const MAX_RESULTS: usize = 200;

/// Represents a single code symbol within a file
#[derive(Debug, Clone)]
pub struct CodeSymbol {
    pub file_path: PathBuf,
    pub symbol: Symbol,
}

/// Symbol cache that stores all symbols in a simple vector
pub struct SymbolCache {
    /// All symbols stored in a vector
    pub(crate) symbols: Vec<CodeSymbol>,
}

impl SymbolCache {
    fn new(symbols: Vec<CodeSymbol>) -> Self {
        Self { symbols }
    }
}

/// Entity that owns a per-repo map of cached [`CodeSymbol`]s (the "symbol cache").
/// Lives on `AIContextMenu` so the cache persists across mixer resets.
///
/// On construction subscribes to [`RepoOutlinesEvent::OutlinesUpdated`]; when an
/// outline changes for a repo, the corresponding cache entry is evicted so the next
/// query re-populates it from the fresh outline.
pub struct CodeSymbolCache {
    symbol_cache: RefCell<HashMap<PathBuf, SymbolCache>>,
    #[cfg(not(target_family = "wasm"))]
    spawner: ModelSpawner<Self>,
}

impl warpui::Entity for CodeSymbolCache {
    type Event = ();
}

impl CodeSymbolCache {
    #[cfg(not(target_family = "wasm"))]
    pub fn new(ctx: &mut warpui::ModelContext<Self>) -> Self {
        let spawner = ctx.spawner();
        let cache = Self {
            symbol_cache: RefCell::new(HashMap::new()),
            spawner,
        };

        ctx.subscribe_to_model(&RepoOutlines::handle(ctx), |me, event, ctx| match event {
            RepoOutlinesEvent::OutlinesUpdated(repo_path) => {
                me.symbol_cache.get_mut().remove(repo_path);
                ctx.emit(());
            }
        });

        cache
    }

    #[cfg(target_family = "wasm")]
    pub fn new() -> Self {
        Self {
            symbol_cache: RefCell::new(HashMap::new()),
        }
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn spawner(&self) -> ModelSpawner<Self> {
        self.spawner.clone()
    }

    /// Resolves the active git repo from the current window, looks up its outline,
    /// and lazily populates the symbol cache from that outline. Returns the repo
    /// path and total symbol count, or `None` when no repo or completed outline is
    /// available.
    #[cfg(not(target_family = "wasm"))]
    pub fn ensure_symbols_cached(&mut self, app: &AppContext) -> Option<(PathBuf, usize)> {
        let git_repo_path = app
            .windows()
            .state()
            .active_window
            .and_then(|window_id| ActiveSession::as_ref(app).path_if_local(window_id))
            .and_then(|current_dir| {
                DetectedRepositories::as_ref(app)
                    .get_root_for_path(&LocalOrRemotePath::Local(
                        Path::new(current_dir).to_path_buf(),
                    ))
                    .and_then(|r| PathBuf::try_from(r).ok())
            })?;

        let (outline_status, _) = RepoOutlines::as_ref(app).get_outline(&git_repo_path)?;
        let outline = match outline_status {
            OutlineStatus::Complete(outline) => outline,
            _ => return None,
        };

        let cache = self.symbol_cache.get_mut();
        let cached = cache.entry(git_repo_path.clone()).or_insert_with(|| {
            let symbols = outline
                .to_symbols_by_file(None)
                .into_iter()
                .flat_map(|(file_path, file_outline)| {
                    let prefix = git_repo_path.clone();
                    file_outline
                        .symbols()
                        .into_iter()
                        .flatten()
                        .map(move |symbol| CodeSymbol {
                            file_path: file_path
                                .strip_prefix(&prefix)
                                .unwrap_or(&file_path)
                                .to_path_buf(),
                            symbol: symbol.clone(),
                        })
                        .collect::<Vec<_>>()
                })
                .collect();
            SymbolCache::new(symbols)
        });

        let count = cached.symbols.len();
        Some((git_repo_path, count))
    }

    /// Processes a chunk of symbols starting at `cursor`, fuzzy-matching each against `query`
    /// until `budget` is exceeded. Returns `(new_cursor, batch_results)`.
    #[cfg(not(target_family = "wasm"))]
    pub fn search_symbols_chunk(
        &mut self,
        repo_path: &Path,
        cursor: usize,
        query: &str,
        budget: Duration,
    ) -> (usize, Vec<CodeSearchItem>) {
        // If the cache was invalidated between chunks, signal the caller with usize::MAX.
        let Some(cached) = self.symbol_cache.get_mut().get(repo_path) else {
            return (usize::MAX, Vec::new());
        };

        let symbols = &cached.symbols;
        if cursor >= symbols.len() {
            return (symbols.len(), Vec::new());
        }

        let start = Instant::now();
        let mut batch = Vec::new();
        let mut i = cursor;
        while i < symbols.len() && start.elapsed() < budget {
            let symbol = &symbols[i];
            let match_result = fuzzy_match_symbol_with_type(symbol, query);
            batch.push(CodeSearchItem {
                code_symbol: symbol.clone(),
                match_result,
            });
            i += 1;
        }
        (i, batch)
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn get_git_changed_files(&self, app: &AppContext) -> HashSet<String> {
        let Some(git_repo_path) = app
            .windows()
            .state()
            .active_window
            .and_then(|window_id| ActiveSession::as_ref(app).path_if_local(window_id))
            .and_then(|current_dir| {
                DetectedRepositories::as_ref(app)
                    .get_root_for_path(&LocalOrRemotePath::Local(
                        Path::new(current_dir).to_path_buf(),
                    ))
                    .and_then(|r| PathBuf::try_from(r).ok())
            })
        else {
            return HashSet::new();
        };

        FileSearchModel::as_ref(app)
            .get_git_changed_files(&git_repo_path)
            .unwrap_or_default()
    }

    #[cfg(target_family = "wasm")]
    pub fn get_git_changed_files(&self, _app: &AppContext) -> HashSet<String> {
        HashSet::new()
    }
}

#[cfg(not(target_family = "wasm"))]
#[derive(Debug)]
struct CodeSearchError;

#[cfg(not(target_family = "wasm"))]
impl DataSourceRunError for CodeSearchError {
    fn user_facing_error(&self) -> String {
        "Code search failed".to_string()
    }

    fn telemetry_payload(&self) -> serde_json::Value {
        serde_json::json!({ "error": "model_dropped" })
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Data source that searches code symbols incrementally on the main thread
/// using time-budgeted chunks, avoiding bulk-cloning the symbol list.
#[cfg(not(target_family = "wasm"))]
pub struct CodeCursorDataSource {
    spawner: ModelSpawner<CodeSymbolCache>,
}

#[cfg(not(target_family = "wasm"))]
impl CodeCursorDataSource {
    pub fn new(spawner: ModelSpawner<CodeSymbolCache>) -> Self {
        Self { spawner }
    }
}

#[cfg(not(target_family = "wasm"))]
impl AsyncDataSource for CodeCursorDataSource {
    type Action = AIContextMenuSearchableAction;

    fn run_query(
        &self,
        query: &Query,
        _app: &AppContext,
    ) -> BoxFuture<'static, Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper>> {
        let spawner = self.spawner.clone();
        let query_text = query.text.clone();
        let is_zero_state = query_text.is_empty();

        Box::pin(async move {
            let map_err = |_| -> DataSourceRunErrorWrapper { Box::new(CodeSearchError) };

            // Populate cache, get repo path + count, and git-changed files if zero-state
            let init_query = query_text.clone();
            let init = spawner
                .spawn(move |cache, ctx| {
                    let (repo_path, total) = cache.ensure_symbols_cached(ctx)?;
                    let git_changed_files = if init_query.is_empty() {
                        cache.get_git_changed_files(ctx)
                    } else {
                        HashSet::new()
                    };
                    Some((repo_path, total, git_changed_files))
                })
                .await
                .map_err(map_err)?;

            let Some((repo_path, total, git_changed_files)) = init else {
                return Ok(Vec::new());
            };

            // We can't actually perform the search off of the main thread
            // (because we don't have access to the code data we need for searching).
            // Instead, we dispatch small search chunks to the main thread so it
            // can access the cache. We yield between chunks, letting
            // the main thread continue to perform render cycles while we're searching.
            let mut cursor = 0usize;
            let mut all_items: Vec<CodeSearchItem> = Vec::new();
            while cursor < total {
                let rp = repo_path.clone();
                let qt = query_text.clone();
                let (new_cursor, batch) = spawner
                    .spawn(move |cache, _ctx| {
                        cache.search_symbols_chunk(&rp, cursor, &qt, Duration::from_millis(5))
                    })
                    .await
                    .map_err(map_err)?;

                all_items.extend(batch);

                // Cache was invalidated or we reached the end
                if new_cursor == usize::MAX || new_cursor >= total {
                    break;
                }
                cursor = new_cursor;
            }

            // Finalize: sort/filter results (runs on background thread)
            if is_zero_state {
                Ok(finalize_zero_state(all_items, &git_changed_files))
            } else {
                Ok(finalize_query(all_items))
            }
        })
    }
}

#[cfg(not(target_family = "wasm"))]
pub fn code_data_source(cache: &CodeSymbolCache) -> CodeCursorDataSource {
    CodeCursorDataSource::new(cache.spawner())
}

/// Zero-state finalisation: prioritize symbols from git-changed files.
#[cfg(not(target_family = "wasm"))]
fn finalize_zero_state(
    items: Vec<CodeSearchItem>,
    git_changed_files: &HashSet<String>,
) -> Vec<QueryResult<AIContextMenuSearchableAction>> {
    let mut results: Vec<QueryResult<AIContextMenuSearchableAction>> = Vec::new();

    // First, add all symbols from git-changed files (they get priority)
    for item in &items {
        let file_path_str = item.code_symbol.file_path.to_string_lossy().to_string();
        if git_changed_files.contains(&file_path_str) {
            let search_item = CodeSearchItem {
                code_symbol: item.code_symbol.clone(),
                match_result: FuzzyMatchResult {
                    score: 10000,
                    matched_indices: vec![],
                },
            };
            results.push(QueryResult::from(search_item));
        }
    }

    // Then add remaining symbols up to MAX_RESULTS total
    for item in &items {
        let file_path_str = item.code_symbol.file_path.to_string_lossy().to_string();
        if !git_changed_files.contains(&file_path_str) && results.len() < MAX_RESULTS {
            let search_item = CodeSearchItem {
                code_symbol: item.code_symbol.clone(),
                match_result: FuzzyMatchResult {
                    score: 0,
                    matched_indices: vec![],
                },
            };
            results.push(QueryResult::from(search_item));
        }
    }

    results
}

/// Query finalisation: take top-k by fuzzy score.
#[cfg(not(target_family = "wasm"))]
fn finalize_query(items: Vec<CodeSearchItem>) -> Vec<QueryResult<AIContextMenuSearchableAction>> {
    items
        .into_iter()
        .k_largest_relaxed_by_key(MAX_RESULTS, |item| item.match_result.score)
        .map(QueryResult::from)
        .collect()
}

/// Matches a symbol name (including type prefix when present) and applies symbol-score weighting.
fn fuzzy_match_symbol_with_type(code_symbol: &CodeSymbol, query: &str) -> FuzzyMatchResult {
    if query.is_empty() {
        return FuzzyMatchResult::no_match();
    }

    let search_text = if let Some(type_prefix) = &code_symbol.symbol.type_prefix {
        format!("{}{}", type_prefix, code_symbol.symbol.name)
    } else {
        code_symbol.symbol.name.clone()
    };

    if let Some(mut match_result) =
        fuzzy_match::match_indices_case_insensitive_ignore_spaces(&search_text, query)
    {
        // Apply 3x weighted multiplier to make symbol scores competitive with file scores
        match_result.score *= 3;
        match_result
    } else {
        FuzzyMatchResult::no_match()
    }
}

#[cfg(test)]
#[path = "data_source_tests.rs"]
mod tests;
