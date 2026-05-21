use itertools::{Either, Itertools};
use warp_editor::editor::NavigationKey;
use warpui::elements::ConstrainedBox;
use warpui::FocusContext;

use std::collections::HashSet;

use warpui::fonts::FamilyId;
use warpui::{
    accessibility::{AccessibilityContent, WarpA11yRole},
    elements::{Clipped, Container, CrossAxisAlignment, Flex, ParentElement, Shrinkable, Text},
    fonts::{Properties, Style, Weight},
    presenter::ChildView,
    Action, AppContext, Element, Entity, ModelContext, ModelHandle, SingletonEntity,
    TypedActionView, View, ViewContext, ViewHandle,
};

use crate::editor::AutosuggestionType;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::SearchMixer;
use crate::search::result_renderer::QueryResultIndex;
use crate::search::result_renderer::QueryResultRenderer;
use crate::search::QueryFilter;

use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
use crate::{
    appearance::Appearance,
    editor::{
        AutosuggestionLocation, EditorView, Event as EditorEvent,
        PlainTextEditorViewAction as EditorAction, PropagateAndNoOpNavigationKeys,
        SingleLineEditorOptions,
    },
};

use super::mixer::SearchMixerEvent;

/// Function to create a [`QueryResultRenderer`] from a [`QueryResult`]. Used to specify the styles
/// and click-action of a a rendered query result.
pub type CreateQueryResultRendererFn<T> =
    fn(QueryResultIndex, QueryResult<T>) -> QueryResultRenderer<T>;

enum SearchResultLimit {
    /// Search results are unbounded.
    Unbounded,
    /// Search results are bounded. Only the first `max_results` number of results will be returned.
    Bounded { max_results: usize },
}

#[derive(Debug)]
enum MoveDirection {
    Up,
    Down,
}

impl MoveDirection {
    fn move_in_direction(&self, current_index: usize, len: usize) -> usize {
        match self {
            MoveDirection::Up => {
                // wrap when we hit the end of the list
                if current_index == 0 {
                    len.saturating_sub(1)
                } else {
                    current_index.saturating_sub(1)
                }
            }
            MoveDirection::Down => {
                // wrap when we hit the start of the list
                if current_index == len.saturating_sub(1) {
                    0
                } else {
                    current_index.saturating_add(1)
                }
            }
        }
    }
}

/// View state for the search bar.
pub struct SearchBarState<T: Action + Clone> {
    selected_index: Option<usize>,
    /// The active query filter, if any.
    query_filter: Option<QueryFilter>,
    /// The filter atom text to be rendered in the Search input, representing the currently applied
    /// filter.
    filter_atom_text: Option<&'static str>,
    query_result_renderers: Option<Vec<QueryResultRenderer<T>>>,
    /// Ordering of how results should be displayed. Used to determine which search result should
    /// be considered selected when the user presses certain navigation keys such as up/down.
    ordering: SearchResultOrdering,
    /// Sets the maximum number of results returned by the mixer. Results are unbounded if `None`.
    max_results: SearchResultLimit,
    /// Whether to run query when buffer is empty. This is useful for search panels that don't have
    /// a dedicated zero state.
    run_query_on_buffer_empty: bool,

    /// The offset to apply to the initial selection when the search bar is opened.
    offset_initial_selection_by: isize,
}

/// Ordering of search results.
pub enum SearchResultOrdering {
    /// Results are ordered top-down, with the highest ranking result ordered first.
    TopDown,
    /// Results are ordered bottom-up, with the highest ranked result ordered last.
    BottomUp,
}

