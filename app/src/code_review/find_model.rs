use std::collections::HashMap;
use std::ops::Range;

use string_offset::CharOffset;
#[cfg(not(target_family = "wasm"))]
use warp_core::channel::ChannelState;
use warp_core::send_telemetry_from_ctx;
#[cfg(not(target_family = "wasm"))]
use warp_editor::content::find::SearchConfig;
#[cfg(not(target_family = "wasm"))]
use warp_editor::search::Searcher;
use warp_editor::search::{RestorableSearchResults, SelectedResult};
#[cfg(not(target_family = "wasm"))]
use warp_errors::report_error;
use warpui::r#async::SpawnedFutureHandle;
use warpui::{AppContext, Entity, EntityId, ModelContext, ViewHandle, WeakViewHandle};

use crate::code::local_code_editor::LocalCodeEditorView;
use crate::code_review::code_review_view::CodeReviewView;
use crate::code_review::telemetry_event::CodeReviewTelemetryEvent;
use crate::view_components::find::{FindDirection, FindEvent, FindModel};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMatch {
    pub editor_id: EntityId,
    pub start_offset: CharOffset,
    pub end_offset: CharOffset,
}

#[derive(Debug, Clone)]
pub struct MultiEditorSelectedResult {
    pub editor_id: EntityId,
    pub selected_result: SelectedResult,
}

#[cfg_attr(target_family = "wasm", expect(dead_code))]
pub struct MultiEditorSearchMatches {
    editor_id: EntityId,
    matches: Vec<SearchMatch>,
}

