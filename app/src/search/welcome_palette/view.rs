use std::collections::HashSet;
use std::ops::Deref as _;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use itertools::Itertools as _;
use pathfinder_geometry::vector::vec2f;
use warp_core::context_flag::ContextFlag;
use warp_core::features::FeatureFlag;
use warpui::elements::{
    Border, ChildView, Clipped, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox,
    Container, CornerRadius, Fill, Flex, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    ParentElement, Radius, SavePosition, Shrinkable,
};
use warpui::platform::Cursor;
use warpui::ui_components::button::{ButtonVariant, TextAndIcon, TextAndIconAlignment};
use warpui::ui_components::components::UiComponent as _;
use warpui::{
    units::{IntoPixels, Pixels},
    AppContext, Element, Entity, FocusContext, ModelHandle, SingletonEntity, TypedActionView,
    ViewContext, ViewHandle,
};

use super::super::palette_styles as styles;
use crate::appearance::Appearance;
use crate::cloud_object::model::persistence::CloudModel;
use crate::drive::CloudObjectTypeAndId;
use crate::palette::PaletteMode;
use crate::pane_group::pane::welcome_view::WelcomeViewAction;
use crate::search::action::search_item::MatchedBinding;
use crate::search::action::{CommandBindingDataSource, Event as CommandBindingDataSourceEvent};
use crate::search::binding_source::BindingSource;
use crate::search::command_palette::conversations::{self};
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::new_session::{AllowedSessionKinds, NewSessionDataSource};
use crate::search::command_palette::{launch_config, warp_drive, CommandPaletteMixer};
use crate::search::command_search::projects::project_data_source::ProjectDataSource;
use crate::search::command_search::projects::{ProjectSearchItem, SuggestedProjectsDataSource};
use crate::search::data_source::QueryResult;
use crate::search::mixer::{dedupe_score, DedupeStrategy};
use crate::search::result_renderer::QueryResultRenderer;
use crate::search::search_bar::{
    SearchBar, SearchBarEvent, SearchBarState, SearchResultOrdering, SelectionUpdate,
};
use crate::search::QueryFilter;
use crate::send_telemetry_from_ctx;
use crate::server::{ids::SyncId, telemetry::TelemetryEvent};
use crate::settings::AISettings;
use crate::terminal::History;
use crate::themes::theme::WarpTheme;
use crate::ui_components::icons::Icon;
use crate::workflows::{WorkflowSelectionSource, WorkflowSource, WorkflowType};
use crate::workspace::WorkspaceAction;

/// Position ID for the command palette list.
const PALETTE_LIST_SAVE_POSITION_ID: &str = "welcome_palette:list";

/// Max number of results to be returned by the search mixer. We set this to an arbitrarily
/// large size to minimize performances issues caused by rendering the elements of the palette
/// using a [`ClippedScrollable`].
// TODO(alokedesai): Remove once we add a properly viewported element.
const MAX_SEARCH_RESULTS: usize = 250;

const MAX_PROJECTS_IN_ZERO_STATE: usize = 4;
const MAX_ITEMS_IN_ZERO_STATE: usize = 5;

#[derive(Debug)]
pub enum Action {
    ResultClicked { action: CommandPaletteItemAction },
    ParentAction { action: WelcomeViewAction },
    Close,
}

pub enum Event {
    Close,
    ParentAction {
        action: WelcomeViewAction,
    },
    /// Execute the workflow identified by `id`.
    ExecuteWorkflow {
        id: SyncId,
    },
    /// Invoke the env vars identified by `id`.
    InvokeEnvironmentVariables {
        id: SyncId,
    },
    /// Open a notebook identified by `id`.
    OpenNotebook {
        id: SyncId,
    },
    /// View the relevant object in the Warp Drive sidebar.
    ViewInWarpDrive {
        id: CloudObjectTypeAndId,
    },
    /// Open a file at the given path.
    OpenFile {
        path: String,
    },
    /// Open a directory at the given path.
    OpenDirectory {
        path: String,
    },
    NewConversationInProject {
        path: String,
    },
}

