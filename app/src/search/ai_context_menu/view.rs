use std::ops::Range;
use std::time::Duration;

use async_channel::Sender;
use itertools::Itertools;
#[cfg(not(target_family = "wasm"))]
use repo_metadata::repositories::DetectedRepositories;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::elements::{
    AnchorPair, Border, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    Dismiss, Empty, Fill, Flex, Hoverable, Icon, MouseStateHandle, OffsetPositioning, OffsetType,
    ParentElement, PositionedElementOffsetBounds, PositioningAxis, Radius, SavePosition,
    ScrollStateHandle, Scrollable, ScrollableElement, ScrollbarWidth, Shrinkable, Stack, Text,
    UniformList, UniformListState, XAxisAnchor, YAxisAnchor,
};
use warpui::platform::Cursor;
use warpui::windowing::WindowManager;
use warpui::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle, WeakViewHandle,
};

use super::core::{
    AIContextMenuCategory, AtContextMenuCoreState, AtContextMenuGates,
    AtContextMenuQueryTransition, NavigationState,
};
use super::styles;
use crate::appearance::Appearance;
use crate::debounce;
#[cfg(not(target_family = "wasm"))]
use crate::search::ai_context_menu::code::data_source::CodeSymbolCache;
#[cfg(not(target_family = "wasm"))]
use crate::search::ai_context_menu::code::is_code_symbols_indexing;
use crate::search::ai_context_menu::mixer::{
    AIContextMenuMixer, AIContextMenuSearchableAction, AtContextMenuSourceContext,
    at_context_menu_query, install_sources_for_all_categories, install_sources_for_category,
};
use crate::search::data_source::QueryResult;
use crate::search::result_renderer::{QueryResultRenderer, QueryResultRendererStyles};
use crate::search::search_bar::{SearchBar, SearchBarEvent, SearchBarState, SearchResultOrdering};
use crate::settings::InputSettings;
#[cfg(not(target_family = "wasm"))]
use crate::workspace::ActiveSession;

const CORNER_RADIUS: f32 = 8.0;
const DEFAULT_PALETTE_WIDTH: f32 = 320.0;
const MAX_DISPLAYED_RESULT_COUNT: usize = 8;
const PALETTE_HEIGHT: f32 = 423.0;
const PADDING: f32 = 10.0;
const SEARCH_DEBOUNCE_PERIOD: Duration = Duration::from_millis(60);
const DETAILS_PANEL_MARGIN: f32 = 4.0;
const PANEL_POSITION_ID: &str = "AIContextMenuPanel";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AIContextMenuPosition {
    /// The user clicked the AI Context Menu button.
    AtButton,
    /// If this is at the user's cursor, then we don't need to show a
    /// text input field.
    AtCursor,
}

impl AIContextMenuCategory {
    pub fn icon(&self) -> &'static str {
        match self {
            AIContextMenuCategory::CurrentFolderFiles => "bundled/svg/folder.svg",
            AIContextMenuCategory::RepoFiles => "bundled/svg/folder.svg",
            AIContextMenuCategory::Commands => "bundled/svg/terminal.svg",
            AIContextMenuCategory::Blocks => "bundled/svg/terminal.svg",
            AIContextMenuCategory::Workflows => "bundled/svg/workflow.svg",
            AIContextMenuCategory::Notebooks => "bundled/svg/notebook.svg",
            AIContextMenuCategory::Plans => "bundled/svg/compass-3.svg",
            AIContextMenuCategory::Diffs => "bundled/svg/diff.svg",
            AIContextMenuCategory::Docs => "bundled/svg/docs.svg",
            AIContextMenuCategory::Tasks => "bundled/svg/tasks.svg",
            AIContextMenuCategory::Rules => "bundled/svg/book-open.svg",
            AIContextMenuCategory::Servers => "bundled/svg/server.svg",
            AIContextMenuCategory::Terminal => "bundled/svg/terminal.svg",
            AIContextMenuCategory::Web => "bundled/svg/web.svg",
            AIContextMenuCategory::RecentDiff => "bundled/svg/diff.svg",
            AIContextMenuCategory::RecentBlock => "bundled/svg/block.svg",
            AIContextMenuCategory::Code => "bundled/svg/code-02.svg",
            AIContextMenuCategory::DiffSet => "bundled/svg/diff.svg",
            AIContextMenuCategory::Conversations => "bundled/svg/conversation.svg",
            AIContextMenuCategory::Skills => "bundled/svg/stars-01.svg",
        }
    }
}

