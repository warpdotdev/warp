//! Searchable `@` context menu state for the TUI input.

use std::ops::Range;

use string_offset::CharOffset;
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::search::mixer::SearchMixerEvent;
use warp::settings::InputSettings;
use warp::tui_export::{
    AIContextMenuCategory, AIContextMenuMixer, AIContextMenuSearchableAction, ActiveSession,
    ActiveSessionEvent, AtContextMenuCoreState, AtContextMenuGates, AtContextMenuQueryTransition,
    AtContextMenuSourceContext, BlocklistAIInputModel, NavigationState, at_context_menu_query,
    install_sources_for_all_categories, install_sources_for_category, is_at_menu_trigger,
    is_valid_at_menu_query, should_close_at_menu,
};
use warp_core::features::FeatureFlag;
use warp_core::settings::Setting as _;
use warp_editor::model::CoreEditorModel;
use warp_search_core::inline_menu::InlineMenuResultsUpdate;
use warpui::SingletonEntity as _;
use warpui_core::text::{byte_offset_for_char_offset, char_slice};
use warpui_core::{AppContext, Entity, ModelContext, ModelHandle};

use crate::inline_menu::{
    MAX_INLINE_MENU_ROWS, TuiInlineMenuAccepted, TuiInlineMenuHandle, TuiInlineMenuHeader,
    TuiInlineMenuListState, TuiInlineMenuRow, TuiInlineMenuRowStyle, TuiInlineMenuSnapshot,
    TuiInlineMenuStatus, result_row_capacity,
};
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};

const MAX_VISIBLE_ROWS: usize = result_row_capacity(MAX_INLINE_MENU_ROWS, true, false);

#[derive(Debug, Clone)]
enum TuiAtContextMenuRow {
    Category(AIContextMenuCategory),
    Result {
        title: String,
        description: Option<String>,
        action: AIContextMenuSearchableAction,
    },
}