impl RestorableSearchResults for MultiEditorSearchMatches {
    fn valid_matches(&self) -> impl Iterator<Item = (usize, CharOffset)> {
        self.matches
            .iter()
            .enumerate()
            .filter(move |(_, m)| m.editor_id == self.editor_id)
            .map(|(index, m)| (index, m.start_offset))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedMatchInfo {
    pub editor_id: EntityId,
    pub index_within_editor: usize,
    pub start_offset: CharOffset,
    pub end_offset: CharOffset,
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub struct CodeReviewFindModel {
    query_text: String,
    case_sensitive: bool,
    regex: bool,
    results: Option<Vec<SearchMatch>>,
    selected_match: Option<MultiEditorSelectedResult>,
    search_handle: Option<SpawnedFutureHandle>,
    is_find_bar_open: bool,
    weak_view_handle: WeakViewHandle<CodeReviewView>,
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
impl CodeReviewFindModel {
    pub fn new(
        weak_view_handle: WeakViewHandle<CodeReviewView>,
        _ctx: &mut ModelContext<Self>,
    ) -> Self {
        Self {
            query_text: String::new(),
            case_sensitive: false,
            regex: false,
            results: None,
            search_handle: None,
            is_find_bar_open: false,
            selected_match: None,
            weak_view_handle,
        }
    }

    pub fn is_find_bar_open(&self) -> bool {
        self.is_find_bar_open
    }

    pub fn set_is_find_bar_open(&mut self, is_open: bool) {
        self.is_find_bar_open = is_open;
    }

    pub fn clear_results(&mut self) {
        self.results = None;
    }

    fn repo_is_local(&self, ctx: &AppContext) -> Option<bool> {
        self.weak_view_handle
            .upgrade(ctx)
            .and_then(|view| view.as_ref(ctx).repo_is_local())
    }

    pub fn update_query(
        &mut self,
        query: Option<String>,
        editor_handles: impl Iterator<Item = ViewHandle<LocalCodeEditorView>>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.query_text = query.unwrap_or_default();
        self.run_search(editor_handles, ctx);
    }

    pub fn set_case_sensitive(
        &mut self,
        case_sensitive: bool,
        editor_handles: impl Iterator<Item = ViewHandle<LocalCodeEditorView>>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.case_sensitive = case_sensitive;
        send_telemetry_from_ctx!(
            CodeReviewTelemetryEvent::FindBarModeChanged {
                is_local: self.repo_is_local(ctx),
                case_sensitive: self.case_sensitive,
                regex: self.regex,
            },
            ctx
        );
        self.run_search(editor_handles, ctx);
    }

    pub fn set_regex(
        &mut self,
        regex: bool,
        editor_handles: impl Iterator<Item = ViewHandle<LocalCodeEditorView>>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.regex = regex;
        send_telemetry_from_ctx!(
            CodeReviewTelemetryEvent::FindBarModeChanged {
                is_local: self.repo_is_local(ctx),
                case_sensitive: self.case_sensitive,
                regex: self.regex,
            },
            ctx
        );
        self.run_search(editor_handles, ctx);
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn focus_next_find_match(
        &mut self,
        direction: FindDirection,
        mut editor_handles: impl Iterator<Item = ViewHandle<LocalCodeEditorView>>,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(results) = &self.results else {
            return;
        };

        if results.is_empty() {
            return;
        }

        send_telemetry_from_ctx!(
            CodeReviewTelemetryEvent::FindNavigated {
                is_local: self.repo_is_local(ctx),
                direction,
            },
            ctx
        );

        let next_index = if let Some(selected) = &self.selected_match {
            match direction {
                FindDirection::Down => {
                    (selected.selected_result.current_index() + 1) % results.len()
                }
                FindDirection::Up => {
                    if selected.selected_result.current_index() == 0 {
                        results.len() - 1
                    } else {
                        selected.selected_result.current_index() - 1
                    }
                }
            }
        } else {
            0
        };

        let search_match = &results[next_index];
        if let Some(editor_handle) =
            editor_handles.find(|editor| editor.id() == search_match.editor_id)
        {
            let searcher = editor_handle
                .as_ref(ctx)
                .editor()
                .as_ref(ctx)
                .searcher
                .clone();
            let selected_result = searcher.update(ctx, |searcher, ctx| {
                searcher.select_match_at_offset(search_match.start_offset, next_index, ctx)
            });
            self.selected_match = Some(MultiEditorSelectedResult {
                editor_id: search_match.editor_id,
                selected_result,
            });
        }

        ctx.emit(FindEvent::UpdatedFocusedMatch);
    }

    pub fn selected_match_info(&self) -> Option<SelectedMatchInfo> {
        let results = self.results.as_ref()?;
        let selected = self.selected_match.as_ref()?;
        let selected_match_index = selected.selected_result.current_index();
        let selected_match = results.get(selected_match_index)?;

        let index_within_editor = results
            .iter()
            .take(selected_match_index)
            .filter(|m| m.editor_id == selected.editor_id)
            .count();

        Some(SelectedMatchInfo {
            editor_id: selected.editor_id,
            index_within_editor,
            start_offset: selected_match.start_offset,
            end_offset: selected_match.end_offset,
        })
    }

    #[cfg(not(target_family = "wasm"))]
    fn get_editor_searcher(
        &self,
        editor_id: EntityId,
        ctx: &AppContext,
    ) -> Option<warpui::ModelHandle<Searcher>> {
        let view = self.weak_view_handle.upgrade(ctx);
        if view.is_none() {
            if ChannelState::enable_debug_features() {
                report_error!(
                    "Failed to upgrade WeakViewHandle<CodeReviewView> in get_editor_searcher"
                );
            }
            return None;
        }

        let view = view.unwrap();
        let editor_handle = view
            .as_ref(ctx)
            .editor_handles()
            .find(|h| h.id() == editor_id);

        if editor_handle.is_none() {
            if ChannelState::enable_debug_features() {
                report_error!(
                    "Failed to find editor in CodeReviewView editor handles",
                    extra: { "editor_id" => ?editor_id }
                );
            }
            return None;
        }

        Some(
            editor_handle
                .unwrap()
                .as_ref(ctx)
                .editor()
                .as_ref(ctx)
                .searcher
                .clone(),
        )
    }

    #[cfg(not(target_family = "wasm"))]
    fn handle_run_search_result(
        &mut self,
        all_matches: Vec<SearchMatch>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Try to restore the previous selection if there was one
        if let Some(selected) = self.selected_match.take() {
            if let Some(searcher) = self.get_editor_searcher(selected.editor_id, ctx) {
                let candidates = MultiEditorSearchMatches {
                    editor_id: selected.editor_id,
                    matches: all_matches.clone(),
                };

                if let Some(restored_result) = searcher.update(ctx, |searcher, ctx| {
                    searcher.restore_selected_result(selected.selected_result, candidates, ctx)
                }) {
                    self.selected_match = Some(MultiEditorSelectedResult {
                        editor_id: selected.editor_id,
                        selected_result: restored_result,
                    });
                }
            }
        }

        // If we still don't have a selection and we have matches, select the first one
        if self.selected_match.is_none() && !all_matches.is_empty() {
            let first_match = &all_matches[0];
            if let Some(searcher) = self.get_editor_searcher(first_match.editor_id, ctx) {
                let selected_result = searcher.update(ctx, |searcher, ctx| {
                    searcher.select_match_at_offset(first_match.start_offset, 0, ctx)
                });
                self.selected_match = Some(MultiEditorSelectedResult {
                    editor_id: first_match.editor_id,
                    selected_result,
                });
            }
        }

        self.results = Some(all_matches);
        ctx.emit(FindEvent::RanFind);
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn run_search(
        &mut self,
        editor_handles: impl Iterator<Item = ViewHandle<LocalCodeEditorView>>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Abort any ongoing search since we're starting a new one
        if let Some(handle) = self.search_handle.take() {
            handle.abort();
        }

        if self.query_text.is_empty() {
            self.results = None;
            self.selected_match = None;
            ctx.emit(FindEvent::RanFind);
            return;
        }

        let mut search_futures = Vec::new();

        for local_editor_handle in editor_handles {
            let editor_model = local_editor_handle
                .as_ref(ctx)
                .editor()
                .as_ref(ctx)
                .model
                .as_ref(ctx);

            let hidden_lines = editor_model.hidden_ranges(ctx);
            let config = SearchConfig::new(&self.query_text)
                .with_regex(self.regex)
                .with_case_sensitive(self.case_sensitive)
                .with_skip_hidden(true)
                .with_hidden_ranges(&hidden_lines);

            match editor_model.run_search(&config, ctx) {
                Ok(search_future) => {
                    search_futures.push((local_editor_handle.id(), search_future));
                }
                Err(err) => {
                    // This should be a user regex error (BuildError from invalid syntax, etc)
                    log::info!(
                        "Invalid regex in search query: {} - {}",
                        self.query_text,
                        err
                    );
                    self.results = None;
                    self.selected_match = None;
                    ctx.emit(FindEvent::RanFind);
                    return;
                }
            }
        }

        self.search_handle = Some(ctx.spawn(
            async move {
                let mut all_matches = Vec::new();

                for (editor_id, search_future) in search_futures {
                    let results = search_future.await;
                    for match_result in results.matches {
                        all_matches.push(SearchMatch {
                            editor_id,
                            start_offset: match_result.start,
                            end_offset: match_result.end,
                        });
                    }
                }

                all_matches
            },
            |me, all_matches, ctx| me.handle_run_search_result(all_matches, ctx),
        ));
    }

    #[cfg(target_family = "wasm")]
    pub fn run_search(
        &mut self,
        _editor_handles: impl Iterator<Item = ViewHandle<LocalCodeEditorView>>,
        _ctx: &mut ModelContext<Self>,
    ) {
        unreachable!("Code review is not available on wasm")
    }

    pub fn matches_by_editor(&self) -> HashMap<EntityId, Vec<Range<CharOffset>>> {
        let mut matches_map: HashMap<EntityId, Vec<Range<CharOffset>>> = HashMap::new();

        if let Some(results) = &self.results {
            for search_match in results {
                matches_map
                    .entry(search_match.editor_id)
                    .or_default()
                    .push(search_match.start_offset..search_match.end_offset);
            }
        }

        matches_map
    }
}

impl FindModel for CodeReviewFindModel {
    fn focused_match_index(&self) -> Option<usize> {
        self.selected_match
            .as_ref()
            .map(|s| s.selected_result.current_index())
    }

    fn match_count(&self) -> usize {
        self.results.as_ref().map_or(0, |r| r.len())
    }

    fn default_find_direction(&self, _app: &AppContext) -> FindDirection {
        FindDirection::Down
    }
}

impl Entity for CodeReviewFindModel {
    type Event = FindEvent;
}

#[cfg(test)]
#[path = "find_model_tests.rs"]
mod tests;