/// View that renders a search bar and runs queries through a [`SearchMixer`] as the user
/// types.
///
/// [`QueryFilter`]s are rendered as autosuggestion "hints" within the search bar. When applied,
/// they are rendered as filters that prepend the search bar.
///
/// Search results are stored as [`QueryResultRenderer`]s, which parents views can use to
/// render search results as they choose.
pub struct SearchBar<T: Action + Clone> {
    editor_handle: ViewHandle<EditorView>,
    /// State for the [`SearchBar`]. Stored in a separate [`ModelHandle`] since this state is shared
    /// between the search bar and any parent view that may render the query results.
    state: ModelHandle<SearchBarState<T>>,
    mixer: ModelHandle<SearchMixer<T>>,
    /// The placeholder text that is rendered in the search bar when no query has been run or
    /// filters have been applied.
    placeholder_text: &'static str,
    create_query_result_renderer_fn: CreateQueryResultRendererFn<T>,
    /// Font family to use when rendering the editor and query filters. If `None` the monospace font
    /// family is used.
    font_family_override: Option<FamilyId>,
}

#[derive(Debug)]
pub enum SearchBarEvent<T: Action + Clone> {
    /// The search view should be closed.
    Close,
    /// The search query buffer was explicitly cleared, via something like ctrl-c.
    BufferCleared { buffer_len: usize },
    /// The user accepted a result by hitting enter.
    ResultAccepted { index: usize, action: T },
    /// The selected result was changed.
    ResultSelected { index: usize },
    /// The active query filter changed.
    QueryFilterChanged { new_filter: Option<QueryFilter> },
    /// The user updated the selection while the zero state was shown. The [`SearchBar`] does not
    /// render zero-state. Callers can update the zero state, if needed, upon receiving this event.
    SelectionUpdateInZeroState { selection_update: SelectionUpdate },
    /// The user pressed enter while the zero state was state. If `cmd_enter` is true, the user
    /// pressed `cmd-enter`.
    EnterInZeroState { modified_enter: bool },
}

impl<T: Action + Clone> SearchBarState<T> {
    pub fn new(ordering: SearchResultOrdering) -> Self {
        Self {
            selected_index: None,
            query_filter: None,
            filter_atom_text: None,
            query_result_renderers: None,
            ordering,
            max_results: SearchResultLimit::Unbounded,
            run_query_on_buffer_empty: false,
            offset_initial_selection_by: 0,
        }
    }

    /// Bounds the number of results returned by the mixer to `max_results`.
    pub fn with_max_results(mut self, max_results: usize) -> Self {
        self.max_results = SearchResultLimit::Bounded { max_results };
        self
    }

    pub fn run_query_on_buffer_empty(mut self) -> Self {
        self.run_query_on_buffer_empty = true;
        self
    }

    pub fn offset_initial_selection_by(&mut self, offset: isize) {
        self.offset_initial_selection_by = offset;
    }

    /// Helper function to find the next interactable item (i.e. non-separator) in the given direction.
    /// Returns the index of the next interactable item, or None if no interactable items exist.
    fn find_next_interactable_index(
        &self,
        start_index: usize,
        direction: MoveDirection,
        query_result_renderers: &[QueryResultRenderer<T>],
    ) -> Option<usize> {
        let len = query_result_renderers.len();

        let mut attempts = 0;
        let mut current_index = direction.move_in_direction(start_index, len);

        // Keep searching until we find an interactable item or we've checked all items
        while attempts < len {
            if let Some(renderer) = query_result_renderers.get(current_index) {
                if !renderer.search_result.is_static_separator() {
                    return Some(current_index);
                }
            }

            current_index = direction.move_in_direction(current_index, len);
            attempts += 1;
        }

        // No interactable items found
        None
    }