#[derive(Debug, Clone, Default)]
enum TuiAtContextMenuState {
    #[default]
    Closed,
    Open {
        /// Zero-based character offset of the `@` in the editor buffer.
        at_symbol_position: usize,
        query: String,
        list: TuiInlineMenuListState<TuiAtContextMenuRow>,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct TuiAtContextMenuAcceptance {
    pub(crate) action: AIContextMenuSearchableAction,
    /// Byte range covering the `@` and the filter text it replaces.
    pub(crate) replacement_range: Range<usize>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TuiAtContextMenuEvent;

pub(crate) struct TuiAtContextMenuModel {
    input_editor: ModelHandle<CodeEditorModel>,
    suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
    active_session: ModelHandle<ActiveSession>,
    input_mode: ModelHandle<BlocklistAIInputModel>,
    mixer: ModelHandle<AIContextMenuMixer>,
    core: AtContextMenuCoreState,
    state: TuiAtContextMenuState,
    /// Potential categories whose zero-state sources returned at least one
    /// result, kept in the core's display order. `None` while discovery loads.
    discovered_categories: Option<Vec<AIContextMenuCategory>>,
    /// An Escape-dismissed trigger must not reopen just because the cursor moves
    /// away and back. Deleting it or typing a different `@` clears the block.
    dismissed_at_symbol_position: Option<usize>,
}

impl TuiAtContextMenuModel {
    pub(crate) fn new(
        input_editor: ModelHandle<CodeEditorModel>,
        suggestions_mode: ModelHandle<TuiInputSuggestionsModeModel>,
        active_session: ModelHandle<ActiveSession>,
        input_mode: ModelHandle<BlocklistAIInputModel>,
        mixer: ModelHandle<AIContextMenuMixer>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&input_editor, |model, _, event, ctx| {
            if matches!(
                event,
                CodeEditorModelEvent::ContentChanged { .. }
                    | CodeEditorModelEvent::SelectionChanged
            ) {
                model.update_from_input(ctx);
            }
        });
        ctx.subscribe_to_model(&active_session, |model, _, event, ctx| {
            if matches!(
                event,
                ActiveSessionEvent::UpdatedPwd | ActiveSessionEvent::Bootstrapped
            ) {
                model.update_working_directory(ctx);
            }
        });
        ctx.subscribe_to_model(&mixer, |model, _, event, ctx| {
            if matches!(event, SearchMixerEvent::ResultsChanged) {
                model.refresh_result_rows(ctx);
            }
        });

        let core = AtContextMenuCoreState::new(AtContextMenuGates {
            supports_blocks: false,
            supports_code_symbols: false,
            ..Default::default()
        });
        let mut model = Self {
            input_editor,
            suggestions_mode,
            active_session,
            input_mode,
            mixer,
            core,
            state: TuiAtContextMenuState::Closed,
            discovered_categories: None,
            dismissed_at_symbol_position: None,
        };
        model.update_working_directory(ctx);
        model
    }

    fn has_open_state(&self) -> bool {
        matches!(self.state, TuiAtContextMenuState::Open { .. })
    }

    pub(crate) fn is_open(&self, app: &AppContext) -> bool {
        self.has_open_state()
            && self.suggestions_mode.as_ref(app).mode() == TuiInputSuggestionsMode::AtContextMenu
    }

    pub(crate) fn at_symbol_position(&self) -> Option<usize> {
        match self.state {
            TuiAtContextMenuState::Closed => None,
            TuiAtContextMenuState::Open {
                at_symbol_position, ..
            } => Some(at_symbol_position),
        }
    }

    fn refresh_discovered_categories(&mut self, ctx: &mut ModelContext<Self>) {
        if self.mixer.as_ref(ctx).is_loading() {
            return;
        }
        let potential_categories = self.core.available_categories(ctx);
        let file_category = potential_categories
            .iter()
            .copied()
            .find(|category| {
                matches!(
                    category,
                    AIContextMenuCategory::CurrentFolderFiles | AIContextMenuCategory::RepoFiles
                )
            })
            .unwrap_or(AIContextMenuCategory::CurrentFolderFiles);
        let result_categories: Vec<_> = self
            .mixer
            .as_ref(ctx)
            .results()
            .iter()
            .filter_map(|result| result.accept_result().category(file_category))
            .collect();
        let discovered_categories: Vec<_> = potential_categories
            .into_iter()
            .filter(|category| {
                matches!(
                    category,
                    AIContextMenuCategory::CurrentFolderFiles | AIContextMenuCategory::RepoFiles
                ) || result_categories.contains(category)
            })
            .collect();
        self.discovered_categories = Some(discovered_categories.clone());

        let query = match &self.state {
            TuiAtContextMenuState::Closed => return,
            TuiAtContextMenuState::Open { query, .. } => query.clone(),
        };
        if query.is_empty() {
            self.refresh_category_rows(ctx);
        } else {
            match self
                .core
                .set_query_for_categories(&query, discovered_categories)
            {
                AtContextMenuQueryTransition::EnteredAllCategories => {
                    self.setup_all_categories(&query, ctx);
                }
                AtContextMenuQueryTransition::SourcesUnchanged => {
                    self.refresh_category_rows(ctx);
                }
            }
        }
        ctx.emit(TuiAtContextMenuEvent);
    }

    fn input_snapshot(&self, app: &AppContext) -> Option<(String, usize)> {
        let editor = self.input_editor.as_ref(app);
        if !editor.selection_is_single_cursor(app) {
            return None;
        }
        let content = editor.content().as_ref(app);
        let text = if content.is_empty() {
            String::new()
        } else {
            content.text().into_string()
        };
        let cursor = editor
            .buffer_selection_model()
            .as_ref(app)
            .first_selection_head()
            .as_usize()
            .saturating_sub(1);
        Some((text, cursor))
    }

    fn update_working_directory(&mut self, ctx: &mut ModelContext<Self>) {
        let working_directory = self
            .active_session
            .as_ref(ctx)
            .current_working_directory_location(ctx);
        if !self.core.set_working_directory(working_directory) || !self.is_open(ctx) {
            return;
        }
        self.core.refresh_categories(ctx);
        self.core.reset_to_main_menu();
        self.discovered_categories = None;
        self.setup_navigation_state("", ctx);
        ctx.emit(TuiAtContextMenuEvent);
    }

    fn update_gates(&mut self, app: &AppContext) -> bool {
        let input_mode = self.input_mode.as_ref(app);
        let is_ai_or_autodetect_mode =
            input_mode.input_type().is_ai() || !input_mode.is_input_type_locked();
        self.core.set_gates(AtContextMenuGates {
            is_ai_or_autodetect_mode,
            supports_blocks: false,
            supports_code_symbols: false,
            ..Default::default()
        })
    }

    fn may_open(&mut self, app: &AppContext) -> bool {
        if !FeatureFlag::AIContextMenuEnabled.is_enabled() {
            return false;
        }
        self.update_gates(app);
        let gates = self.core.gates();
        gates.is_ai_or_autodetect_mode
            || (FeatureFlag::AtMenuOutsideOfAIMode.is_enabled()
                && *InputSettings::as_ref(app)
                    .at_context_menu_in_terminal_mode
                    .value())
    }

    fn update_from_input(&mut self, ctx: &mut ModelContext<Self>) {
        let Some((text, cursor)) = self.input_snapshot(ctx) else {
            self.close(ctx);
            return;
        };

        if !self.has_open_state() {
            if let Some(dismissed_position) = self.dismissed_at_symbol_position {
                if text.chars().nth(dismissed_position) == Some('@') {
                    if cursor.saturating_sub(1) == dismissed_position {
                        return;
                    }
                } else {
                    self.dismissed_at_symbol_position = None;
                }
            }
            if self.suggestions_mode.as_ref(ctx).mode() != TuiInputSuggestionsMode::Closed
                || !is_at_menu_trigger(&text, cursor)
                || !self.may_open(ctx)
            {
                return;
            }
            self.open(cursor.saturating_sub(1), ctx);
            return;
        }

        if !self.is_open(ctx) {
            self.state = TuiAtContextMenuState::Closed;
            return;
        }

        let TuiAtContextMenuState::Open {
            at_symbol_position,
            query: previous_query,
            ..
        } = &self.state
        else {
            return;
        };
        let at_symbol_position = *at_symbol_position;
        let previous_query = previous_query.clone();

        if should_close_at_menu(&text, cursor, at_symbol_position)
            || text.chars().nth(at_symbol_position) != Some('@')
        {
            self.close(ctx);
            return;
        }
        let Some(query) = char_slice(&text, at_symbol_position + 1, cursor).map(str::to_owned)
        else {
            self.close(ctx);
            return;
        };
        if !is_valid_at_menu_query(false, &previous_query, &query) {
            self.close(ctx);
            return;
        }

        let visible_results_are_empty = match &self.state {
            TuiAtContextMenuState::Closed => true,
            TuiAtContextMenuState::Open { list, .. } => list.rows().is_empty(),
        };
        let should_close = self.core.record_results_update(
            visible_results_are_empty,
            self.mixer.as_ref(ctx).is_loading(),
            query.contains(&previous_query),
        );
        if should_close {
            self.close(ctx);
            return;
        }

        if let TuiAtContextMenuState::Open {
            query: current_query,
            ..
        } = &mut self.state
        {
            *current_query = query.clone();
        }

        if self.core.navigation_state() == NavigationState::MainMenu
            && self.discovered_categories.is_none()
        {
            // Discovery is still loading. Store the query now; the final
            // discovery result applies it to the non-empty category set.
            ctx.emit(TuiAtContextMenuEvent);
            return;
        }

        let query_transition = if self.core.navigation_state() == NavigationState::MainMenu {
            self.core.set_query_for_categories(
                &query,
                self.discovered_categories.clone().unwrap_or_default(),
            )
        } else {
            self.core.set_query(&query, ctx)
        };
        match query_transition {
            AtContextMenuQueryTransition::EnteredAllCategories => {
                self.setup_all_categories(&query, ctx);
            }
            AtContextMenuQueryTransition::SourcesUnchanged => {
                if self.core.navigation_state() == NavigationState::MainMenu {
                    self.refresh_category_rows(ctx);
                } else {
                    self.run_query(&query, ctx);
                }
            }
        }
        ctx.emit(TuiAtContextMenuEvent);
    }

    fn open(&mut self, at_symbol_position: usize, ctx: &mut ModelContext<Self>) {
        let did_open = self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.try_open(TuiInputSuggestionsMode::AtContextMenu, ctx)
        });
        if !did_open {
            return;
        }

        self.update_gates(ctx);
        self.core.refresh_categories(ctx);
        self.core.reset_to_main_menu();
        self.discovered_categories = None;
        self.state = TuiAtContextMenuState::Open {
            at_symbol_position,
            query: String::new(),
            list: TuiInlineMenuListState::default(),
        };
        self.setup_navigation_state("", ctx);
        ctx.emit(TuiAtContextMenuEvent);
    }

