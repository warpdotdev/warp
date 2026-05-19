use crate::appearance::Appearance;
use crate::debounce::debounce;
use crate::drive::settings::WarpDriveSettings;
#[cfg(not(target_family = "wasm"))]
use crate::search::ai_context_menu::blocks::data_source::BlockDataSource;
#[cfg(not(target_family = "wasm"))]
use crate::search::ai_context_menu::code::data_source::{code_data_source, CodeSymbolCache};
#[cfg(not(target_family = "wasm"))]
use crate::search::ai_context_menu::code::is_code_symbols_indexing;
#[cfg(not(target_family = "wasm"))]
use crate::search::ai_context_menu::commands::data_source::CommandDataSource;
use crate::search::ai_context_menu::conversations::data_source::ConversationDataSource;
#[cfg(not(target_family = "wasm"))]
use crate::search::ai_context_menu::diffset::data_source::DiffSetDataSource;
#[cfg(not(target_family = "wasm"))]
use crate::search::ai_context_menu::files::data_source::{
    file_data_source_for_current_repo, file_data_source_for_pwd,
};
use crate::search::ai_context_menu::mixer::AIContextMenuMixer;
use crate::search::ai_context_menu::mixer::AIContextMenuSearchableAction;
#[cfg(not(target_family = "wasm"))]
use crate::search::ai_context_menu::notebooks::data_source::NotebookDataSource;
#[cfg(not(target_family = "wasm"))]
use crate::search::ai_context_menu::rules::data_source::RulesDataSource;
#[cfg(not(target_family = "wasm"))]
use crate::search::ai_context_menu::skills::data_source::SkillsDataSource;
#[cfg(not(target_family = "wasm"))]
use crate::search::ai_context_menu::workflows::data_source::WorkflowDataSource;
use crate::search::data_source::QueryResult;
use crate::search::data_source::{Query, QueryFilter};
#[cfg(not(target_family = "wasm"))]
use crate::search::mixer::AddAsyncSourceOptions;
use crate::search::result_renderer::{QueryResultRenderer, QueryResultRendererStyles};
use crate::search::search_bar::{SearchBar, SearchBarEvent, SearchBarState, SearchResultOrdering};
use crate::settings::InputSettings;
use async_channel::Sender;
use itertools::Itertools;
use settings::Setting as _;
use std::collections::HashSet;
use std::ops::Range;
use std::time::Duration;
use warp_core::features::FeatureFlag;
use warpui::elements::ConstrainedBox;
use warpui::elements::CrossAxisAlignment;
use warpui::elements::Empty;
use warpui::elements::Fill;
use warpui::elements::Hoverable;
use warpui::elements::MouseStateHandle;
use warpui::elements::ScrollStateHandle;
use warpui::elements::Scrollable;
use warpui::elements::ScrollableElement;
use warpui::elements::ScrollbarWidth;
use warpui::elements::UniformList;
use warpui::elements::UniformListState;
use warpui::elements::{
    AnchorPair, Border, ChildView, Container, CornerRadius, Dismiss, Flex, Icon, OffsetPositioning,
    OffsetType, ParentElement, PositionedElementOffsetBounds, PositioningAxis, Radius,
    SavePosition, Shrinkable, Stack, Text, XAxisAnchor, YAxisAnchor,
};

use warpui::platform::Cursor;
use warpui::windowing::WindowManager;
use warpui::SingletonEntity;
use warpui::View;
use warpui::{
    AppContext, Element, Entity, ModelHandle, TypedActionView, ViewContext, ViewHandle,
    WeakViewHandle,
};

#[cfg(not(target_family = "wasm"))]
use crate::workspace::ActiveSession;
#[cfg(not(target_family = "wasm"))]
use repo_metadata::repositories::DetectedRepositories;
#[cfg(not(target_family = "wasm"))]
use std::path::Path;
#[cfg(not(target_family = "wasm"))]
use warp_util::local_or_remote_path::LocalOrRemotePath;

use super::styles;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AIContextMenuCategory {
    CurrentFolderFiles,
    RepoFiles,
    Commands,
    Blocks,
    Workflows,
    Notebooks,
    Plans,
    Diffs,
    Docs,
    Tasks,
    Rules,
    Servers,
    Terminal,
    Web,
    RecentDiff,
    RecentBlock,
    Code,
    DiffSet,
    Conversations,
    Skills,
}