    /// Updates the search results view to reflect the selection of
    /// `self.query_result_renderers[self.selected_index + offset]`.
    pub fn handle_selection_update(
        &mut self,
        selection_update: SelectionUpdate,
        should_show_zero_state: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        // Even if the zero state should be shown, still update the internal selected index of the
        // search bar. We do this for two reasons:
        // 1) Consumers of the search bar aren't _required_ to show a zero state, in which case
        // there should still be a selected index.
        // 2) We are resilient to any race conditions where selection update may be called before
        // `should_show_zero_state` is actually set to `false`, which would then cause no index to
        // be selected.
        if should_show_zero_state {
            ctx.emit(SearchBarEvent::SelectionUpdateInZeroState { selection_update });
        }

        match (
            selection_update,
            self.selected_index,
            self.query_result_renderers.as_ref(),
        ) {
            (SelectionUpdate::Up, Some(current_index), Some(query_result_renderers)) => {
                if let Some(new_index) = self.find_next_interactable_index(
                    current_index,
                    MoveDirection::Up,
                    query_result_renderers,
                ) {
                    self.selected_index = Some(new_index);
                    ctx.emit(SearchBarEvent::ResultSelected { index: new_index });
                    ctx.notify();
                }
                // If no interactable item found, keep current selection
            }
            (SelectionUpdate::Down, Some(current_index), Some(query_result_renderers)) => {
                if let Some(new_index) = self.find_next_interactable_index(
                    current_index,
                    MoveDirection::Down,
                    query_result_renderers,
                ) {
                    self.selected_index = Some(new_index);
                    ctx.emit(SearchBarEvent::ResultSelected { index: new_index });
                    ctx.notify();
                }
                // If no interactable item found, keep current selection
            }
            (SelectionUpdate::Bottom, _, Some(query_result_renderers)) => {
                if !query_result_renderers.is_empty() {
                    // Adjust the index, wrapping around if necessary.
                    let len = query_result_renderers.len() as isize;
                    let new_index =
                        (len - 1 + self.offset_initial_selection_by).rem_euclid(len) as usize;
                    if self.selected_index != Some(new_index) {
                        self.selected_index = Some(new_index);
                        ctx.emit(SearchBarEvent::ResultSelected { index: new_index });
                        ctx.notify();
                    }
                }
            }
            (SelectionUpdate::Top, _, Some(query_result_renderers)) => {
                if !query_result_renderers.is_empty() {
                    // Adjust the index, wrapping around if necessary.
                    let new_index = self
                        .offset_initial_selection_by
                        .rem_euclid(query_result_renderers.len() as isize)
                        as usize;
                    if self.selected_index != Some(new_index) {
                        self.selected_index = Some(new_index);
                        ctx.emit(SearchBarEvent::ResultSelected { index: new_index });
                        ctx.notify();
                    }
                }
            }
            (SelectionUpdate::Clear, _, _) => {
                self.selected_index = None;
                ctx.notify();
            }
            _ => {}
        }
    }

    /// Returns an `Option` containing the currently selected result, if there is one.
    pub fn selected_result(&self) -> Option<&QueryResult<T>> {
        self.selected_result_renderer()
            .map(|renderer| &renderer.search_result)
    }

    /// Returns an `Option` containing the [`QueryResultRenderer`] that owns the currently selected
    /// result, if any.
    pub fn selected_result_renderer(&self) -> Option<&QueryResultRenderer<T>> {
        let query_result_renderers = self.query_result_renderers.as_ref()?;
        let selected_index = self.selected_index?;
        query_result_renderers.get(selected_index)
    }

    /// Returns the current set of [`QueryResultRenderer`]s based on the current search query. If
    /// no query has been performed, `None` is returned.
    pub fn query_result_renderers(&self) -> Option<&Vec<QueryResultRenderer<T>>> {
        self.query_result_renderers.as_ref()
    }

    /// Returns the active [`QueryFilter`], if any.
    pub fn active_query_filter(&self) -> Option<QueryFilter> {
        self.query_filter
    }

    /// Returns the index of currently selected search result.
    pub fn selected_index(&self) -> Option<usize> {
        self.selected_index
    }
}

impl<T: Action + Clone> Entity for SearchBarState<T> {
    type Event = SearchBarEvent<T>;
}

#[derive(Copy, Clone, Debug)]
pub enum SelectionUpdate {
    /// Select the result above the currently selected result.
    Up,

    /// Select the result below the currently selected result.
    Down,

    /// Select the result at the bottom of the list.
    Bottom,

    /// Select the result at the top of the list.
    Top,

    /// Clear the selection (e.g. set the current selection to None).
    Clear,
}