    fn setup_navigation_state(&mut self, query: &str, ctx: &mut ModelContext<Self>) {
        match self.core.navigation_state() {
            NavigationState::MainMenu => self.setup_category_discovery(ctx),
            NavigationState::Category(category) => self.setup_category(category, query, ctx),
            NavigationState::AllCategories => self.setup_all_categories(query, ctx),
        }
    }

    /// Runs every potentially available category's zero state through one
    /// mixer. Once all sources settle, result actions tell us which categories
    /// actually contain entries.
    fn setup_category_discovery(&mut self, ctx: &mut ModelContext<Self>) {
        let categories = self.core.available_categories(ctx);
        let source_context = self.source_context();
        self.discovered_categories = None;
        if let TuiAtContextMenuState::Open { list, .. } = &mut self.state {
            list.replace_rows(Vec::new(), true, None, MAX_VISIBLE_ROWS, |_| true);
        }
        self.mixer.update(ctx, |mixer, ctx| {
            mixer.reset(ctx);
            install_sources_for_all_categories(mixer, &categories, &source_context, ctx);
            mixer.run_query(at_context_menu_query(""), ctx);
        });
        self.refresh_result_rows(ctx);
    }

    fn source_context(&self) -> AtContextMenuSourceContext {
        AtContextMenuSourceContext {
            code_symbol_cache: None,
            working_directory: self.core.working_directory().cloned(),
        }
    }

