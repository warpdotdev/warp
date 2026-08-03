use std::ops::Range;

use itertools::Itertools;
use lazy_static::lazy_static;
use pathfinder_color::ColorU;
use string_offset::CharOffset;
use warp_core::ui::theme::Fill;
use warpui_core::r#async::SpawnedFutureHandle;
use warpui_core::{Entity, ModelContext, ModelHandle};

use crate::content::anchor::Anchor;
use crate::content::buffer::{Buffer, BufferEvent};
use crate::content::find::{Query, SearchConfig, SearchResults};
use crate::content::selection_model::BufferSelectionModel;
use crate::render::model::Decoration;

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;

pub struct Searcher {
    query_text: String,
    case_sensitive: bool,
    regex: bool,
    buffer: ModelHandle<Buffer>,
    selection_model: ModelHandle<BufferSelectionModel>,
    results: Option<SearchResults>,
    selected_result: Option<SelectedResult>,
    search_handle: Option<SpawnedFutureHandle>,
    auto_select_on_search: bool,
}

/// A selected search result.
#[derive(Debug, Clone)]
pub struct SelectedResult {
    /// The index of the selected result in the overall list of matches.
    current_index: usize,
    /// Handle to the start of the selected result, used to find it again when rerunning a search.
    start: Anchor,
}

impl SelectedResult {
    pub fn current_index(&self) -> usize {
        self.current_index
    }
}

pub trait RestorableSearchResults {
    fn valid_matches(&self) -> impl Iterator<Item = (usize, CharOffset)>;
}

#[derive(Debug, Clone, Copy)]
pub enum SearchEvent {
    /// Search results have updated.
    Updated,
    /// The search query is invalid (i.e. a mistyped regex).
    InvalidQuery,
    /// The selected search result changed.
    SelectedResultChanged,
}

lazy_static! {
    pub static ref MATCH_FILL: Fill = Fill::Solid(ColorU::new(255, 254, 61, 180));
    pub static ref SELECTED_MATCH_FILL: Fill = Fill::Solid(ColorU::new(238, 146, 59, 180));
}