impl<T: Action + Clone> SearchBar<T> {
    pub fn new(
        mixer: ModelHandle<SearchMixer<T>>,
        state: ModelHandle<SearchBarState<T>>,
        placeholder_text: &'static str,
        create_query_result_renderer_fn: CreateQueryResultRendererFn<T>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let editor_handle = ctx.add_typed_action_view(|ctx| {
            let options = SingleLineEditorOptions {
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            EditorView::single_line(options, ctx)
        });
        ctx.subscribe_to_view(&editor_handle, |me, _handle, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        ctx.observe(&state, |_, _, ctx| {
            ctx.notify();
        });

        ctx.subscribe_to_model(&mixer, |me, _handle, event, ctx| {
            me.handle_mixer_event(event, ctx);
        });

        let me = Self {
            editor_handle,
            mixer,
            placeholder_text,
            state,
            create_query_result_renderer_fn,
            font_family_override: None,
        };

        me.update_placeholder_text(ctx);
        me
    }

    /// Sets the font that the search bar editor and query filter should use. If unspecified the
    /// monospace font family is used.
    pub fn with_font_family(mut self, font_family: FamilyId, ctx: &mut ViewContext<Self>) -> Self {
        self.editor_handle
            .update(ctx, |editor, ctx| editor.set_font_family(font_family, ctx));
        self.font_family_override = Some(font_family);
        self
    }

    fn handle_mixer_event(&mut self, event: &SearchMixerEvent, ctx: &mut ViewContext<Self>) {
        match event {
            SearchMixerEvent::ResultsChanged => {
                self.on_mixer_results_changed(ctx);
            }
        }
    }

    pub fn up(&mut self, ctx: &mut ViewContext<Self>) {
        self.editor_handle.update(ctx, |editor, ctx| {
            editor.up(ctx);
        });
    }

    pub fn down(&mut self, ctx: &mut ViewContext<Self>) {
        self.editor_handle.update(ctx, |editor, ctx| {
            editor.down(ctx);
        });
    }

    pub fn select_current_item(&mut self, ctx: &mut ViewContext<Self>) {
        self.handle_editor_event(&EditorEvent::Enter, ctx);
    }

    /// Handles an `EditorEvent` emitted by the query editor.
    fn handle_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Edited(_) => {
                self.handle_editor_text_update(ctx);
            }
            // TODO(vorporeal): Implement real support for placeholder text.
            EditorEvent::AutosuggestionAccepted { .. } => {
                // Autosuggestions can be accepted by pressing right arrow when they are
                // visible.  In the command search view, they're only visible when the
                // buffer is empty, so if the user accepts one, immediately re-clear the
                // buffer (to effectively revert the event).
                if self.should_show_zero_state(ctx) {
                    self.editor_handle.update(ctx, |editor, ctx| {
                        editor.clear_buffer_and_reset_undo_stack(ctx);
                    });
                }
            }
            EditorEvent::Navigate(NavigationKey::Up) => {
                self.handle_selection_update(SelectionUpdate::Up, ctx);
            }
            EditorEvent::Navigate(NavigationKey::Down) => {
                self.handle_selection_update(SelectionUpdate::Down, ctx);
            }
            EditorEvent::Enter | EditorEvent::CmdEnter | EditorEvent::ShiftEnter => {
                // If the results are loading, enter should be a no-op.
                if self.mixer.as_ref(ctx).is_loading() {
                    return;
                }

                let modified_enter = match event {
                    EditorEvent::CmdEnter if cfg!(target_os = "macos") => true,
                    EditorEvent::ShiftEnter if cfg!(linux_or_windows) => true,
                    _ => false,
                };

                if !self.should_run_query(ctx) {
                    ctx.emit(SearchBarEvent::EnterInZeroState { modified_enter });
                    return;
                }

                if let (Some(selected_result), Some(result_index)) = (
                    self.state.as_ref(ctx).selected_result(),
                    self.state.as_ref(ctx).selected_index,
                ) {
                    let result_action = if modified_enter {
                        selected_result.execute_result()
                    } else {
                        selected_result.accept_result()
                    };
                    self.handle_result_accepted(result_index, result_action, ctx);
                }
            }
            EditorEvent::Escape => self.close(ctx),
            EditorEvent::CtrlC {
                cleared_buffer_len: buffer_len,
            } => self.buffer_cleared(ctx, *buffer_len),
            EditorEvent::BackspaceOnEmptyBuffer => {
                self.set_query_filter(None, ctx);
            }
            EditorEvent::Navigate(NavigationKey::Tab) => {
                self.handle_editor_tab(ctx);
            }
            _ => {}
        }
    }