    fn setup_category(
        &mut self,
        category: AIContextMenuCategory,
        query: &str,
        ctx: &mut ModelContext<Self>,
    ) {
        let source_context = self.source_context();
        let query = query.to_owned();
        self.mixer.update(ctx, |mixer, ctx| {
            mixer.reset(ctx);
            install_sources_for_category(mixer, category, &source_context, ctx);
            mixer.run_query(at_context_menu_query(&query), ctx);
        });
        self.refresh_result_rows(ctx);
    }

    fn setup_all_categories(&mut self, query: &str, ctx: &mut ModelContext<Self>) {
        let categories = self
            .discovered_categories
            .clone()
            .unwrap_or_else(|| self.core.available_categories(ctx));
        let source_context = self.source_context();
        let query = query.to_owned();
        self.mixer.update(ctx, |mixer, ctx| {
            mixer.reset(ctx);
            install_sources_for_all_categories(mixer, &categories, &source_context, ctx);
            mixer.run_query(at_context_menu_query(&query), ctx);
        });
        self.refresh_result_rows(ctx);
    }

    fn run_query(&mut self, query: &str, ctx: &mut ModelContext<Self>) {
        self.mixer.update(ctx, |mixer, ctx| {
            mixer.run_query(at_context_menu_query(query), ctx);
        });
        self.refresh_result_rows(ctx);
    }

    fn refresh_category_rows(&mut self, ctx: &mut ModelContext<Self>) {
        let categories = self.core.filtered_categories_from(
            self.discovered_categories
                .clone()
                .unwrap_or_else(|| self.core.available_categories(ctx)),
        );
        let rows = categories
            .into_iter()
            .map(TuiAtContextMenuRow::Category)
            .collect();
        let TuiAtContextMenuState::Open { list, .. } = &mut self.state else {
            return;
        };
        let preferred_index = list.selected_index().unwrap_or_default();
        list.replace_rows(rows, false, Some(preferred_index), MAX_VISIBLE_ROWS, |_| {
            true
        });
    }