#[derive(Default)]
struct StateHandles {
    clipped_scroll_state: ClippedScrollStateHandle,
    open_project_button: MouseStateHandle,
    terminal_session_button: MouseStateHandle,
}

#[derive(Copy, Clone, Debug)]
enum SelectedItem {
    ListItem(usize),
    OpenProjectButton,
    TerminalSessionButton,
}

impl Default for SelectedItem {
    fn default() -> Self {
        Self::ListItem(0)
    }
}

pub struct WelcomePalette {
    startup_directory: Option<PathBuf>,
    search_bar: ViewHandle<SearchBar<CommandPaletteItemAction>>,
    search_bar_state: ModelHandle<SearchBarState<CommandPaletteItemAction>>,
    state_handles: StateHandles,
    /// Placeholder element to render when no results are found.
    placeholder_query_renderer: QueryResultRenderer<CommandPaletteItemAction>,
    binding_source: ModelHandle<BindingSource>,
    zero_state_items: Vec<QueryResultRenderer<CommandPaletteItemAction>>,
    selected_item: SelectedItem,
    project_data_source: ModelHandle<ProjectDataSource>,
    conversations_data_source: ModelHandle<conversations::DataSource>,
    suggested_projects_data_source: ModelHandle<SuggestedProjectsDataSource>,
    open_project_keybinding: Option<String>,
    terminal_session_keybinding: Option<String>,
}

impl Entity for WelcomePalette {
    type Event = Event;
}

impl TypedActionView for WelcomePalette {
    type Action = Action;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            Action::ResultClicked { action } => {
                self.handle_result_accepted(action.clone(), ctx);
            }
            Action::ParentAction { action } => ctx.emit(Event::ParentAction { action: *action }),
            Action::Close => self.close(ctx, None),
        }
    }
}

impl warpui::View for WelcomePalette {
    fn ui_name() -> &'static str {
        "WelcomePalette"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut palette = Flex::column();
        palette.add_child(self.render_search_bar());

        if self.search_bar.as_ref(app).should_show_zero_state(app) {
            palette.add_child(
                Shrinkable::new(
                    1.,
                    self.render_item_list(&self.zero_state_items, self.selected_item, theme, app),
                )
                .finish(),
            );
            palette.add_child(self.render_footer_buttons(self.selected_item, appearance));
        } else {
            palette.add_child(Shrinkable::new(1., self.render_palette_list(theme, app)).finish());
        };

        Container::new(
            ConstrainedBox::new(palette.finish())
                .with_width(styles::PALETTE_WIDTH)
                .with_max_height(styles::PALETTE_HEIGHT)
                .finish(),
        )
        .with_background(theme.surface_2())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .with_border(Border::all(1.0).with_border_fill(theme.outline()))
        .with_padding_bottom(10.)
        .with_drop_shadow(*styles::DROP_SHADOW)
        .finish()
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.search_bar);
            self.binding_source.update(ctx, |_, ctx| ctx.notify());
        }
    }
}