impl AIContextMenuCategory {
    pub fn name(&self) -> &'static str {
        match self {
            AIContextMenuCategory::CurrentFolderFiles => "Files and folders",
            AIContextMenuCategory::RepoFiles => "Files and folders",
            AIContextMenuCategory::Commands => "Commands",
            AIContextMenuCategory::Blocks => "Blocks",
            AIContextMenuCategory::Workflows => "Workflows",
            AIContextMenuCategory::Notebooks => "Notebooks",
            AIContextMenuCategory::Plans => "Plans",
            AIContextMenuCategory::Diffs => "Diffs",
            AIContextMenuCategory::Docs => "Docs",
            AIContextMenuCategory::Tasks => "Past tasks",
            AIContextMenuCategory::Rules => "Rules",
            AIContextMenuCategory::Servers => "Servers and integrations",
            AIContextMenuCategory::Terminal => "Terminal",
            AIContextMenuCategory::Web => "Web",
            AIContextMenuCategory::RecentDiff => "Most recent diff",
            AIContextMenuCategory::RecentBlock => "Most recent block",
            AIContextMenuCategory::Code => "Code",
            AIContextMenuCategory::DiffSet => "Diff sets",
            AIContextMenuCategory::Conversations => "Conversations",
            AIContextMenuCategory::Skills => "Skills",
        }
    }

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

/// The different navigation states for the AI context menu.
#[derive(Debug, Clone)]
pub enum NavigationState {
    /// The main menu showing all categories.
    MainMenu,
    /// Viewing items from a specific category.
    Category(AIContextMenuCategory),
    /// Viewing search results from all categories combined.
    AllCategories,
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

/// View state for the AI context menu.
struct AIContextMenuState {
    /// The current navigation state.
    navigation_state: NavigationState,
    scroll_state: ScrollStateHandle,
    uniform_list_state: UniformListState,
    category_hover_states: Vec<MouseStateHandle>,
    /// Selected category index in main menu
    selected_category_index: usize,
    /// Current query for filtering categories in main menu
    main_menu_query: String,
    /// Whether we're in AI/autodetect mode (true) or locked in terminal mode (false)
    is_ai_or_autodetect_mode: bool,
    /// Whether this terminal is viewing a shared session
    is_shared_session_viewer: bool,
    /// Whether this terminal is in an ambient agent session
    is_in_ambient_agent: bool,
    /// Whether this is a CLI agent rich input (restricts categories to files/folders + code)
    is_cli_agent_input: bool,
}

/// Maximum number of results to display
const MAX_SEARCH_RESULTS: usize = 250;
const MAX_CONSECUTIVE_EMPTY_RESULTS_EVENTS: usize = 7;

/// AI Context Menu View
pub struct AIContextMenu {
    mixer: ModelHandle<AIContextMenuMixer>,
    /// While we aren't rendering a search bar, the view contains
    /// a lot of helpful logic for managing the search state.
    search_bar: ViewHandle<SearchBar<AIContextMenuSearchableAction>>,
    search_bar_state: ModelHandle<SearchBarState<AIContextMenuSearchableAction>>,
    #[cfg(not(target_family = "wasm"))]
    code_symbol_cache: ModelHandle<CodeSymbolCache>,
    state: AIContextMenuState,
    /// Debounce channel for search queries
    search_debounce_tx: Sender<String>,
    handle: WeakViewHandle<Self>,
    /// Keep track of how many times in a row
    /// we had 0 matches and the user continued to make the query longer.
    /// We can use this to close the menu if the user doesn't make any progress.
    num_consecutive_empty_results_events: usize,
}

impl AIContextMenu {
    pub fn set_is_shared_session_viewer(&mut self, is_viewer: bool, ctx: &mut ViewContext<Self>) {
        if self.state.is_shared_session_viewer != is_viewer {
            self.state.is_shared_session_viewer = is_viewer;
            self.refresh_categories_state(ctx);
        }
    }

    pub fn set_is_in_ambient_agent(&mut self, is_ambient: bool, ctx: &mut ViewContext<Self>) {
        if self.state.is_in_ambient_agent != is_ambient {
            self.state.is_in_ambient_agent = is_ambient;
            self.refresh_categories_state(ctx);
        }
    }