#[derive(Debug, Clone)]
pub enum AIContextMenuAction {
    Prev,
    Next,
    SelectCurrentItem,
    ResultAccepted {
        action: AIContextMenuSearchableAction,
    },
    CategorySelected {
        category: AIContextMenuCategory,
    },
    Close,
}

pub enum AIContextMenuEvent {
    Close {
        query_length: usize,
        item_count: Option<usize>,
    },
    ResultAccepted {
        action: AIContextMenuSearchableAction,
        query_length: usize,
        item_count: Option<usize>,
    },
    CategorySelected {
        category: AIContextMenuCategory,
    },
}

/// GUI-only presentation state. Navigation, category selection, the
/// availability gates, and the no-progress counter live on the shared
/// [`AtContextMenuCoreState`].
struct AIContextMenuState {
    scroll_state: ScrollStateHandle,
    uniform_list_state: UniformListState,
    category_hover_states: Vec<MouseStateHandle>,
}

/// Maximum number of results to display
const MAX_SEARCH_RESULTS: usize = 250;

/// AI Context Menu View
pub struct AIContextMenu {
    mixer: ModelHandle<AIContextMenuMixer>,
    /// While we aren't rendering a search bar, the view contains
    /// a lot of helpful logic for managing the search state.
    search_bar: ViewHandle<SearchBar<AIContextMenuSearchableAction>>,
    search_bar_state: ModelHandle<SearchBarState<AIContextMenuSearchableAction>>,
    #[cfg(not(target_family = "wasm"))]
    code_symbol_cache: ModelHandle<CodeSymbolCache>,
    /// Menu state shared with the TUI front-end.
    core: AtContextMenuCoreState,
    state: AIContextMenuState,
    /// Debounce channel for search queries
    search_debounce_tx: Sender<String>,
    handle: WeakViewHandle<Self>,
}

impl AIContextMenu {
    pub fn set_is_shared_session_viewer(&mut self, is_viewer: bool, ctx: &mut ViewContext<Self>) {
        let mut gates = self.core.gates();
        gates.is_shared_session_viewer = is_viewer;
        self.apply_gates(gates, ctx);
    }

    pub fn set_is_in_ambient_agent(&mut self, is_ambient: bool, ctx: &mut ViewContext<Self>) {
        let mut gates = self.core.gates();
        gates.is_in_ambient_agent = is_ambient;
        self.apply_gates(gates, ctx);
    }

    pub fn set_is_cli_agent_input(
        &mut self,
        is_cli_agent_input: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let mut gates = self.core.gates();
        gates.is_cli_agent_input = is_cli_agent_input;
        self.apply_gates(gates, ctx);
    }

    fn apply_gates(&mut self, gates: AtContextMenuGates, ctx: &mut ViewContext<Self>) {
        if self.core.set_gates(gates) {
            self.refresh_categories_state(ctx);
        }
    }
}

impl Entity for AIContextMenu {
    type Event = AIContextMenuEvent;
}

impl TypedActionView for AIContextMenu {
    type Action = AIContextMenuAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AIContextMenuAction::Prev => match self.core.navigation_state() {
                NavigationState::MainMenu => {
                    self.core.select_previous_category(ctx);
                    ctx.notify();
                }
                NavigationState::Category(_) | NavigationState::AllCategories => {
                    // Navigate up in search results
                    self.search_bar.update(ctx, |search_bar, ctx| {
                        search_bar.up(ctx);
                    });
                }
            },
            AIContextMenuAction::Next => match self.core.navigation_state() {
                NavigationState::MainMenu => {
                    self.core.select_next_category(ctx);
                    ctx.notify();
                }
                NavigationState::Category(_) | NavigationState::AllCategories => {
                    // Navigate down in search results
                    self.search_bar.update(ctx, |search_bar, ctx| {
                        search_bar.down(ctx);
                    });
                }
            },
            AIContextMenuAction::SelectCurrentItem => {
                self.select_current_item(ctx);
            }
            AIContextMenuAction::ResultAccepted { action } => {
                let query_length = self.query(ctx).len();
                let item_count = self.item_count(ctx);
                ctx.emit(AIContextMenuEvent::ResultAccepted {
                    action: action.clone(),
                    query_length,
                    item_count,
                });
            }
            AIContextMenuAction::CategorySelected { category } => {
                // Navigate to the category view
                self.core.enter_category(*category);
                self.reset_mixer(ctx);
                // Emit CategorySelected event to let the input handle it
                ctx.emit(AIContextMenuEvent::CategorySelected {
                    category: *category,
                });
                ctx.notify();
            }
            AIContextMenuAction::Close => self.close(ctx),
        }
    }
}