impl WelcomePalette {
    pub fn new(
        startup_directory: Option<PathBuf>,
        binding_source: BindingSource,
        open_project_keybinding: Option<String>,
        terminal_session_keybinding: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let search_bar_state = ctx.add_model(|_ctx| {
            SearchBarState::new(SearchResultOrdering::TopDown).with_max_results(MAX_SEARCH_RESULTS)
        });

        ctx.subscribe_to_model(&search_bar_state, |me, _, event, ctx| {
            me.handle_search_bar_event(event, ctx);
        });
        ctx.observe(&search_bar_state, |_, _, ctx| ctx.notify());

        let binding_source = ctx.add_model(|_| binding_source);
        let actions_data_source =
            ctx.add_model(|ctx| CommandBindingDataSource::new(binding_source.clone(), ctx));
        ctx.subscribe_to_model(&actions_data_source, Self::handle_actions_data_source_event);

        let project_data_source = ctx.add_model(ProjectDataSource::new);
        let suggested_projects_data_source = ctx.add_model(SuggestedProjectsDataSource::new);
        let conversations_data_source = ctx.add_model(|_| conversations::DataSource::new());
        let launch_config_data_source = ctx.add_model(launch_config::DataSource::new);
        let new_session_data_source = ctx.add_model(|ctx| {
            NewSessionDataSource::new(binding_source.clone(), ctx)
                .with_allowed_kinds(AllowedSessionKinds::tabs_only())
        });
        let warp_drive_data_source = ctx.add_model(warp_drive::DataSource::new);

        let mixer = ctx.add_model(|ctx| {
            let mut mixer = CommandPaletteMixer::new();
            mixer.add_sync_source(actions_data_source.clone(), HashSet::new());
            mixer.add_sync_source(project_data_source.clone(), HashSet::new());
            mixer.add_sync_source(suggested_projects_data_source.clone(), HashSet::new());
            mixer.add_sync_source(warp_drive_data_source.clone(), HashSet::new());

            if AISettings::as_ref(ctx).is_any_ai_enabled(ctx) {
                mixer.add_sync_source(conversations_data_source.clone(), HashSet::new());
            }

            if ContextFlag::LaunchConfigurations.is_enabled() {
                mixer.add_sync_source(launch_config_data_source.clone(), HashSet::new());
            }
            if FeatureFlag::ShellSelector.is_enabled() && cfg!(feature = "local_tty") {
                mixer.add_sync_source(new_session_data_source.clone(), HashSet::new());
            }

            mixer.set_dedupe_strategy(DedupeStrategy::HighestScore);
            mixer
        });

        let ui_font_family = Appearance::as_ref(ctx).ui_font_family();

        let search_bar = ctx.add_typed_action_view(|ctx| {
            SearchBar::new(
                mixer.clone(),
                search_bar_state.clone(),
                "Code, build, or search for anything...",
                Self::create_query_result_renderer,
                ctx,
            )
            .with_font_family(ui_font_family, ctx)
        });

        ctx.subscribe_to_view(&search_bar, |me, _, event, ctx| {
            me.handle_search_bar_event(event, ctx);
        });

        ctx.subscribe_to_model(&History::handle(ctx), |me, _model, _event, ctx| {
            me.compute_zero_state_items(ctx);
            ctx.notify();
        });

        let placeholder_element = QueryResultRenderer::new(
            MatchedBinding::placeholder("No results found".into()).into(),
            "welcome_palette:no_results".into(),
            |_, _, _| {},
            *styles::QUERY_RESULT_RENDERER_STYLES,
        );

        Self {
            startup_directory,
            search_bar,
            search_bar_state,
            state_handles: Default::default(),
            placeholder_query_renderer: placeholder_element,
            binding_source,
            project_data_source,
            conversations_data_source,
            open_project_keybinding,
            terminal_session_keybinding,
            suggested_projects_data_source,
            zero_state_items: Default::default(),
            selected_item: Default::default(),
        }
    }

    fn handle_actions_data_source_event(
        &mut self,
        _: ModelHandle<CommandBindingDataSource>,
        event: &CommandBindingDataSourceEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        #[allow(irrefutable_let_patterns)]
        if let CommandBindingDataSourceEvent::IndexUpdated = event {
            self.compute_zero_state_items(ctx);
        }
    }