impl Searcher {
    pub fn new(
        buffer: ModelHandle<Buffer>,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&buffer, Self::handle_buffer_event);
        Self {
            query_text: String::new(),
            case_sensitive: false,
            regex: false,
            buffer,
            selection_model,
            results: None,
            selected_result: None,
            search_handle: None,
            auto_select_on_search: false,
        }
    }
    /// Returns the index of the closest match at or after the cursor position.
    /// If there are no matches after the cursor, wraps around to the first match.
    pub fn next_match_index_from_cursor(&self, ctx: &ModelContext<Self>) -> Option<usize> {
        let results = self.results.as_ref()?;
        if results.matches.is_empty() {
            return None;
        }

        let cursor_position = self.selection_model.as_ref(ctx).first_selection_head();
        let final_cursor_position = self.buffer.as_ref(ctx).max_charoffset();

        results
            .matches
            .iter()
            .enumerate()
            .min_by_key(|(_, m)| {
                if m.start <= cursor_position {
                    // Wrap around: add final position to make it come after matches >= cursor_position
                    m.start + final_cursor_position
                } else {
                    // Normal distance for matches at or after cursor
                    m.start - cursor_position
                }
            })
            .map(|(index, _)| index)
    }

    /// Set the query text.
    pub fn set_query(&mut self, query: impl Into<String>, ctx: &mut ModelContext<Self>) {
        self.query_text = query.into();
        self.selected_result = None;
        self.run_search(ctx);
    }

    /// Whether or not there's a non-empty search query.
    pub fn has_query(&self) -> bool {
        !self.query_text.is_empty()
    }

    /// Set regex mode.
    pub fn set_regex(&mut self, regex: bool, ctx: &mut ModelContext<Self>) {
        self.regex = regex;
        self.selected_result = None;
        self.run_search(ctx);
    }

    /// Whether or not regex mode is on.
    pub fn is_regex(&self) -> bool {
        self.regex
    }

    /// Set case sensitivity.
    pub fn set_case_sensitive(&mut self, case_sensitive: bool, ctx: &mut ModelContext<Self>) {
        self.case_sensitive = case_sensitive;
        self.selected_result = None;
        self.run_search(ctx);
    }

    /// Whether or not the search is case-sensitive.
    pub fn is_case_sensitive(&self) -> bool {
        self.case_sensitive
    }

    pub fn set_auto_select(&mut self, should_auto_select: bool) {
        self.auto_select_on_search = should_auto_select;
    }

    /// The count of search results.
    pub fn match_count(&self) -> usize {
        self.results
            .as_ref()
            .map_or(0, |results| results.matches.len())
    }

    /// The index of the currently-selected match within the overall search results.
    pub fn selected_match(&self) -> Option<usize> {
        self.selected_result
            .as_ref()
            .map(|result| result.current_index)
    }

    /// The text range of the currently-selected match.
    pub fn selected_match_range(&self) -> Option<Range<CharOffset>> {
        let match_index = self.selected_result.as_ref()?.current_index;
        let match_result = self.results.as_ref()?.matches.get(match_index)?;
        Some(match_result.start..match_result.end)
    }

    pub fn select_next_from_cursor(&mut self, ctx: &mut ModelContext<Self>) {
        match &self.results {
            Some(results) if !results.matches.is_empty() => {
                if let Some(next_match_index) = self.next_match_index_from_cursor(ctx) {
                    self.select_internal(next_match_index, ctx);
                }
            }
            _ => self.clear_selected_result(ctx),
        }
    }

    pub fn select_prev_from_cursor(&mut self, ctx: &mut ModelContext<Self>) {
        match &self.results {
            Some(results) if !results.matches.is_empty() => {
                if let Some(next_match_index) = self.next_match_index_from_cursor(ctx) {
                    let prev_match_index = next_match_index
                        .checked_sub(1)
                        .unwrap_or_else(|| results.matches.len() - 1);
                    self.select_internal(prev_match_index, ctx);
                }
            }
            _ => self.clear_selected_result(ctx),
        }
    }

    pub fn select_next_result(&mut self, ctx: &mut ModelContext<Self>) {
        let result_count = match &self.results {
            Some(results) if !results.matches.is_empty() => results.matches.len(),
            _ => return,
        };

        match self.selected_result.as_ref() {
            Some(selected_result) => {
                let target_index = (selected_result.current_index + 1) % result_count;
                self.select_internal(target_index, ctx);
            }
            None => self.select_next_from_cursor(ctx),
        }
    }

    pub fn select_previous_result(&mut self, ctx: &mut ModelContext<Self>) {
        let result_count = match &self.results {
            Some(results) if !results.matches.is_empty() => results.matches.len(),
            _ => return,
        };

        match self.selected_result.as_ref() {
            Some(selected_result) => {
                let target_index = selected_result
                    .current_index
                    .checked_sub(1)
                    .unwrap_or(result_count - 1);
                self.select_internal(target_index, ctx);
            }
            None => self.select_prev_from_cursor(ctx),
        }
    }

    /// Make `index` the selected search result. This will panic if there are no search results or
    /// if `index` is out of bounds for the current result set.
    fn select_internal(&mut self, index: usize, ctx: &mut ModelContext<Self>) {
        let start_offset = self
            .results
            .as_ref()
            .expect("Must have search results")
            .matches[index]
            .start;
        self.selected_result = Some(self.select_match_at_offset(start_offset, index, ctx));
        ctx.emit(SearchEvent::SelectedResultChanged);
    }

    pub fn select_match_at_offset(
        &mut self,
        start_offset: CharOffset,
        index: usize,
        ctx: &mut ModelContext<Self>,
    ) -> SelectedResult {
        let new_anchor = self.selection_model.update(ctx, |selection_model, ctx| {
            selection_model.anchor(start_offset, ctx)
        });
        SelectedResult {
            current_index: index,
            start: new_anchor,
        }
    }

    fn build_query(&self, ctx: &mut ModelContext<Self>) -> anyhow::Result<Query> {
        let config = SearchConfig::new(&self.query_text)
            .with_regex(self.regex)
            .with_case_sensitive(self.case_sensitive);
        self.buffer.as_ref(ctx).prepare_search(&config)
    }

    fn handle_buffer_event(
        &mut self,
        _: ModelHandle<Buffer>,
        event: &BufferEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        if let BufferEvent::ContentChanged { .. } = event {
            self.run_search(ctx);
        }
    }

    #[cfg(test)]
    pub fn search_finished(
        &self,
        ctx: &mut warpui_core::AppContext,
    ) -> impl std::future::Future<Output = ()> + use<> {
        let maybe_search = self
            .search_handle
            .as_ref()
            .map(|handle| ctx.await_spawned_future(handle.future_id()));
        async move {
            if let Some(search) = maybe_search {
                search.await;
            }
        }
    }

    /// Clear the search results.
    pub fn reset_results(&mut self, ctx: &mut ModelContext<Self>) {
        self.selected_result = None;
        self.results = None;
        ctx.emit(SearchEvent::Updated);
    }

    /// Clear the currently selected result without affecting the search results.
    pub fn clear_selected_result(&mut self, ctx: &mut ModelContext<Self>) {
        self.selected_result = None;
        ctx.emit(SearchEvent::SelectedResultChanged);
    }

    fn run_search(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some(handle) = self.search_handle.take() {
            handle.abort();
        }

        if self.query_text.is_empty() {
            self.reset_results(ctx);
            return;
        }

        // TODO(ben): Cache and reuse the query.
        let search_future = match self.build_query(ctx) {
            Ok(query) => self.buffer.as_ref(ctx).search(query),
            Err(_) => {
                // If we detect an invalid query, clear all existing results until the next successful query.
                self.reset_results(ctx);
                ctx.emit(SearchEvent::InvalidQuery);
                return;
            }
        };

        self.search_handle =
            Some(
                ctx.spawn(search_future, |me, (_query, results), ctx| match results {
                    Ok(results) => {
                        // If a result is currently selected, preserves it between searches
                        if let Some(selected_result) = me.selected_result.take() {
                            me.selected_result =
                                me.restore_selected_result(selected_result, &results, ctx);
                        }

                        me.results = Some(results);
                        if me.auto_select_on_search {
                            me.select_next_from_cursor(ctx);
                        }

                        ctx.emit(SearchEvent::Updated);
                    }
                    Err(err) => {
                        log::warn!("Search failed: {err:?}");
                        // If the search fails, clear all existing results until the next successful query.
                        me.reset_results(ctx);
                        ctx.emit(SearchEvent::Updated);
                    }
                }),
            );
    }

    pub fn restore_selected_result(
        &self,
        prev_selected_result: SelectedResult,
        matches: impl RestorableSearchResults,
        ctx: &mut ModelContext<Self>,
    ) -> Option<SelectedResult> {
        if let Some(start_offset) = self
            .selection_model
            .as_ref(ctx)
            .resolve_anchor(&prev_selected_result.start)
        {
            let candidates = matches.valid_matches().collect_vec();
            if let Ok(index) = candidates.binary_search_by_key(&start_offset, |(_, offset)| *offset)
            {
                return Some(SelectedResult {
                    current_index: candidates[index].0,
                    start: prev_selected_result.start,
                });
            }
        }
        None
    }

    /// Builds text decorations to highlight search results.
    pub fn result_decorations(&self) -> Vec<Decoration> {
        let matches = match &self.results {
            Some(results) => &results.matches,
            None => return Vec::new(),
        };

        let selected_match = self.selected_match();

        matches
            .iter()
            .enumerate()
            .map(|(index, m)| {
                let fill = if Some(index) == selected_match {
                    *SELECTED_MATCH_FILL
                } else {
                    *MATCH_FILL
                };

                // TODO(CLD-558): This matches how we shift the selection by 1.
                Decoration::new(m.start - 1, m.end - 1).with_background(fill)
            })
            .collect()
    }

    pub fn results(&self) -> Option<&SearchResults> {
        self.results.as_ref()
    }
}

impl Entity for Searcher {
    type Event = SearchEvent;
}