lazy_static::lazy_static! {
    static ref QUERY_RESULT_RENDERER_STYLES: QueryResultRendererStyles =
        QueryResultRendererStyles {
            result_item_height_fn: |appearance| {
                10.0 + appearance.monospace_font_size()
            },
            panel_border_fn: |appearance| {
                Border::all(1.0).with_border_fill(appearance.theme().outline())
            },
            result_horizontal_padding: PADDING,
            ..Default::default()
        };

    static ref TERMINAL_MODE_CATEGORIES: Vec<AIContextMenuCategory> = {
        vec![AIContextMenuCategory::RepoFiles]
    };
}
impl AIContextMenu {
    /// Set the input mode and update the menu state accordingly
    pub fn set_input_mode(&mut self, is_ai_or_autodetect_mode: bool, ctx: &mut ViewContext<Self>) {
        let mut gates = self.core.gates();
        gates.is_ai_or_autodetect_mode = is_ai_or_autodetect_mode;
        self.apply_gates(gates, ctx);
    }

    /// The working directory of the active window's session, which decides
    /// whether the repo-scoped categories are available.
    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn active_working_directory(app: &AppContext) -> Option<LocalOrRemotePath> {
        app.windows()
            .state()
            .active_window
            .and_then(|window_id| ActiveSession::as_ref(app).working_directory(window_id))
            .cloned()
    }

    /// WASM builds have no session working directory to resolve.
    #[cfg(target_family = "wasm")]
    pub(crate) fn active_working_directory(_app: &AppContext) -> Option<LocalOrRemotePath> {
        None
    }

    /// Recompute category-dependent state when repository availability changes.
    fn refresh_categories_state(&mut self, ctx: &mut ViewContext<Self>) {
        self.refresh_categories(ctx);
        ctx.notify();
    }

    /// The body of [`Self::refresh_categories_state`] without the redraw
    /// request, so construction can reuse it before the view is registered.
    fn refresh_categories(&mut self, ctx: &mut ViewContext<Self>) {
        self.core
            .set_working_directory(Self::active_working_directory(ctx));
        self.core.refresh_categories(ctx);

        // One retained hover state per category row the main menu renders.
        let category_count = self.core.available_categories(ctx).len();
        self.state.category_hover_states =
            (0..category_count).map(|_| Default::default()).collect();

        // Reset mixer with new category configuration
        self.reset_mixer(ctx);
    }

    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let search_bar_state = ctx.add_model(|_ctx| {
            SearchBarState::new(SearchResultOrdering::TopDown)
                .with_max_results(MAX_SEARCH_RESULTS)
                .run_query_on_buffer_empty()
        });

        let mixer = ctx.add_model(|_| AIContextMenuMixer::new());
        ctx.observe(&search_bar_state, |_, _, ctx| {
            ctx.notify();
        });

        let search_bar = ctx.add_typed_action_view(|ctx| {
            SearchBar::new(
                mixer.clone(),
                search_bar_state.clone(),
                "",
                Self::create_query_result_renderer,
                ctx,
            )
        });

        ctx.subscribe_to_view(&search_bar, |me, _handle, event, ctx| {
            me.handle_search_bar_event(event, ctx);
        });

        ctx.subscribe_to_model(&search_bar_state, |me, _handle, event, ctx| {
            me.handle_search_bar_event(event, ctx);
        });

        // Subscribe to InputSettings changes to detect when the outline_codebase_symbols_for_at_context_menu setting changes
        ctx.subscribe_to_model(&InputSettings::handle(ctx), |me, _handle, _event, ctx| {
            // When settings change, close the menu to reset state and reflect new category configuration
            me.close(ctx);
        });

        // Subscribe to repository detection so categories (Files/Code) update when a git repo is found.
        #[cfg(not(target_family = "wasm"))]
        ctx.subscribe_to_model(
            &DetectedRepositories::handle(ctx),
            |me, _handle, _event, ctx| {
                // Repo availability may have changed; refresh categories and hover state.
                me.refresh_categories_state(ctx);
            },
        );
        ctx.subscribe_to_model(&WindowManager::handle(ctx), |me, _handle, _event, ctx| {
            // Need to update categories state because the active window may have changed, affecting the active repo data.
            me.refresh_categories_state(ctx);
        });