    fn refresh_result_rows(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.is_open(ctx) {
            return;
        }
        if self.core.navigation_state() == NavigationState::MainMenu {
            self.refresh_discovered_categories(ctx);
            return;
        }
        let (is_loading, rows) = {
            let mixer = self.mixer.as_ref(ctx);
            let rows = mixer
                .results()
                .iter()
                .filter(|result| !result.is_static_separator() && !result.is_disabled())
                .filter_map(|result| {
                    let detail = result.detail_data()?;
                    Some(TuiAtContextMenuRow::Result {
                        title: detail.title,
                        description: detail.description,
                        action: result.accept_result(),
                    })
                })
                .collect();
            (mixer.is_loading(), rows)
        };
        let TuiAtContextMenuState::Open { list, .. } = &mut self.state else {
            return;
        };
        let update = list.reconcile_mixer_rows(rows, is_loading, MAX_VISIBLE_ROWS, |_| true);
        if !matches!(update, InlineMenuResultsUpdate::Loading) {
            ctx.emit(TuiAtContextMenuEvent);
        }
    }

    pub(crate) fn select_previous(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiAtContextMenuState::Open { list, .. } = &mut self.state else {
            return;
        };
        list.select_previous(MAX_VISIBLE_ROWS, |_| true);
        ctx.emit(TuiAtContextMenuEvent);
    }

    pub(crate) fn select_next(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiAtContextMenuState::Open { list, .. } = &mut self.state else {
            return;
        };
        list.select_next(MAX_VISIBLE_ROWS, |_| true);
        ctx.emit(TuiAtContextMenuEvent);
    }

    pub(crate) fn select_at_snapshot_index(
        &mut self,
        index: usize,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let TuiAtContextMenuState::Open { list, .. } = &mut self.state else {
            return false;
        };
        let selected = list.select_absolute(index, MAX_VISIBLE_ROWS, |_| true);
        ctx.emit(TuiAtContextMenuEvent);
        selected
    }

    pub(crate) fn scroll_by_delta(&mut self, delta: isize, ctx: &mut ModelContext<Self>) {
        let TuiAtContextMenuState::Open { list, .. } = &mut self.state else {
            return;
        };
        list.scroll_by(delta, MAX_VISIBLE_ROWS);
        ctx.emit(TuiAtContextMenuEvent);
    }