    fn compute_zero_state_items(&mut self, ctx: &mut ViewContext<Self>) {
        let mut projects = self.project_data_source.read(ctx, |projects, ctx| {
            projects
                .top_n(MAX_PROJECTS_IN_ZERO_STATE, ctx)
                .collect_vec()
        });

        // We want the first item to be the startup directory as if it were a project. If it
        // already is a project, push it to the front. If not, create an item for it.
        let startup_dir_idx = projects
            .iter()
            .find_position(|proj| Some(PathBuf::from(proj.path.clone())) == self.startup_directory);
        if let Some((i, _)) = startup_dir_idx {
            let item = projects.remove(i);
            projects.insert(0, item);
        } else if let Some(dir) = &self.startup_directory {
            let startup_dir_item = ProjectSearchItem::new(
                dir.to_string_lossy().into_owned(),
                fuzzy_match::FuzzyMatchResult::no_match(),
                Utc::now().naive_utc(),
            );
            projects.insert(0, startup_dir_item);
        }

        let suggestion_slots = MAX_PROJECTS_IN_ZERO_STATE.saturating_sub(projects.len());

        let suggested_projects = if suggestion_slots > 0 {
            self.suggested_projects_data_source
                .read(ctx, |suggested, _ctx: &AppContext| {
                    suggested.top_n(suggestion_slots)
                })
        } else {
            vec![]
        };

        let conversation_slots = MAX_ITEMS_IN_ZERO_STATE
            .saturating_sub(projects.len())
            .saturating_sub(suggested_projects.len());

        let conversations = if conversation_slots > 0 {
            self.conversations_data_source
                .read(ctx, |conversations, ctx| {
                    conversations.top_n(conversation_slots, ctx).collect()
                })
        } else {
            vec![]
        };

        let projects = dedupe_score(
            projects
                .into_iter()
                .map(QueryResult::from)
                .chain(suggested_projects)
                .collect(),
        );

        self.zero_state_items = projects
            .into_iter()
            .chain(conversations)
            .enumerate()
            .map(|(i, item)| Self::create_query_result_renderer(i, item))
            .collect();

        if self.zero_state_items.is_empty() {
            self.selected_item = SelectedItem::OpenProjectButton;
        }
    }

    /// Set the active query filter in the search bar to be `filter`.
    pub fn set_active_query_filter(&mut self, filter: QueryFilter, ctx: &mut ViewContext<Self>) {
        self.search_bar.update(ctx, |view, ctx| {
            view.set_query_filter(Some((filter, filter.filter_atom().primary_text)), ctx)
        });
        ctx.notify();
    }

    pub fn select_next_item(&mut self, ctx: &mut ViewContext<Self>) {
        self.search_bar.update(ctx, |search_bar, ctx| {
            search_bar.handle_selection_update(SelectionUpdate::Down, ctx);
        });
        ctx.notify();
    }

    pub fn select_prev_item(&mut self, ctx: &mut ViewContext<Self>) {
        self.search_bar.update(ctx, |search_bar, ctx| {
            search_bar.handle_selection_update(SelectionUpdate::Up, ctx);
        });
        ctx.notify();
    }

    pub fn active_query_filter(&self, app: &AppContext) -> Option<QueryFilter> {
        self.search_bar_state.as_ref(app).active_query_filter()
    }

    pub fn is_mode_enabled(&self, mode: PaletteMode, app: &AppContext) -> bool {
        let Some(active_query_filter) = self.active_query_filter(app) else {
            return false;
        };

        matches!(
            (mode, active_query_filter),
            (PaletteMode::Command, QueryFilter::Actions)
                | (PaletteMode::Navigation, QueryFilter::Sessions)
                | (PaletteMode::LaunchConfig, QueryFilter::LaunchConfigurations)
        )
    }

    fn create_query_result_renderer(
        index: usize,
        result: QueryResult<CommandPaletteItemAction>,
    ) -> QueryResultRenderer<CommandPaletteItemAction> {
        QueryResultRenderer::new(
            result,
            Self::query_result_save_position_id(index),
            |_, action, event_ctx| {
                event_ctx.dispatch_typed_action(Action::ResultClicked { action })
            },
            *styles::QUERY_RESULT_RENDERER_STYLES,
        )
    }

    /// Returns the position ID for a query result at `index`.
    fn query_result_save_position_id(index: usize) -> String {
        format!("welcome_palette:query_result:{index}")
    }