        #[cfg(not(target_family = "wasm"))]
        ctx.observe(
            &ActiveSession::handle(ctx),
            Self::handle_active_session_change,
        );

        // Set up debounce system for search queries
        let (search_debounce_tx, search_debounce_rx) = async_channel::unbounded();
        let _ = ctx.spawn_stream_local(
            debounce(SEARCH_DEBOUNCE_PERIOD, search_debounce_rx),
            |me, query, ctx| me.update_search_query_internal(query, ctx),
            |_me, _ctx| {},
        );

        #[cfg(not(target_family = "wasm"))]
        let code_symbol_cache = ctx.add_model(CodeSymbolCache::new);

        // When the outline updates (e.g. indexing finishes), re-run the current
        // mixer query so the Code results refresh automatically.
        #[cfg(not(target_family = "wasm"))]
        ctx.subscribe_to_model(&code_symbol_cache, |me, _handle, _event, ctx| {
            let code_active = matches!(
                me.core.navigation_state(),
                NavigationState::Category(AIContextMenuCategory::Code)
                    | NavigationState::AllCategories
            );
            if code_active {
                me.mixer.update(ctx, |mixer, ctx| {
                    if let Some(query) = mixer.current_query().cloned() {
                        mixer.run_query(query, ctx);
                    }
                });
            }
        });

        let mut result = Self {
            mixer,
            search_bar,
            search_bar_state,
            #[cfg(not(target_family = "wasm"))]
            code_symbol_cache,
            // The gate defaults start in AI mode; the input updates them through
            // `set_input_mode` and the `set_is_*` setters as its state resolves.
            core: AtContextMenuCoreState::new(AtContextMenuGates::default()),
            state: AIContextMenuState {
                scroll_state: Default::default(),
                uniform_list_state: Default::default(),
                category_hover_states: Vec::new(),
            },
            handle: ctx.handle(),
            search_debounce_tx,
        };