    pub(crate) fn accept_selected(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> Option<TuiAtContextMenuAcceptance> {
        if !self.is_open(ctx) {
            return None;
        }
        let row = match &self.state {
            TuiAtContextMenuState::Closed => return None,
            TuiAtContextMenuState::Open { list, .. } => list.selected_row()?.clone(),
        };
        match row {
            TuiAtContextMenuRow::Category(category) => {
                self.remove_category_filter_from_input(ctx);
                self.core.enter_category(category);
                if let TuiAtContextMenuState::Open { query, .. } = &mut self.state {
                    query.clear();
                }
                self.setup_category(category, "", ctx);
                ctx.emit(TuiAtContextMenuEvent);
                None
            }
            TuiAtContextMenuRow::Result { action, .. } => {
                let (text, cursor) = self.input_snapshot(ctx)?;
                let at_symbol_position = self.at_symbol_position()?;
                let start =
                    byte_offset_for_char_offset(&text, CharOffset::from(at_symbol_position))?;
                let end = byte_offset_for_char_offset(&text, CharOffset::from(cursor))?;
                self.close(ctx);
                Some(TuiAtContextMenuAcceptance {
                    action,
                    replacement_range: start.as_usize()..end.as_usize(),
                })
            }
        }
    }

    fn remove_category_filter_from_input(&mut self, ctx: &mut ModelContext<Self>) {
        let Some((_, cursor)) = self.input_snapshot(ctx) else {
            return;
        };
        let Some(at_symbol_position) = self.at_symbol_position() else {
            return;
        };
        let start = at_symbol_position + 2;
        let end = cursor + 1;
        if start >= end {
            return;
        }
        self.input_editor.update(ctx, |editor, ctx| {
            editor.select_at(CharOffset::from(start), false, ctx);
            editor.set_last_selection_head(CharOffset::from(end), ctx);
            editor.end_selection(ctx);
            editor.user_insert("", ctx);
        });
    }

    pub(crate) fn dismiss(&mut self, ctx: &mut ModelContext<Self>) {
        self.dismissed_at_symbol_position = self.at_symbol_position();
        self.close(ctx);
    }

    fn close(&mut self, ctx: &mut ModelContext<Self>) {
        if self.has_open_state() {
            self.state = TuiAtContextMenuState::Closed;
            self.core.reset_results_progress();
            self.mixer.update(ctx, |mixer, ctx| mixer.reset(ctx));
            ctx.emit(TuiAtContextMenuEvent);
        }
        self.suggestions_mode.update(ctx, |mode, ctx| {
            mode.close_if_active(TuiInputSuggestionsMode::AtContextMenu, ctx);
        });
    }

    pub(crate) fn snapshot(&self, app: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        if !self.is_open(app) {
            return None;
        }
        let TuiAtContextMenuState::Open { list, .. } = &self.state else {
            return None;
        };
        let title = match self.core.navigation_state() {
            NavigationState::MainMenu => "Add context".to_owned(),
            NavigationState::Category(category) => category.name().to_owned(),
            NavigationState::AllCategories => "Search context".to_owned(),
        };
        let status = if list.rows().is_empty() {
            Some(if list.is_loading() {
                TuiInlineMenuStatus::Loading("Loading context…".to_owned())
            } else {
                TuiInlineMenuStatus::Empty("No context found".to_owned())
            })
        } else {
            None
        };
        Some(TuiInlineMenuSnapshot {
            header: Some(TuiInlineMenuHeader {
                title: Some(title),
                tabs: Vec::new(),
            }),
            rows: list
                .rows()
                .iter()
                .map(|row| match row {
                    TuiAtContextMenuRow::Category(category) => TuiInlineMenuRow {
                        title: category.name().to_owned(),
                        prefix: None,
                        description: None,
                        state_suffix: Some("›".to_owned()),
                        is_selectable: true,
                        style: TuiInlineMenuRowStyle::Default,
                    },
                    TuiAtContextMenuRow::Result {
                        title, description, ..
                    } => TuiInlineMenuRow {
                        title: title.clone(),
                        prefix: None,
                        description: description.clone(),
                        state_suffix: None,
                        is_selectable: true,
                        style: TuiInlineMenuRowStyle::InlineMenuItem,
                    },
                })
                .collect(),
            selected_index: list.selected_index(),
            scroll_offset: list.scroll_offset(),
            scroll_anchor: list.scroll_anchor(),
            max_visible_rows: MAX_VISIBLE_ROWS,
            status,
        })
    }
}

impl TuiInlineMenuHandle for ModelHandle<TuiAtContextMenuModel> {
    fn mode(&self) -> TuiInputSuggestionsMode {
        TuiInputSuggestionsMode::AtContextMenu
    }

    fn is_open(&self, app: &AppContext) -> bool {
        self.as_ref(app).is_open(app)
    }

    fn input_highlight_range(&self, _app: &AppContext) -> Option<Range<CharOffset>> {
        None
    }

    fn input_argument_hint_text(&self, _app: &AppContext) -> Option<&'static str> {
        None
    }

    fn select_previous(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.select_previous(ctx));
    }

    fn select_next(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.select_next(ctx));
    }

    fn accept(&self, ctx: &mut AppContext) -> Option<TuiInlineMenuAccepted> {
        self.update(ctx, |model, ctx| model.accept_selected(ctx))
            .map(TuiInlineMenuAccepted::AtContextMenu)
    }

    fn dismiss(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.dismiss(ctx));
    }

    fn snapshot(&self, app: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        self.as_ref(app).snapshot(app)
    }

    fn select_by_snapshot_index(&self, index: usize, ctx: &mut AppContext) -> bool {
        self.update(ctx, |model, ctx| model.select_at_snapshot_index(index, ctx))
    }

    fn scroll_by_delta(&self, delta: isize, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.scroll_by_delta(delta, ctx));
    }
}

impl Entity for TuiAtContextMenuModel {
    type Event = TuiAtContextMenuEvent;
}