    fn close(&self, ctx: &mut ViewContext<Self>) {
        ctx.emit(SearchBarEvent::Close);
    }

    fn buffer_cleared(&self, ctx: &mut ViewContext<Self>, buffer_len: usize) {
        ctx.emit(SearchBarEvent::BufferCleared { buffer_len });
    }

    /// Returns the current search query in the editor.
    pub fn query(&self, app: &AppContext) -> String {
        self.editor_handle.as_ref(app).buffer_text(app)
    }

    /// Returns the mixer handle.
    pub fn mixer(&self) -> ModelHandle<SearchMixer<T>> {
        self.mixer.clone()
    }

    pub fn insert_query_text(&mut self, query: &str, ctx: &mut ViewContext<Self>) {
        self.editor_handle.update(ctx, |editor, ctx| {
            editor.user_initiated_insert(query, EditorAction::SystemInsert, ctx);
        })
    }

    /// Sets the search query, replacing any existing text in the editor and triggering search.
    pub fn set_query(&mut self, query: String, ctx: &mut ViewContext<Self>) {
        self.editor_handle.update(ctx, |editor, ctx| {
            editor.set_buffer_text(&query, ctx);
        });
        self.handle_editor_text_update(ctx);
    }

    /// Resets the [`SearchBar`] and its state.
    pub fn reset(
        &mut self,
        initial_query: Option<String>,
        query_filter: Option<QueryFilter>,
        ordering: SearchResultOrdering,
        ctx: &mut ViewContext<Self>,
    ) {
        self.mixer.update(ctx, |mixer, ctx| {
            mixer.reset_results(ctx);
        });

        self.state.update(ctx, |state, ctx| {
            state.ordering = ordering;

            state.query_filter = query_filter;

            ctx.notify();
        });

        self.editor_handle
            .update(ctx, |editor: &mut EditorView, ctx| {
                editor.clear_buffer_and_reset_undo_stack(ctx);
                if let Some(initial_query) = initial_query {
                    editor.user_initiated_insert(
                        initial_query.as_str(),
                        EditorAction::SystemInsert,
                        ctx,
                    );
                }
            });

        // Reset the query filter state before processing any initial query text,
        // as it may contain a filter atom that should be immediately applied.
        let filter_and_atom_text = self
            .state
            .as_ref(ctx)
            .query_filter
            .map(|filter| (filter, filter.filter_atom().primary_text));
        self.set_query_filter(filter_and_atom_text, ctx);
        self.handle_editor_text_update(ctx);
        ctx.notify();
    }

    /// Handles tab keypresses emitted by the editor, which may result in the selection of a
    /// filter.
    ///
    /// If the user has begun typing a filter's atom text, tab will complete the filter atom text
    /// and set the corresponding filter as the active filter.
    fn handle_editor_tab(&mut self, ctx: &mut ViewContext<Self>) {
        let buffer_text = self
            .editor_handle
            .read(ctx, |editor, ctx| editor.buffer_text(ctx));

        if !buffer_text.is_empty() && self.state.as_ref(ctx).query_filter.is_none() {
            let registered_filters = self.mixer.as_ref(ctx).registered_filters().collect_vec();
            for filter in registered_filters {
                if filter.filter_atom().primary_text.starts_with(&buffer_text) {
                    self.editor_handle
                        .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
                    self.set_query_filter(Some((filter, filter.filter_atom().primary_text)), ctx);
                }
            }
        }
    }