        result.refresh_categories(ctx);
        result
    }

    #[cfg(not(target_family = "wasm"))]
    fn handle_active_session_change(
        &mut self,
        _handle: ModelHandle<ActiveSession>,
        ctx: &mut ViewContext<Self>,
    ) {
        // Need to refresh categories state because the current working directory may have changed,
        // affecting whether we're in a git repository or not (changing the categories available).
        self.refresh_categories_state(ctx);
    }

    pub fn select_current_item(&mut self, ctx: &mut ViewContext<Self>) {
        match self.core.navigation_state() {
            NavigationState::MainMenu => {
                // Select the current category from filtered categories
                if let Some(category) = self.core.selected_category(ctx) {
                    self.handle_action(&AIContextMenuAction::CategorySelected { category }, ctx);
                }
            }
            NavigationState::Category(_) | NavigationState::AllCategories => {
                // Select the current search result
                self.search_bar.update(ctx, |search_bar, ctx| {
                    search_bar.select_current_item(ctx);
                });
            }
        }
    }

    fn create_query_result_renderer(
        index: usize,
        result: QueryResult<AIContextMenuSearchableAction>,
    ) -> QueryResultRenderer<AIContextMenuSearchableAction> {
        QueryResultRenderer::new(
            result,
            Self::query_result_save_position_id(index),
            |_result_index, action, event_ctx| {
                event_ctx.dispatch_typed_action(AIContextMenuAction::ResultAccepted { action })
            },
            *QUERY_RESULT_RENDERER_STYLES,
        )
    }

    /// Returns the position ID for a query result at `index`.
    fn query_result_save_position_id(index: usize) -> String {
        format!("ai_context_menu:query_result:{index}")
    }

    pub fn close(&mut self, ctx: &mut ViewContext<Self>) {
        self.core.reset_results_progress();
        let query_length = self.query(ctx).len();
        let item_count = self.item_count(ctx);
        self.core.return_to_main_menu_if_multiple_categories(ctx);
        ctx.emit(AIContextMenuEvent::Close {
            query_length,
            item_count,
        });
        ctx.notify();
    }

    /// Reset the menu to the main menu state only if there are more than 1 available categories.
    pub fn reset_menu_state(&mut self, ctx: &mut ViewContext<Self>) {
        if self.core.return_to_main_menu_if_multiple_categories(ctx) {
            self.core.clear_category_filter();
            self.reset_mixer(ctx);
            ctx.notify();
        }
    }

    pub fn update_search_query(&mut self, query: String, _ctx: &mut ViewContext<Self>) {
        // Send the query through the debounce channel instead of updating directly
        let _ = self.search_debounce_tx.try_send(query);
    }

    /// Internal method called by the debounce system to actually update the search
    fn update_search_query_internal(&mut self, query: String, ctx: &mut ViewContext<Self>) {
        let results_are_empty = self
            .search_bar_state
            .as_ref(ctx)
            .query_result_renderers()
            .map(|results| results.is_empty())
            .unwrap_or_default();

        // While the category list is showing, the query filters category names.
        // When nothing matches, the core falls through to searching every
        // category, whose sources have to be installed before the query runs.
        if self.core.set_query(&query, ctx) == AtContextMenuQueryTransition::EnteredAllCategories {
            self.setup_data_sources_for_all_categories(&query, ctx);
        }

        let is_loading = self.mixer.as_ref(ctx).is_loading();
        // A query that still contains the previous one means the user narrowed
        // rather than rewrote, which is what the no-progress counter tracks.
        let query_grew = query.contains(&self.query(ctx));
        let should_close =
            self.core
                .record_results_update(results_are_empty, is_loading, query_grew);

        self.search_bar.update(ctx, |search_bar, ctx| {
            search_bar.set_query(query, ctx);
        });

        if should_close {
            self.close(ctx);
        }
    }

    fn query(&self, ctx: &ViewContext<Self>) -> String {
        self.search_bar.as_ref(ctx).query(ctx)
    }

    fn item_count(&self, ctx: &ViewContext<Self>) -> Option<usize> {
        self.search_bar_state
            .as_ref(ctx)
            .query_result_renderers()
            .map(|results| results.len())
    }

    /// Scrolls the query result at `index` into view.
    fn scroll_selected_index_into_view(&self, index: usize, ctx: &mut ViewContext<Self>) {
        self.state.uniform_list_state.scroll_to(index);
        ctx.notify();
    }

    fn reset_mixer(&mut self, ctx: &mut ViewContext<Self>) {
        let navigation_state = self.core.navigation_state();
        let source_context = self.source_context();
        self.mixer.update(ctx, |mixer, ctx| {
            mixer.reset(ctx);
            // The category list filters locally and needs no sources, and the
            // all-categories search installs its own set alongside the query
            // that triggered the fall-through.
            let NavigationState::Category(category) = navigation_state else {
                return;
            };
            install_sources_for_category(mixer, category, &source_context, ctx);
            mixer.run_query(at_context_menu_query(""), ctx);
        });
    }

    /// Installs the sources for every available category and runs `query`
    /// across all of them.
    fn setup_data_sources_for_all_categories(&mut self, query: &str, ctx: &mut ViewContext<Self>) {
        // Searching across every category does not include current-folder files:
        // outside a git repository, file results come from the dedicated files
        // category only.
        let categories: Vec<_> = self
            .core
            .available_categories(ctx)
            .into_iter()
            .filter(|category| *category != AIContextMenuCategory::CurrentFolderFiles)
            .collect();
        let source_context = self.source_context();
        let query = query.to_owned();
        self.mixer.update(ctx, |mixer, ctx| {
            mixer.reset(ctx);
            install_sources_for_all_categories(mixer, &categories, &source_context, ctx);
            mixer.run_query(at_context_menu_query(&query), ctx);
        });
    }

    /// Long-lived handles the menu's data sources need, resolved once per
    /// installation.
    fn source_context(&self) -> AtContextMenuSourceContext {
        AtContextMenuSourceContext {
            #[cfg(not(target_family = "wasm"))]
            code_symbol_cache: Some(self.code_symbol_cache.clone()),
            #[cfg(target_family = "wasm")]
            code_symbol_cache: None,
        }
    }

    fn handle_search_bar_event(
        &mut self,
        event: &SearchBarEvent<AIContextMenuSearchableAction>,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SearchBarEvent::ResultSelected { index } => {
                self.scroll_selected_index_into_view(*index, ctx);
            }
            SearchBarEvent::ResultAccepted { action, .. } => {
                self.handle_action(
                    &AIContextMenuAction::ResultAccepted {
                        action: action.clone(),
                    },
                    ctx,
                );
            }
            SearchBarEvent::Close => {
                self.handle_action(&AIContextMenuAction::Close, ctx);
            }
            // All other events we can ignore
            _ => {}
        }
    }

    fn render_main_menu(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut flex = Flex::column();

        // Get filtered categories based on the current query
        let filtered_categories = self.core.filtered_categories(app);

        // If no categories match the filter, show "No results found"
        // Ideally we don't enter this state because we transition to AllCategories mode
        // when no categories match.
        if filtered_categories.is_empty() {
            return self.render_no_results(app);
        }

        let last_display_index = filtered_categories.len().saturating_sub(1);
        for (display_index, category) in filtered_categories.iter().enumerate() {
            let is_selected = display_index == self.core.selected_category_index();
            let is_first = display_index == 0;
            let is_last = display_index == last_display_index;
            let text_color = if is_selected {
                theme.main_text_color(theme.accent()).into_solid()
            } else {
                theme.main_text_color(theme.background()).into_solid()
            };

            let icon = ConstrainedBox::new(Icon::new(category.icon(), text_color).finish())
                .with_width(styles::ICON_SIZE)
                .with_height(styles::ICON_SIZE)
                .finish();

            let text = Container::new(
                Text::new(
                    category.name(),
                    appearance.ui_font_family(),
                    appearance.monospace_font_size() - 1.0,
                )
                .with_color(text_color)
                .finish(),
            )
            .with_horizontal_padding(8.)
            .finish();

            let chevron = ConstrainedBox::new(
                Icon::new("bundled/svg/chevron-right.svg", text_color).finish(),
            )
            .with_width(styles::ICON_SIZE)
            .with_height(styles::ICON_SIZE)
            .finish();

            let row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(icon)
                .with_child(text)
                .with_child(Shrinkable::new(1.0, Empty::new().finish()).finish())
                .with_child(chevron)
                .finish();

            // Find the original index of this category in current categories for hover state
            let categories = self.core.available_categories(app);
            let original_index = categories.iter().position(|c| *c == *category).unwrap_or(0);
            let hover_state = self
                .state
                .category_hover_states
                .get(original_index)
                .cloned()
                .unwrap_or_default();

            // Extract theme colors outside the closure to avoid lifetime issues
            let accent_color = theme.accent();
            let accent_overlay_color = theme.accent_overlay();

            let highlight_radius = Radius::Pixels(styles::MENU_ITEM_HIGHLIGHT_CORNER_RADIUS);
            let highlight_corner_radius = match (is_first, is_last) {
                (true, true) => CornerRadius::with_all(highlight_radius),
                (true, false) => CornerRadius::with_top(highlight_radius),
                (false, true) => CornerRadius::with_bottom(highlight_radius),
                (false, false) => CornerRadius::default(),
            };

            let category_clone_for_click = *category;
            let category_row = Hoverable::new(hover_state, move |hover_state| {
                let mut container = Container::new(row)
                    .with_horizontal_padding(styles::MENU_ITEM_HORIZONTAL_PADDING)
                    .with_vertical_padding(styles::MENU_ITEM_VERTICAL_PADDING)
                    .with_corner_radius(highlight_corner_radius);
                if is_selected {
                    container = container.with_background(accent_color);
                } else if hover_state.is_hovered() {
                    container = container.with_background(accent_overlay_color);
                }
                container.finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(AIContextMenuAction::CategorySelected {
                    category: category_clone_for_click,
                });
            })
            .finish();

            flex.add_child(category_row);
        }
        flex.finish()
    }

    fn render_no_results(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        Container::new(
            Text::new(
                "No results found",
                appearance.ui_font_family(),
                appearance.monospace_font_size(),
            )
            .with_color(theme.main_text_color(theme.background()).into_solid())
            .finish(),
        )
        .with_uniform_padding(PADDING)
        .finish()
    }

    fn render_loading_results(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        Container::new(
            Text::new(
                "Loading results...",
                appearance.ui_font_family(),
                appearance.monospace_font_size(),
            )
            .with_color(theme.main_text_color(theme.background()).into_solid())
            .finish(),
        )
        .with_uniform_padding(PADDING)
        .finish()
    }

    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    fn render_code_symbols_indexing(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        Container::new(
            Text::new(
                "Code symbols indexing...",
                appearance.ui_font_family(),
                appearance.monospace_font_size(),
            )
            .with_color(theme.main_text_color(theme.background()).into_solid())
            .finish(),
        )
        .with_uniform_padding(PADDING)
        .finish()
    }

    fn render_matching_results(
        &self,
        selected_index: Option<usize>,
        query_result_renderers: &[QueryResultRenderer<AIContextMenuSearchableAction>],
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let view_handle = self.handle.clone();
        let build_items = move |range: Range<usize>, app: &AppContext| {
            let context_menu = view_handle
                .upgrade(app)
                .expect("View handle should be upgradeable.");
            let context_menu_ref = context_menu.as_ref(app);
            let query_result_renderers = context_menu_ref
                .search_bar_state
                .as_ref(app)
                .query_result_renderers();

            match query_result_renderers {
                Some(query_result_renderers) => {
                    let query_result_iter = if range.end == 1 {
                        // Despite being upper-bound exclusive, taking a slice where
                        // the end of the range is out of bounds results in a panic.
                        query_result_renderers[range.start..].iter()
                    } else {
                        query_result_renderers[range.start..range.end].iter()
                    };
                    query_result_iter
                        .enumerate()
                        .map(|(result_index, result_renderer)| {
                            let result_index = result_index + range.start;
                            SavePosition::new(
                                result_renderer.render(
                                    result_index,
                                    selected_index == Some(result_index),
                                    app,
                                ),
                                result_renderer.position_id.as_str(),
                            )
                            .finish()
                        })
                        .collect_vec()
                        .into_iter()
                }
                None => Vec::new().into_iter(),
            }
        };

        let max_height: f32 = MAX_DISPLAYED_RESULT_COUNT as f32 * styles::ESTIMATED_RESULT_HEIGHT;
        ConstrainedBox::new(
            Scrollable::vertical(
                self.state.scroll_state.clone(),
                UniformList::new(
                    self.state.uniform_list_state.clone(),
                    query_result_renderers.len(),
                    build_items,
                )
                .finish_scrollable(),
                ScrollbarWidth::Auto,
                theme.disabled_text_color(theme.surface_2()).into(),
                theme.main_text_color(theme.surface_2()).into(),
                Fill::None,
            )
            .finish(),
        )
        .with_max_height(max_height)
        .finish()
    }

    /// Whether the AI context menu should render.
    #[cfg(not(target_family = "wasm"))]
    pub fn should_render(&self, app: &AppContext) -> bool {
        !self.core.available_categories(app).is_empty()
    }

    #[cfg(target_family = "wasm")]
    pub fn should_render(&self, _app: &AppContext) -> bool {
        false
    }

    /// Returns the selected result renderer, if any.
    fn selected_result_renderer<'a>(
        &self,
        app: &'a AppContext,
    ) -> Option<&'a QueryResultRenderer<AIContextMenuSearchableAction>> {
        self.search_bar_state.as_ref(app).selected_result_renderer()
    }

    /// Returns the positioning for the details panel relative to the selected item.
    /// If there isn't enough space to render to the right, returns None so the details panel doesn't render.
    fn offset_positioning_for_details_panel(&self, app: &AppContext) -> Option<OffsetPositioning> {
        let _selected_index = self.search_bar_state.as_ref(app).selected_index()?;
        let selected_result_renderer = self.selected_result_renderer(app)?;

        // Use positioning logic similar to command search - render to the right with space checking
        let x_axis_positioning = PositioningAxis::relative_to_stack_child(
            PANEL_POSITION_ID,
            PositionedElementOffsetBounds::WindowBySize, // This enforces space constraints
            OffsetType::Pixel(DETAILS_PANEL_MARGIN),
            AnchorPair::new(XAxisAnchor::Right, XAxisAnchor::Left),
        );

        // Position vertically aligned with the selected result
        let y_axis_positioning = PositioningAxis::relative_to_stack_child(
            selected_result_renderer.position_id.clone(),
            PositionedElementOffsetBounds::WindowByPosition,
            OffsetType::Pixel(0.),
            AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Top),
        );

        Some(OffsetPositioning::from_axes(
            x_axis_positioning,
            y_axis_positioning,
        ))
    }

    fn render_category_view(
        &self,
        category: AIContextMenuCategory,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let state = self.search_bar_state.as_ref(app);
        let selected_index = state.selected_index();
        let query_result_renderers = state.query_result_renderers();

        let mut column = Flex::column();

        let title = Container::new(
            Text::new(
                category.name(),
                appearance.ui_font_family(),
                appearance.monospace_font_size() - 2.0,
            )
            .with_color(theme.disabled_text_color(theme.background()).into_solid())
            .finish(),
        )
        .with_vertical_padding(4.)
        .with_horizontal_padding(10.0)
        .finish();

        // Only show the title if there are multiple categories
        if self.core.available_categories(app).len() > 1 {
            column.add_child(title);
        }

        column.add_child(match query_result_renderers {
            Some(query_result_renderers) if query_result_renderers.is_empty() => {
                self.render_empty_state(Some(category), self.render_no_results(app), app)
            }
            Some(query_result_renderers) => {
                self.render_matching_results(selected_index, query_result_renderers, app)
            }
            None => self.render_empty_state(Some(category), Empty::new().finish(), app),
        });

        column.finish()
    }

    fn render_all_categories_view(&self, app: &AppContext) -> Box<dyn Element> {
        let state = self.search_bar_state.as_ref(app);
        let selected_index = state.selected_index();
        let query_result_renderers = state.query_result_renderers();

        let mut column = Flex::column();

        column.add_child(match query_result_renderers {
            Some(query_result_renderers) if query_result_renderers.is_empty() => {
                self.render_empty_state(None, self.render_no_results(app), app)
            }
            Some(query_result_renderers) => {
                self.render_matching_results(selected_index, query_result_renderers, app)
            }
            None => self.render_empty_state(None, Empty::new().finish(), app),
        });

        column.finish()
    }

    /// Renders the appropriate empty-state element: code-symbols-indexing
    /// indicator (when applicable), loading spinner, or the provided fallback.
    #[cfg_attr(target_family = "wasm", allow(unused_variables))]
    fn render_empty_state(
        &self,
        category: Option<AIContextMenuCategory>,
        fallback: Box<dyn Element>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        #[cfg(not(target_family = "wasm"))]
        if let Some(cat) = category
            && cat == AIContextMenuCategory::Code
            && is_code_symbols_indexing(app)
        {
            return self.render_code_symbols_indexing(app);
        }

        if self.mixer.as_ref(app).is_loading() {
            self.render_loading_results(app)
        } else {
            fallback
        }
    }

    #[allow(dead_code)]
    fn render_search_bar(&self) -> Box<dyn Element> {
        ChildView::new(&self.search_bar).finish()
    }
}