    fn render_search_bar(&self) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(Clipped::new(ChildView::new(&self.search_bar).finish()).finish())
                .finish(),
        )
        .with_vertical_padding(styles::SEARCH_BAR_PADDING_VERTICAL)
        .with_horizontal_padding(styles::RESULT_PADDING_HORIZONTAL)
        .finish()
    }

    /// Handles events emitted by the search bar.
    fn handle_search_bar_event(
        &mut self,
        event: &SearchBarEvent<CommandPaletteItemAction>,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SearchBarEvent::Close => {
                self.close(ctx, None);
            }
            SearchBarEvent::BufferCleared { .. } => {}
            SearchBarEvent::ResultAccepted { action, .. } => {
                self.handle_result_accepted(action.clone(), ctx);
            }
            SearchBarEvent::ResultSelected { index } => {
                self.scroll_selected_index_into_view(*index, ctx);
                ctx.notify();
            }
            SearchBarEvent::QueryFilterChanged { .. } => {}
            SearchBarEvent::SelectionUpdateInZeroState { selection_update } => {
                match selection_update {
                    SelectionUpdate::Up => {
                        self.selected_item = match self.selected_item {
                            SelectedItem::ListItem(i) => {
                                SelectedItem::ListItem(i.saturating_sub(1))
                            }
                            SelectedItem::OpenProjectButton => {
                                SelectedItem::ListItem(self.zero_state_items.len() - 1)
                            }
                            SelectedItem::TerminalSessionButton => SelectedItem::OpenProjectButton,
                        };
                        ctx.notify();
                    }
                    SelectionUpdate::Down => {
                        self.selected_item = match self.selected_item {
                            SelectedItem::ListItem(i) => {
                                if i == self.zero_state_items.len() - 1 {
                                    SelectedItem::OpenProjectButton
                                } else {
                                    SelectedItem::ListItem(i.saturating_add(1))
                                }
                            }
                            SelectedItem::OpenProjectButton
                            | SelectedItem::TerminalSessionButton => {
                                SelectedItem::TerminalSessionButton
                            }
                        };
                        ctx.notify();
                    }
                    SelectionUpdate::Clear | SelectionUpdate::Top => {
                        self.selected_item = SelectedItem::ListItem(0);
                        ctx.notify();
                    }
                    SelectionUpdate::Bottom => {
                        self.selected_item = SelectedItem::TerminalSessionButton;
                        ctx.notify();
                    }
                }
            }
            SearchBarEvent::EnterInZeroState { modified_enter } => match self.selected_item {
                SelectedItem::ListItem(i) => {
                    let Some(query_result) = self.zero_state_items.get(i) else {
                        return;
                    };
                    let action = if *modified_enter {
                        query_result.search_result.execute_result()
                    } else {
                        query_result.search_result.accept_result()
                    };
                    self.handle_result_accepted(action, ctx);
                }
                SelectedItem::OpenProjectButton => ctx.emit(Event::ParentAction {
                    action: WelcomeViewAction::OpenProject,
                }),
                SelectedItem::TerminalSessionButton => ctx.emit(Event::ParentAction {
                    action: WelcomeViewAction::CreateTerminalSession,
                }),
            },
        }
    }

    /// Scrolls the query result at `index` into view.
    fn scroll_selected_index_into_view(&self, index: usize, ctx: &mut ViewContext<Self>) {
        let list_bounds = ctx.element_position_by_id(PALETTE_LIST_SAVE_POSITION_ID);
        let item_bounds =
            ctx.element_position_by_id(Self::query_result_save_position_id(index).as_str());

        let Some((viewport_bounds, position_size)) = list_bounds.zip(item_bounds) else {
            return;
        };

        // If the selected index is contained within the viewport, there is no need to change the
        // scroll position.
        if viewport_bounds.contains_rect(position_size) {
            return;
        }

        let scroll_delta = if position_size.max_y() > viewport_bounds.max_y() {
            // The item is below the viewport. Update the scroll position by the number of pixels
            // the bottom of the item is below the viewport.
            position_size.max_y() - viewport_bounds.max_y()
        } else {
            // The item is above the viewport. Update the scroll position by the number of pixels
            // the top of the item is above the viewport.
            position_size.min_y() - viewport_bounds.min_y()
        };

        let scroll_top = self.state_handles.clipped_scroll_state.scroll_start();
        self.state_handles
            .clipped_scroll_state
            .scroll_to(scroll_top + scroll_delta.into_pixels());

        ctx.notify();
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>, accepted_action_type: Option<&'static str>) {
        let buffer_length = self.search_bar.as_ref(ctx).query(ctx).len();
        let filter = self.active_query_filter(ctx);
        let event = if let Some(result_type) = accepted_action_type {
            TelemetryEvent::PaletteSearchResultAccepted {
                result_type,
                filter,
                buffer_length,
            }
        } else {
            TelemetryEvent::PaletteSearchExited {
                filter,
                buffer_length,
            }
        };

        send_telemetry_from_ctx!(event, ctx);

        self.state_handles.clipped_scroll_state = Default::default();
        self.reset(ctx);

        // Some of the actions that are dispatched before closing can close the Window (e.g. "Close
        // Tab" on the final tab of the window). Confirm that the Window still exists before trying
        // to update the view.
        if ctx.root_view_id(ctx.window_id()).is_some() {
            ctx.emit(Event::Close);
        }
    }

    pub fn reset(&mut self, ctx: &mut ViewContext<Self>) {
        self.state_handles
            .clipped_scroll_state
            .scroll_to(Pixels::zero());
        self.search_bar.update(ctx, |search_bar, ctx| {
            search_bar.reset(
                None, /* initial_query */
                None, /* query_filter */
                SearchResultOrdering::TopDown,
                ctx,
            )
        });

        ctx.notify();
    }

    /// Inserts `query` into the search bar.
    pub fn insert_query_text(&mut self, query: &str, ctx: &mut ViewContext<Self>) {
        self.search_bar.update(ctx, |search_bar, ctx| {
            search_bar.insert_query_text(query, ctx);
        })
    }

    fn render_palette_list(&self, theme: &WarpTheme, app: &AppContext) -> Box<dyn Element> {
        match self.search_bar_state.as_ref(app).query_result_renderers() {
            None => {
                self.placeholder_query_renderer
                    .render(0, true /* is_selected */, app)
            }
            Some(renderers) if renderers.is_empty() => {
                self.placeholder_query_renderer
                    .render(0, true /* is_selected */, app)
            }
            Some(renderers) => {
                let selected_index = self.search_bar_state.as_ref(app).selected_index();
                self.render_item_list(
                    renderers,
                    SelectedItem::ListItem(selected_index.unwrap_or_default()),
                    theme,
                    app,
                )
            }
        }
    }

    fn render_item_list(
        &self,
        renderers: &[QueryResultRenderer<CommandPaletteItemAction>],
        selected_item: SelectedItem,
        theme: &WarpTheme,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let selected_index = match selected_item {
            SelectedItem::ListItem(i) => Some(i),
            _ => None,
        };
        let list = Flex::column()
            .with_children(renderers.iter().enumerate().map(|(index, renderer)| {
                SavePosition::new(
                    renderer.render(index, Some(index) == selected_index, app),
                    renderer.position_id.as_str(),
                )
                .finish()
            }))
            .finish();

        SavePosition::new(
            ClippedScrollable::vertical(
                self.state_handles.clipped_scroll_state.clone(),
                list,
                styles::SCROLLBAR_WIDTH,
                theme.nonactive_ui_detail().into(),
                theme.active_ui_detail().into(),
                // Leave the scrollbar gutter background transparent.
                Fill::None,
            )
            .with_overlayed_scrollbar()
            .finish(),
            PALETTE_LIST_SAVE_POSITION_ID,
        )
        .finish()
    }

    fn render_footer_buttons(
        &self,
        selected_item: SelectedItem,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        let mut open_project_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Basic,
                self.state_handles.open_project_button.clone(),
            )
            .with_text_and_icon_label(TextAndIcon::new(
                TextAndIconAlignment::IconFirst,
                match &self.open_project_keybinding {
                    Some(keystroke) => format!("Add repository {keystroke}"),
                    None => "Add repository".to_string(),
                },
                Icon::Plus.to_warpui_icon(theme.foreground()),
                MainAxisSize::Max,
                MainAxisAlignment::Center,
                vec2f(16., 16.),
            ))
            .with_cursor(Some(Cursor::PointingHand));
        if let SelectedItem::OpenProjectButton = selected_item {
            let style = *open_project_button.hovered_styles();
            open_project_button = open_project_button.with_style(style);
        }

        let mut terminal_session_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Basic,
                self.state_handles.terminal_session_button.clone(),
            )
            .with_text_and_icon_label(TextAndIcon::new(
                TextAndIconAlignment::IconFirst,
                match &self.terminal_session_keybinding {
                    Some(keystroke) => format!("Terminal session {keystroke}"),
                    None => "Terminal session".to_string(),
                },
                Icon::Terminal.to_warpui_icon(theme.foreground()),
                MainAxisSize::Max,
                MainAxisAlignment::Center,
                vec2f(16., 16.),
            ))
            .with_cursor(Some(Cursor::PointingHand));
        if let SelectedItem::TerminalSessionButton = selected_item {
            let style = *terminal_session_button.hovered_styles();
            terminal_session_button = terminal_session_button.with_style(style);
        }

        let row = Flex::row().with_children([
            Shrinkable::new(
                1.,
                Container::new(
                    open_project_button
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(Action::ParentAction {
                                action: WelcomeViewAction::OpenProject,
                            });
                        })
                        .finish(),
                )
                .with_margin_right(4.)
                .finish(),
            )
            .finish(),
            Shrinkable::new(
                1.,
                Container::new(
                    terminal_session_button
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(Action::ParentAction {
                                action: WelcomeViewAction::CreateTerminalSession,
                            });
                        })
                        .finish(),
                )
                .with_padding_left(4.)
                .finish(),
            )
            .finish(),
        ]);

        Container::new(row.finish())
            .with_padding_left(8.)
            .with_padding_right(8.)
            .with_margin_top(16.)
            .finish()
    }

    /// Handles the `WelcomePaletteItemAction` action and closes the search panel.
    fn handle_result_accepted(
        &mut self,
        result_action: CommandPaletteItemAction,
        ctx: &mut ViewContext<Self>,
    ) {
        match &result_action {
            CommandPaletteItemAction::AcceptBinding { binding } => {
                if let Some(action) = binding.action.as_deref() {
                    self.dispatch_typed_action_on_view(action, ctx);
                };
            }
            CommandPaletteItemAction::NewConversationInProject {
                path,
                project_name: _,
            } => {
                ctx.emit(Event::NewConversationInProject { path: path.clone() });
            }
            CommandPaletteItemAction::NavigateToConversation { .. } => {
                // This code is dead, so no need to support this case
            }
            CommandPaletteItemAction::OpenNotebook { id } => {
                self.dispatch_typed_action_on_view(&WorkspaceAction::OpenNotebook { id: *id }, ctx);
                self.close(ctx, Some(result_action.result_type()));
            }
            CommandPaletteItemAction::ExecuteWorkflow { id } => {
                let Some(workflow) = CloudModel::as_ref(ctx).get_workflow(id) else {
                    log::warn!("Tried to execute workflow for id {id:?} but it does not exist");
                    return;
                };

                self.dispatch_typed_action_on_view(
                    &WorkspaceAction::RunWorkflow {
                        workflow: Arc::new(WorkflowType::Cloud(Box::new(workflow.clone()))),
                        workflow_source: WorkflowSource::Global,
                        workflow_selection_source: WorkflowSelectionSource::CommandPalette,
                        argument_override: None,
                    },
                    ctx,
                );
                self.close(ctx, Some(result_action.result_type()));
            }
            CommandPaletteItemAction::NewSession { source } => {
                self.dispatch_typed_action_on_view(source.action().deref(), ctx);
                self.close(ctx, Some(result_action.result_type()));
            }
            _ => {
                // TODO
            }
        }
    }

    /// Dispatches `action` to the correct window and [`warpui::View`] by using the current state of
    /// the [`BindingSource`] model.
    fn dispatch_typed_action_on_view(
        &self,
        action: &dyn warpui::Action,
        ctx: &mut ViewContext<Self>,
    ) {
        send_telemetry_from_ctx!(
            TelemetryEvent::SelectCommandPaletteOption(format!("{action:?}")),
            ctx
        );

        let (window_id, view_id) = match self.binding_source.as_ref(ctx) {
            BindingSource::View {
                window_id, view_id, ..
            } => (*window_id, *view_id),
            BindingSource::None => return,
        };

        ctx.dispatch_typed_action_for_view(window_id, view_id, action);
    }
}