    /// Emits a `ResultAccepted` event with the passed `result_action` to parent views.
    fn handle_result_accepted(
        &self,
        result_index: usize,
        result_action: T,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.emit(SearchBarEvent::ResultAccepted {
            index: result_index,
            action: result_action,
        })
    }

    /// Updates the active filter and re-runs the query if the current search state warrants it.
    ///
    /// Empty zero-state buffers skip querying unless this search bar has opted into
    /// `run_query_on_buffer_empty`.
    pub fn set_query_filter(
        &mut self,
        filter_and_atom_text: Option<(QueryFilter, &'static str)>,
        ctx: &mut ViewContext<Self>,
    ) {
        let new_filter = filter_and_atom_text.map(|(filter, _)| filter);
        let new_filter_atom_text = filter_and_atom_text.map(|(_, atom_text)| atom_text);
        // NOTE: This event is deferred. When this method starts a query below, handlers
        // receive it after the async search has already started. Handlers must not reset or
        // modify the mixer, as doing so would abort the in-flight query without re-running it.
        ctx.emit(SearchBarEvent::QueryFilterChanged { new_filter });

        self.state.update(ctx, |state, ctx| {
            state.query_filter = new_filter;
            state.filter_atom_text = new_filter_atom_text;
            ctx.notify();
        });

        if self.should_run_query(ctx) {
            self.run_query_internal(ctx);
        }

        // Update the editor placeholder text if necessary.
        self.update_placeholder_text(ctx);

        self.update_filter_autosuggestion_text(ctx);

        // Make sure the view gets re-rendered.
        ctx.notify();
    }

    /// Runs a search query using the editor's current contents as the query string with filters
    /// applied.
    fn run_query_internal(&mut self, ctx: &mut ViewContext<Self>) {
        let current_editor_text = self
            .editor_handle
            .read(ctx, |editor, ctx| editor.buffer_text(ctx));
        let filters: HashSet<QueryFilter> = match &self.state.as_ref(ctx).query_filter {
            Some(filter) => HashSet::from_iter([*filter]),
            None => HashSet::new(),
        };

        self.mixer.update(ctx, |mixer, ctx| {
            mixer.run_query(
                Query {
                    text: current_editor_text.trim().to_string(),
                    filters,
                },
                ctx,
            );
        });
    }

    pub fn run_query(&mut self, ctx: &mut ViewContext<Self>) {
        if self.should_run_query(ctx) {
            self.run_query_internal(ctx);
        }
    }

    fn on_mixer_results_changed(&mut self, ctx: &mut ViewContext<Self>) {
        let results = self.mixer.read(ctx, |mixer, _ctx| mixer.results().clone());
        let create_query_result_renderer = self.create_query_result_renderer_fn;

        let (selection_start, query_results) = match self.state.as_ref(ctx).ordering {
            SearchResultOrdering::BottomUp => {
                (SelectionUpdate::Bottom, Either::Left(results.into_iter()))
            }
            SearchResultOrdering::TopDown => (
                SelectionUpdate::Top,
                Either::Right(results.into_iter().rev()),
            ),
        };

        let max_results = match self.state.as_ref(ctx).max_results {
            SearchResultLimit::Unbounded => usize::MAX,
            SearchResultLimit::Bounded { max_results } => max_results,
        };

        let query_result_renderers = query_results
            .take(max_results)
            .enumerate()
            .map(|(index, result)| create_query_result_renderer(index, result))
            .collect_vec();

        self.state.update(ctx, |state, ctx| {
            state.query_result_renderers = Some(query_result_renderers);
            ctx.notify();
        });

        self.handle_selection_update(selection_start, ctx);

        // Re-render the view with the updated results.
        ctx.notify();
    }

    fn emit_accessibility_content(&self, ctx: &mut ViewContext<Self>) {
        if let Some(loading_filters) = self.mixer.as_ref(ctx).loading_query_filters() {
            for loading_filter in loading_filters.into_iter() {
                ctx.emit_a11y_content(AccessibilityContent::new_without_help(
                    format!("Loading {} suggestions", loading_filter.display_name()),
                    WarpA11yRole::MenuItemRole,
                ));
            }

            return;
        }

        if let Some((.., data_source_err)) = self.mixer.as_ref(ctx).first_data_source_error() {
            ctx.emit_a11y_content(AccessibilityContent::new(
                "Error finding results",
                data_source_err.user_facing_error(),
                WarpA11yRole::MenuItemRole,
            ));
            return;
        }

        if let Some(selected_result) = self.state.as_ref(ctx).selected_result() {
            let a11y_content_text = format!("Selected {}", selected_result.accessibility_label(),);
            let a11y_content = match selected_result.accessibility_help_message() {
                None => AccessibilityContent::new_without_help(
                    a11y_content_text,
                    WarpA11yRole::MenuItemRole,
                ),
                Some(help_message) => AccessibilityContent::new(
                    a11y_content_text,
                    help_message,
                    WarpA11yRole::MenuItemRole,
                ),
            };
            ctx.emit_a11y_content(a11y_content);
        }
    }
    /// Updates state when the editor's buffer has been changed or modified.
    ///
    /// If there is a filter atom present in the buffer, it is parsed out as an active filter and
    /// the filter atom text is removed from the actual editor contents. Then a search query is run
    /// based on the (potentially modified) editor's buffer contents.
    fn handle_editor_text_update(&mut self, ctx: &mut ViewContext<Self>) {
        let current_buffer_text = self
            .editor_handle
            .read(ctx, |editor, ctx| editor.buffer_text(ctx));

        if self.should_show_zero_state(ctx) {
            self.handle_selection_update(SelectionUpdate::Clear, ctx);
            if self.should_run_query(ctx) {
                self.run_query_internal(ctx);
            }
        } else {
            if self.state.as_ref(ctx).query_filter.is_none() {
                let registered_filters = self.mixer.as_ref(ctx).registered_filters().collect_vec();
                for filter in registered_filters {
                    if let Some(matched_filter_atom) =
                        filter.filter_atom().query_match(&current_buffer_text)
                    {
                        self.set_query_filter(Some((filter, matched_filter_atom)), ctx);
                        self.editor_handle.update(ctx, |editor, ctx| {
                            // The 'filter atom text' will be rendered separately (not as text
                            // within the editor), so remove it from the editor contents.
                            editor.clear_buffer(ctx);
                            editor.user_initiated_insert(
                                current_buffer_text[matched_filter_atom.len()..].trim(),
                                EditorAction::SystemInsert,
                                ctx,
                            );
                        });
                        break;
                    }
                }
            }
            self.run_query_internal(ctx);
        }
        self.update_placeholder_text(ctx);
        self.update_filter_autosuggestion_text(ctx);
    }

    /// Updates the content and visibility of the input placeholder text depending on the state of
    /// the editor.
    fn update_placeholder_text(&self, ctx: &mut ViewContext<Self>) {
        self.editor_handle.update(ctx, |editor, ctx| {
            if editor.is_empty(ctx) {
                // Set the appropriate placeholder text if the editor buffer is empty.
                match self.state.as_ref(ctx).query_filter {
                    Some(filter) => {
                        editor.set_placeholder_text(filter.placeholder_text(), ctx);
                    }
                    None => {
                        editor.set_placeholder_text(self.placeholder_text, ctx);
                    }
                }
            }
        });
    }

    /// Updates the search results view to reflect the selection of
    /// self.state.as_ref(ctx).query_result_renderers[self.state.as_ref(ctx).selected_index + offset].
    pub(crate) fn handle_selection_update(
        &mut self,
        selection_update: SelectionUpdate,
        ctx: &mut ViewContext<Self>,
    ) {
        let should_show_zero_state = self.should_show_zero_state(ctx);
        self.state.update(ctx, |state, ctx| {
            state.handle_selection_update(selection_update, should_show_zero_state, ctx);
        });

        self.emit_accessibility_content(ctx);
    }

    pub fn should_show_zero_state(&self, app: &AppContext) -> bool {
        self.editor_handle.as_ref(app).is_empty(app)
            && self.state.as_ref(app).query_filter.is_none()
    }

    fn should_run_query(&self, app: &AppContext) -> bool {
        !self.should_show_zero_state(app) || self.state.as_ref(app).run_query_on_buffer_empty
    }

    /// Updates the state and visibility of filter atom autosuggestion text in the editor.
    ///
    /// If the editor is non-empty and its contents are the prefix of a filter atom, display the
    /// rest of the filter atom text as an autosuggestion. Otherwise, clear the autosuggestion
    /// text.
    ///
    /// For example, if the user types 'his' into the buffer, show the remaining 'tory:'
    /// (completing the 'history:' atom text) as an autosuggestion.
    fn update_filter_autosuggestion_text(&self, ctx: &mut ViewContext<Self>) {
        match self.state.as_ref(ctx).query_filter {
            Some(_) => {
                // If there is an active filter, there should never be filter autosuggestion text
                // (the user has already applied a filter and can't apply another one).
                if self.editor_handle.as_ref(ctx).active_autosuggestion() {
                    self.editor_handle.update(ctx, |editor, ctx| {
                        editor.clear_autosuggestion(ctx);
                    });
                }
            }
            None => {
                self.editor_handle.update(ctx, |editor, ctx| {
                    if !editor.buffer_text(ctx).is_empty() {
                        let filters = self.mixer.as_ref(ctx).registered_filters().collect_vec();
                        let editor_text = editor.buffer_text(ctx);
                        let suggested_filter = filters.into_iter().find(|filter| {
                            filter.filter_atom().primary_text.starts_with(&editor_text)
                        });

                        if let Some(filter) = suggested_filter {
                            let suggestion = filter.filter_atom().primary_text[editor_text.len()..]
                                .to_owned()
                                + " ";
                            editor.set_autosuggestion(
                                suggestion,
                                AutosuggestionLocation::EndOfBuffer,
                                AutosuggestionType::Command {
                                    was_intelligent_autosuggestion: false,
                                },
                                ctx,
                            );
                        } else {
                            editor.clear_autosuggestion(ctx);
                        }
                    }
                });
            }
        }
    }
}

impl<T: Action + Clone> Entity for SearchBar<T> {
    type Event = SearchBarEvent<T>;
}

impl<T: Action + Clone> TypedActionView for SearchBar<T> {
    type Action = ();
}

impl<T: Action + Clone> View for SearchBar<T> {
    fn ui_name() -> &'static str {
        "SearchBar"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.editor_handle);
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut input_contents = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        if let Some(filter_atom_text) = self.state.as_ref(app).filter_atom_text {
            let font_family = self
                .font_family_override
                .unwrap_or_else(|| appearance.monospace_font_family());
            input_contents.add_child(
                Container::new(
                    Text::new_inline(
                        filter_atom_text,
                        font_family,
                        appearance.monospace_font_size(),
                    )
                    .with_color(theme.main_text_color(theme.surface_2()).into_solid())
                    .with_style(
                        Properties::default()
                            .style(Style::Italic)
                            .weight(Weight::Semibold),
                    )
                    .finish(),
                )
                .with_margin_right(8.)
                .finish(),
            );
        } else {
            let size = appearance.monospace_font_size();
            let magnifying_glass = Container::new(
                ConstrainedBox::new(
                    Icon::Search
                        .to_warpui_icon(blended_colors::text_sub(theme, theme.surface_2()).into())
                        .finish(),
                )
                .with_height(size)
                .with_width(size)
                .finish(),
            )
            .with_margin_right(12.)
            .finish();
            input_contents.add_child(magnifying_glass);
        }
        input_contents.add_child(
            Shrinkable::new(
                1.,
                Clipped::new(ChildView::new(&self.editor_handle).finish()).finish(),
            )
            .finish(),
        );
        input_contents.finish()
    }
}

#[cfg(feature = "integration_tests")]
impl<T: Action + Clone> SearchBar<T> {
    pub fn active_query_filter(&self, app: &AppContext) -> Option<QueryFilter> {
        self.state.as_ref(app).active_query_filter()
    }
}