    pub fn set_is_cli_agent_input(
        &mut self,
        is_cli_agent_input: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.state.is_cli_agent_input != is_cli_agent_input {
            self.state.is_cli_agent_input = is_cli_agent_input;
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
            AIContextMenuAction::Prev => {
                match &self.state.navigation_state {
                    NavigationState::MainMenu => {
                        // Navigate up in filtered categories
                        let filtered_categories = self.get_filtered_categories(ctx);
                        if !filtered_categories.is_empty() {
                            if self.state.selected_category_index > 0 {
                                self.state.selected_category_index -= 1;
                            } else {
                                self.state.selected_category_index = filtered_categories.len() - 1;
                            }
                        }
                        ctx.notify();
                    }
                    NavigationState::Category(_) | NavigationState::AllCategories => {
                        // Navigate up in search results
                        self.search_bar.update(ctx, |search_bar, ctx| {
                            search_bar.up(ctx);
                        });
                    }
                }
            }
            AIContextMenuAction::Next => {
                match &self.state.navigation_state {
                    NavigationState::MainMenu => {
                        // Navigate down in filtered categories
                        let filtered_categories = self.get_filtered_categories(ctx);
                        if !filtered_categories.is_empty() {
                            if self.state.selected_category_index < filtered_categories.len() - 1 {
                                self.state.selected_category_index += 1;
                            } else {
                                self.state.selected_category_index = 0;
                            }
                        }
                        ctx.notify();
                    }
                    NavigationState::Category(_) | NavigationState::AllCategories => {
                        // Navigate down in search results
                        self.search_bar.update(ctx, |search_bar, ctx| {
                            search_bar.down(ctx);
                        });
                    }
                }
            }
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
                self.state.navigation_state = NavigationState::Category(*category);
                self.state.main_menu_query = String::new();
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
    /// Get the appropriate categories based on the current input mode
    /// If is_ai_or_autodetect_mode is true, return all AI categories
    /// If false (locked in terminal mode), return only Files category
    pub(crate) fn get_categories_for_mode(
        is_ai_or_autodetect_mode: bool,
        is_shared_session_viewer: bool,
        is_in_ambient_agent: bool,
        is_cli_agent_input: bool,
        app: &AppContext,
    ) -> Vec<AIContextMenuCategory> {
        let show_warp_drive = WarpDriveSettings::is_warp_drive_enabled(app);

        // Compute once — used by CLI agent, AI-mode, and terminal-mode branches.
        let is_active_dir_in_git_repo = {
            #[cfg(target_family = "wasm")]
            {
                false
            }

            #[cfg(not(target_family = "wasm"))]
            {
                let active_window_id = app.windows().state().active_window;
                let active_dir = active_window_id
                    .and_then(|window_id| ActiveSession::as_ref(app).path_if_local(window_id));
                active_dir.is_some_and(|dir| {
                    DetectedRepositories::as_ref(app)
                        .get_root_for_path(&LocalOrRemotePath::Local(Path::new(dir).to_path_buf()))
                        .is_some()
                })
            }
        };

        // For CLI agent input, use a positive allowlist of categories that CLI agents
        // can interpret. This is safer than a blocklist because new categories added
        // to the enum in the future won't accidentally leak into the CLI agent menu.
        if is_cli_agent_input {
            let mut categories = vec![];
            if !is_shared_session_viewer {
                if is_active_dir_in_git_repo {
                    categories.push(AIContextMenuCategory::RepoFiles);
                } else {
                    categories.push(AIContextMenuCategory::CurrentFolderFiles);
                }
            }
            if FeatureFlag::AIContextMenuCode.is_enabled()
                && *InputSettings::as_ref(app)
                    .outline_codebase_symbols_for_at_context_menu
                    .value()
                && is_active_dir_in_git_repo
                && !is_shared_session_viewer
            {
                categories.push(AIContextMenuCategory::Code);
            }
            return categories;
        }

        // For ambient agent sessions, only show limited categories
        if is_in_ambient_agent {
            let mut categories = vec![];
            if show_warp_drive {
                if FeatureFlag::DriveObjectsAsContext.is_enabled() {
                    categories.push(AIContextMenuCategory::Workflows);
                    categories.push(AIContextMenuCategory::Notebooks);
                    categories.push(AIContextMenuCategory::Plans);
                }
                categories.push(AIContextMenuCategory::Rules);
            }
            return categories;
        }

        if is_ai_or_autodetect_mode {
            let mut categories = vec![];

            // Hide file options for shared session viewers
            if !is_shared_session_viewer {
                if is_active_dir_in_git_repo {
                    categories.push(AIContextMenuCategory::RepoFiles);
                } else {
                    categories.push(AIContextMenuCategory::CurrentFolderFiles);
                }
            }

            if FeatureFlag::AIContextMenuCommands.is_enabled() {
                categories.push(AIContextMenuCategory::Commands);
            }
            categories.push(AIContextMenuCategory::Blocks);
            if FeatureFlag::AIContextMenuCode.is_enabled()
                && *InputSettings::as_ref(app)
                    .outline_codebase_symbols_for_at_context_menu
                    .value()
                && is_active_dir_in_git_repo
                && !is_shared_session_viewer
            {
                categories.push(AIContextMenuCategory::Code);
            }
            if show_warp_drive && FeatureFlag::DriveObjectsAsContext.is_enabled() {
                categories.push(AIContextMenuCategory::Workflows);
                categories.push(AIContextMenuCategory::Notebooks);
                categories.push(AIContextMenuCategory::Plans);
            }
            if FeatureFlag::DiffSetAsContext.is_enabled()
                && is_active_dir_in_git_repo
                && !is_shared_session_viewer
            {
                categories.push(AIContextMenuCategory::DiffSet);
            }
            if FeatureFlag::ConversationsAsContext.is_enabled() {
                categories.push(AIContextMenuCategory::Conversations);
            }
            if show_warp_drive {
                categories.push(AIContextMenuCategory::Rules);
            }
            categories.push(AIContextMenuCategory::Skills);
            categories
        } else if !is_shared_session_viewer {
            // Terminal mode: show Files and Code categories (when enabled)
            let mut categories = if is_active_dir_in_git_repo {
                vec![AIContextMenuCategory::RepoFiles]
            } else {
                vec![AIContextMenuCategory::CurrentFolderFiles]
            };

            // Also show Code category in terminal mode when enabled
            if FeatureFlag::AIContextMenuCode.is_enabled()
                && *InputSettings::as_ref(app)
                    .outline_codebase_symbols_for_at_context_menu
                    .value()
                && is_active_dir_in_git_repo
            {
                categories.push(AIContextMenuCategory::Code);
            }

            categories
        } else {
            // File searching is not available in shared session viewers
            vec![]
        }
    }

    /// Set the input mode and update the menu state accordingly
    pub fn set_input_mode(&mut self, is_ai_or_autodetect_mode: bool, ctx: &mut ViewContext<Self>) {
        if self.state.is_ai_or_autodetect_mode != is_ai_or_autodetect_mode {
            self.state.is_ai_or_autodetect_mode = is_ai_or_autodetect_mode;
            self.refresh_categories_state(ctx);
        }
    }

    /// Recompute category-dependent state when repository availability changes.
    fn refresh_categories_state(&mut self, ctx: &mut ViewContext<Self>) {
        let categories = Self::get_categories_for_mode(
            self.state.is_ai_or_autodetect_mode,
            self.state.is_shared_session_viewer,
            self.state.is_in_ambient_agent,
            self.state.is_cli_agent_input,
            ctx,
        );

        // Reset to appropriate initial state based on categories available
        if categories.is_empty() || categories.len() > 1 {
            // If there are zero or more than one categories, go to main menu
            self.state.navigation_state = NavigationState::MainMenu;
        } else {
            // Only one category, go directly to it
            self.state.navigation_state = NavigationState::Category(categories[0]);
        }

        // Update category hover states for the new set of categories
        self.state.category_hover_states = categories.iter().map(|_| Default::default()).collect();

        // Reset selection and query
        self.state.selected_category_index = 0;
        self.state.main_menu_query = String::new();

        // Reset mixer with new category configuration
        self.reset_mixer(ctx);

        ctx.notify();
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

        // Get initial categories for proper initialization
        let initial_categories = Self::get_categories_for_mode(true, false, false, false, ctx); // Default to AI mode, not a viewer, not ambient agent, not CLI agent input

        #[cfg(not(target_family = "wasm"))]
        let code_symbol_cache = ctx.add_model(CodeSymbolCache::new);

        // When the outline updates (e.g. indexing finishes), re-run the current
        // mixer query so the Code results refresh automatically.
        #[cfg(not(target_family = "wasm"))]
        ctx.subscribe_to_model(&code_symbol_cache, |me, _handle, _event, ctx| {
            let code_active = matches!(
                me.state.navigation_state,
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
            state: AIContextMenuState {
                navigation_state: if initial_categories.len() > 1 {
                    NavigationState::MainMenu
                } else {
                    NavigationState::Category(AIContextMenuCategory::RepoFiles)
                },
                scroll_state: Default::default(),
                uniform_list_state: Default::default(),
                category_hover_states: initial_categories
                    .iter()
                    .map(|_| Default::default())
                    .collect(),
                selected_category_index: 0,
                main_menu_query: String::new(),
                is_ai_or_autodetect_mode: true,  // Default to AI mode
                is_shared_session_viewer: false, // Will be updated by set_is_shared_session_viewer if needed
                is_in_ambient_agent: false, // Will be updated by set_is_in_ambient_agent if needed
                is_cli_agent_input: false,  // Will be updated by set_is_cli_agent_input if needed
            },
            handle: ctx.handle(),
            search_debounce_tx,
            num_consecutive_empty_results_events: 0,
        };

        result.reset_mixer(ctx);
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
        match &self.state.navigation_state {
            NavigationState::MainMenu => {
                // Select the current category from filtered categories
                let filtered_categories = self.get_filtered_categories(ctx);
                if let Some(category) = filtered_categories.get(self.state.selected_category_index)
                {
                    self.handle_action(
                        &AIContextMenuAction::CategorySelected {
                            category: *category,
                        },
                        ctx,
                    );
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
        self.num_consecutive_empty_results_events = 0;
        let query_length = self.query(ctx).len();
        let item_count = self.item_count(ctx);
        let categories = Self::get_categories_for_mode(
            self.state.is_ai_or_autodetect_mode,
            self.state.is_shared_session_viewer,
            self.state.is_in_ambient_agent,
            self.state.is_cli_agent_input,
            ctx,
        );
        if categories.len() > 1 {
            self.state.navigation_state = NavigationState::MainMenu;
        }
        ctx.emit(AIContextMenuEvent::Close {
            query_length,
            item_count,
        });
        ctx.notify();
    }

    /// Reset the menu to the main menu state only if there are more than 1 available categories.
    pub fn reset_menu_state(&mut self, ctx: &mut ViewContext<Self>) {
        let categories = Self::get_categories_for_mode(
            self.state.is_ai_or_autodetect_mode,
            self.state.is_shared_session_viewer,
            self.state.is_in_ambient_agent,
            self.state.is_cli_agent_input,
            ctx,
        );
        if categories.len() > 1 {
            self.state.navigation_state = NavigationState::MainMenu;
            self.state.main_menu_query = String::new();
            self.state.selected_category_index = 0;
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
        let is_empty = self
            .search_bar_state
            .as_ref(ctx)
            .query_result_renderers()
            .map(|results| results.is_empty())
            .unwrap_or_default();

        // Handle navigation state transitions based on query
        if let NavigationState::MainMenu = &self.state.navigation_state {
            self.state.main_menu_query = query.clone();
            // Update selected index based on filtered categories
            self.update_selected_category_for_filtered_view(ctx);

            // Check if no categories match and transition to AllCategories mode
            self.check_and_transition_to_all_categories(&query, ctx);
        }

        self.search_bar.update(ctx, |search_bar, ctx| {
            let prev_query = search_bar.query(ctx);
            // In MainMenu state the mixer has no data sources, so empty results
            // are expected. Skip the consecutive-empty-results counter to avoid
            // prematurely dismissing the menu while the user is still typing a
            // category filter.
            let is_main_menu = matches!(self.state.navigation_state, NavigationState::MainMenu);
            if !is_main_menu
                && is_empty
                && !self.mixer.as_ref(ctx).is_loading()
                && query.contains(&prev_query)
            {
                self.num_consecutive_empty_results_events += 1;
            } else {
                self.num_consecutive_empty_results_events = 0;
            }
            search_bar.set_query(query, ctx);
        });
        if self.num_consecutive_empty_results_events >= MAX_CONSECUTIVE_EMPTY_RESULTS_EVENTS {
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
        self.mixer.update(ctx, |mixer, ctx| {
            mixer.reset(ctx);
        });

        match self.state.navigation_state {
            NavigationState::MainMenu => {}
            #[cfg(not(target_family = "wasm"))]
            NavigationState::Category(AIContextMenuCategory::CurrentFolderFiles) => {
                self.mixer.update(ctx, |mixer, ctx| {
                    mixer.add_async_source(
                        file_data_source_for_pwd(ctx),
                        [QueryFilter::Files],
                        AddAsyncSourceOptions {
                            debounce_interval: Some(Duration::from_millis(50)),
                            run_in_zero_state: true,
                            run_when_unfiltered: true,
                        },
                        ctx,
                    );
                    mixer.run_query(
                        Query {
                            text: "".into(),
                            filters: HashSet::new(),
                        },
                        ctx,
                    );
                });
            }
            #[cfg(not(target_family = "wasm"))]
            NavigationState::Category(AIContextMenuCategory::RepoFiles) => {
                self.mixer.update(ctx, |mixer, ctx| {
                    mixer.add_async_source(
                        file_data_source_for_current_repo(),
                        [QueryFilter::Files],
                        AddAsyncSourceOptions {
                            debounce_interval: Some(Duration::from_millis(50)),
                            run_in_zero_state: true,
                            run_when_unfiltered: true,
                        },
                        ctx,
                    );
                    mixer.run_query(
                        Query {
                            text: "".into(),
                            filters: HashSet::new(),
                        },
                        ctx,
                    );
                });
            }
            #[cfg(not(target_family = "wasm"))]
            NavigationState::Category(AIContextMenuCategory::Commands) => {
                let command_data_source = ctx.add_model(|_| CommandDataSource::new());
                self.mixer.update(ctx, |mixer, ctx| {
                    mixer.add_sync_source(command_data_source, [QueryFilter::Commands]);
                    mixer.run_query(
                        Query {
                            text: "".into(),
                            filters: HashSet::new(),
                        },
                        ctx,
                    );
                });
            }
            #[cfg(not(target_family = "wasm"))]
            NavigationState::Category(AIContextMenuCategory::Blocks) => {
                let block_data_source = ctx.add_model(|_| BlockDataSource::new());
                self.mixer.update(ctx, |mixer, ctx| {
                    mixer.add_sync_source(block_data_source, [QueryFilter::Blocks]);
                    mixer.run_query(
                        Query {
                            text: "".into(),
                            filters: HashSet::new(),
                        },
                        ctx,
                    );
                });
            }
            #[cfg(not(target_family = "wasm"))]
            NavigationState::Category(AIContextMenuCategory::Code) => {
                self.mixer.update(ctx, |mixer, ctx| {
                    mixer.add_async_source(
                        code_data_source(self.code_symbol_cache.as_ref(ctx)),
                        [QueryFilter::Code],
                        AddAsyncSourceOptions {
                            debounce_interval: Some(Duration::from_millis(50)),
                            run_in_zero_state: true,
                            run_when_unfiltered: true,
                        },
                        ctx,
                    );
                    mixer.run_query(
                        Query {
                            text: "".into(),
                            filters: HashSet::new(),
                        },
                        ctx,
                    );
                });
            }
            #[cfg(not(target_family = "wasm"))]
            NavigationState::Category(AIContextMenuCategory::Workflows) => {
                let workflow_data_source = ctx.add_model(|_| WorkflowDataSource::new());
                self.mixer.update(ctx, |mixer, ctx| {
                    mixer.add_sync_source(workflow_data_source, [QueryFilter::Workflows]);
                    mixer.run_query(
                        Query {
                            text: "".into(),
                            filters: HashSet::new(),
                        },
                        ctx,
                    );
                });
            }
            #[cfg(not(target_family = "wasm"))]
            NavigationState::Category(AIContextMenuCategory::Notebooks) => {
                let notebook_data_source = ctx.add_model(|_| NotebookDataSource::new(false));
                self.mixer.update(ctx, |mixer, ctx| {
                    mixer.add_sync_source(notebook_data_source, [QueryFilter::Notebooks]);
                    mixer.run_query(
                        Query {
                            text: "".into(),
                            filters: HashSet::new(),
                        },
                        ctx,
                    );
                });
            }
            #[cfg(not(target_family = "wasm"))]
            NavigationState::Category(AIContextMenuCategory::Plans) => {
                let notebook_data_source = ctx.add_model(|_| NotebookDataSource::new(true));
                self.mixer.update(ctx, |mixer, ctx| {
                    mixer.add_sync_source(notebook_data_source, [QueryFilter::Notebooks]);
                    mixer.run_query(
                        Query {
                            text: "".into(),
                            filters: HashSet::new(),
                        },
                        ctx,
                    );
                });
            }
            #[cfg(not(target_family = "wasm"))]
            NavigationState::Category(AIContextMenuCategory::Rules) => {
                let rules_data_source = ctx.add_model(|_| RulesDataSource::new());
                self.mixer.update(ctx, |mixer, ctx| {
                    mixer.add_sync_source(rules_data_source, [QueryFilter::Rules]);
                    mixer.run_query(
                        Query {
                            text: "".into(),
                            filters: HashSet::new(),
                        },
                        ctx,
                    );
                });
            }
            #[cfg(not(target_family = "wasm"))]
            NavigationState::Category(AIContextMenuCategory::DiffSet) => {
                let diffset_data_source = ctx.add_model(|_| DiffSetDataSource);
                self.mixer.update(ctx, |mixer, ctx| {
                    mixer.add_sync_source(diffset_data_source, [QueryFilter::DiffSets]);
                    mixer.run_query(
                        Query {
                            text: "".into(),
                            filters: HashSet::new(),
                        },
                        ctx,
                    );
                });
            }
            NavigationState::Category(AIContextMenuCategory::Conversations) => {
                let conversation_data_source = ctx.add_model(|_| ConversationDataSource);
                self.mixer.update(ctx, |mixer, ctx| {
                    mixer.add_sync_source(conversation_data_source, [QueryFilter::Conversations]);
                    mixer.run_query(
                        Query {
                            text: "".into(),
                            filters: HashSet::new(),
                        },
                        ctx,
                    );
                });
            }
            #[cfg(not(target_family = "wasm"))]
            NavigationState::Category(AIContextMenuCategory::Skills) => {
                let skills_data_source = ctx.add_model(|_| SkillsDataSource::new());
                self.mixer.update(ctx, |mixer, ctx| {
                    mixer.add_sync_source(skills_data_source, [QueryFilter::Skills]);
                    mixer.run_query(
                        Query {
                            text: "".into(),
                            filters: HashSet::new(),
                        },
                        ctx,
                    );
                });
            }
            NavigationState::Category(_) => {
                // TODO: Add other data sources
            }
            NavigationState::AllCategories => {
                // AllCategories state is only used when transitioning from query-based filtering
                // This method should not be called directly for AllCategories
                // Instead, setup_data_sources_for_all_categories should be used
            }
        }
    }

    /// Update the selected category index to ensure it's valid for the filtered view
    fn update_selected_category_for_filtered_view(&mut self, app: &AppContext) {
        let filtered_categories = self.get_filtered_categories(app);
        if filtered_categories.is_empty()
            || self.state.selected_category_index >= filtered_categories.len()
        {
            self.state.selected_category_index = 0;
        }
        // If the current selection is still valid, keep it
    }

    /// Check and transition to AllCategories if no categories match
    fn check_and_transition_to_all_categories(&mut self, query: &str, ctx: &mut ViewContext<Self>) {
        let filtered_categories = self.get_filtered_categories(ctx);
        if filtered_categories.is_empty() {
            // No categories match - transition to AllCategories mode
            self.state.navigation_state = NavigationState::AllCategories;
            // Set up data sources for all categories and run the query
            self.setup_data_sources_for_all_categories(query, ctx);
        }
    }

    /// Set up data sources for all available categories
    #[cfg(not(target_family = "wasm"))]
    fn setup_data_sources_for_all_categories(&mut self, query: &str, ctx: &mut ViewContext<Self>) {
        // Reset mixer first
        self.mixer.update(ctx, |mixer, ctx| {
            mixer.reset(ctx);
        });

        // Add all available data sources
        let categories = Self::get_categories_for_mode(
            self.state.is_ai_or_autodetect_mode,
            self.state.is_shared_session_viewer,
            self.state.is_in_ambient_agent,
            self.state.is_cli_agent_input,
            ctx,
        );
        for category in categories.iter() {
            match category {
                AIContextMenuCategory::RepoFiles => {
                    self.mixer.update(ctx, |mixer, ctx| {
                        mixer.add_async_source(
                            file_data_source_for_current_repo(),
                            [QueryFilter::Files],
                            AddAsyncSourceOptions {
                                debounce_interval: Some(Duration::from_millis(50)),
                                run_in_zero_state: true,
                                run_when_unfiltered: true,
                            },
                            ctx,
                        );
                    });
                }
                AIContextMenuCategory::Commands => {
                    let command_data_source = ctx.add_model(|_| CommandDataSource::new());
                    self.mixer.update(ctx, |mixer, _ctx| {
                        mixer.add_sync_source(command_data_source, [QueryFilter::Commands]);
                    });
                }
                AIContextMenuCategory::Blocks => {
                    let block_data_source = ctx.add_model(|_| BlockDataSource::new());
                    self.mixer.update(ctx, |mixer, _ctx| {
                        mixer.add_sync_source(block_data_source, [QueryFilter::Blocks]);
                    });
                }
                AIContextMenuCategory::Code => {
                    self.mixer.update(ctx, |mixer, ctx| {
                        mixer.add_async_source(
                            code_data_source(self.code_symbol_cache.as_ref(ctx)),
                            [QueryFilter::Code],
                            AddAsyncSourceOptions {
                                debounce_interval: Some(Duration::from_millis(50)),
                                run_in_zero_state: true,
                                run_when_unfiltered: true,
                            },
                            ctx,
                        );
                    });
                }
                AIContextMenuCategory::Workflows => {
                    let workflow_data_source = ctx.add_model(|_| WorkflowDataSource::new());
                    self.mixer.update(ctx, |mixer, _ctx| {
                        mixer.add_sync_source(workflow_data_source, [QueryFilter::Workflows]);
                    });
                }
                AIContextMenuCategory::Notebooks => {
                    let notebook_data_source = ctx.add_model(|_| NotebookDataSource::new(false));
                    self.mixer.update(ctx, |mixer, _ctx| {
                        mixer.add_sync_source(notebook_data_source, [QueryFilter::Notebooks]);
                    });
                }
                AIContextMenuCategory::Plans => {
                    let notebook_data_source = ctx.add_model(|_| NotebookDataSource::new(true));
                    self.mixer.update(ctx, |mixer, _ctx| {
                        mixer.add_sync_source(notebook_data_source, [QueryFilter::Notebooks]);
                    });
                }
                AIContextMenuCategory::Rules => {
                    let rules_data_source = ctx.add_model(|_| RulesDataSource::new());
                    self.mixer.update(ctx, |mixer, _ctx| {
                        mixer.add_sync_source(rules_data_source, [QueryFilter::Rules]);
                    });
                }
                AIContextMenuCategory::DiffSet => {
                    let diffset_data_source = ctx.add_model(|_| DiffSetDataSource);
                    self.mixer.update(ctx, |mixer, _ctx| {
                        mixer.add_sync_source(diffset_data_source, [QueryFilter::DiffSets]);
                    });
                }
                AIContextMenuCategory::Conversations => {
                    let conversation_data_source = ctx.add_model(|_| ConversationDataSource);
                    self.mixer.update(ctx, |mixer, _ctx| {
                        mixer.add_sync_source(
                            conversation_data_source,
                            [QueryFilter::Conversations],
                        );
                    });
                }
                AIContextMenuCategory::Skills => {
                    let skills_data_source = ctx.add_model(|_| SkillsDataSource::new());
                    self.mixer.update(ctx, |mixer, _ctx| {
                        mixer.add_sync_source(skills_data_source, [QueryFilter::Skills]);
                    });
                }
                _ => {
                    // TODO: Add other categories
                }
            }
        }

        // Run the query with all data sources
        self.mixer.update(ctx, |mixer, ctx| {
            mixer.run_query(
                Query {
                    text: query.into(),
                    filters: HashSet::new(),
                },
                ctx,
            );
        });
    }

    #[cfg(target_family = "wasm")]
    fn setup_data_sources_for_all_categories(&mut self, query: &str, ctx: &mut ViewContext<Self>) {
        self.mixer.update(ctx, |mixer, ctx| {
            mixer.reset(ctx);
        });

        let categories = Self::get_categories_for_mode(
            self.state.is_ai_or_autodetect_mode,
            self.state.is_shared_session_viewer,
            self.state.is_in_ambient_agent,
            self.state.is_cli_agent_input,
            ctx,
        );
        for category in categories.iter() {
            if matches!(category, AIContextMenuCategory::Conversations) {
                let conversation_data_source = ctx.add_model(|_| ConversationDataSource);
                self.mixer.update(ctx, |mixer, _ctx| {
                    mixer.add_sync_source(conversation_data_source, [QueryFilter::Conversations]);
                });
            }
        }

        self.mixer.update(ctx, |mixer, ctx| {
            mixer.run_query(
                Query {
                    text: query.into(),
                    filters: HashSet::new(),
                },
                ctx,
            );
        });
    }

    /// Get the list of categories that match the current query filter
    fn get_filtered_categories(&self, app: &AppContext) -> Vec<AIContextMenuCategory> {
        let categories = Self::get_categories_for_mode(
            self.state.is_ai_or_autodetect_mode,
            self.state.is_shared_session_viewer,
            self.state.is_in_ambient_agent,
            self.state.is_cli_agent_input,
            app,
        );
        if self.state.main_menu_query.is_empty() {
            categories
        } else {
            let query_lower = self.state.main_menu_query.trim().to_lowercase();
            categories
                .into_iter()
                .filter(|category| {
                    let category_name_lower = category.name().to_lowercase();
                    category_name_lower.contains(&query_lower)
                })
                .collect()
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
        let filtered_categories = self.get_filtered_categories(app);

        // If no categories match the filter, show "No results found"
        // Ideally we don't enter this state because we transition to AllCategories mode
        // when no categories match.
        if filtered_categories.is_empty() {
            return self.render_no_results(app);
        }

        let last_display_index = filtered_categories.len().saturating_sub(1);
        for (display_index, category) in filtered_categories.iter().enumerate() {
            let is_selected = display_index == self.state.selected_category_index;
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
            let categories = Self::get_categories_for_mode(
                self.state.is_ai_or_autodetect_mode,
                self.state.is_shared_session_viewer,
                self.state.is_in_ambient_agent,
                self.state.is_cli_agent_input,
                app,
            );
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
        !Self::get_categories_for_mode(
            self.state.is_ai_or_autodetect_mode,
            self.state.is_shared_session_viewer,
            self.state.is_in_ambient_agent,
            self.state.is_cli_agent_input,
            app,
        )
        .is_empty()
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
        category: &AIContextMenuCategory,
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
        let categories = Self::get_categories_for_mode(
            self.state.is_ai_or_autodetect_mode,
            self.state.is_shared_session_viewer,
            self.state.is_in_ambient_agent,
            self.state.is_cli_agent_input,
            app,
        );
        if categories.len() > 1 {
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
        category: Option<&AIContextMenuCategory>,
        fallback: Box<dyn Element>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        #[cfg(not(target_family = "wasm"))]
        if let Some(cat) = category {
            if *cat == AIContextMenuCategory::Code && is_code_symbols_indexing(app) {
                return self.render_code_symbols_indexing(app);
            }
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

        let body = match &self.state.navigation_state {
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
        if !matches!(self.state.navigation_state, NavigationState::MainMenu) {
            if let (Some(selected_result_renderer), Some(details_panel_positioning)) = (
                self.selected_result_renderer(app),
                self.offset_positioning_for_details_panel(app),
            ) {
                if let Some(details) = selected_result_renderer.render_details(app) {
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
            }
        }

        // Use proper keybinding handling instead of event handlers
        Dismiss::new(stack.finish())
            .on_dismiss(|ctx, _app| ctx.dispatch_typed_action(AIContextMenuAction::Close))
            .prevent_interaction_with_other_elements()
            .finish()
    }
}