impl View for AIContextMenu {
    fn ui_name() -> &'static str {
        "AIContextMenuView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let body = match self.core.navigation_state() {
            NavigationState::MainMenu => self.render_main_menu(app),
            NavigationState::Category(category) => self.render_category_view(category, app),
            NavigationState::AllCategories => self.render_all_categories_view(app),
        };

        let mut context_menu = Flex::column();

        context_menu.add_child(body);
        let scalar = appearance.monospace_ui_scalar();

        // Create the main container with SavePosition for positioning reference
        let main_container = SavePosition::new(
            ConstrainedBox::new(
                Container::new(context_menu.finish())
                    .with_background(theme.surface_2())
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CORNER_RADIUS)))
                    .with_border(Border::all(1.0).with_border_fill(theme.outline()))
                    .with_drop_shadow(QUERY_RESULT_RENDERER_STYLES.panel_drop_shadow)
                    .finish(),
            )
            .with_width(DEFAULT_PALETTE_WIDTH * scalar)
            .with_max_height(PALETTE_HEIGHT)
            .finish(),
            PANEL_POSITION_ID,
        )
        .finish();

        // Create a stack to enable overlay details panel
        let mut stack = Stack::new();
        stack.add_child(main_container);

        // Add details panel overlay if there's a selected result
        if !matches!(self.core.navigation_state(), NavigationState::MainMenu)
            && let (Some(selected_result_renderer), Some(details_panel_positioning)) = (
                self.selected_result_renderer(app),
                self.offset_positioning_for_details_panel(app),
            )
            && let Some(details) = selected_result_renderer.render_details(app)
        {
            // QueryResultRenderer already applies styling, padding, border, etc.
            // Just add some margin for spacing from the main menu
            stack.add_positioned_overlay_child(
                Container::new(details)
                    .with_margin_bottom(DETAILS_PANEL_MARGIN)
                    .with_margin_right(DETAILS_PANEL_MARGIN)
                    .finish(),
                details_panel_positioning,
            );
        }

        // Use proper keybinding handling instead of event handlers
        Dismiss::new(stack.finish())
            .on_dismiss(|ctx, _app| ctx.dispatch_typed_action(AIContextMenuAction::Close))
            .prevent_interaction_with_other_elements()
            .finish()
    }
}
