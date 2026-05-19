use itertools::Itertools;

use async_channel::Sender;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::Vector2F;

use crate::search::mixer::AddAsyncSourceOptions;
use lazy_static::lazy_static;
use std::{collections::HashSet, ops::Range, sync::Arc, time::Duration};
use warp_core::features::FeatureFlag;
use warpui::{
    accessibility::{AccessibilityContent, WarpA11yRole},
    elements::{
        resizable_state_handle, Align, AnchorPair, Border, ConstrainedBox, Container, CornerRadius,
        CrossAxisAlignment, Dismiss, Fill, Flex, MouseStateHandle, OffsetPositioning, OffsetType,
        ParentElement, ParentOffsetBounds, PositionedElementOffsetBounds, PositioningAxis, Radius,
        Resizable, ResizableStateHandle, SavePosition, ScrollStateHandle, Scrollable,
        ScrollableElement, Shrinkable, Stack, UniformList, UniformListState, XAxisAnchor,
        YAxisAnchor,
    },
    presenter::ChildView,
    ui_components::components::{UiComponent, UiComponentStyles},
    AppContext, Element, Entity, FocusContext, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle, WeakViewHandle,
};

use crate::{
    ai_assistant::{
        execution_context::WarpAiExecutionContext, GenerateCommandsFromNaturalLanguageError,
    },
    appearance::Appearance,
    auth::{
        auth_manager::AuthManager, auth_state::AuthState, auth_view_modal::AuthViewVariant,
        AuthStateProvider, UserUid,
    },
    completer::SessionContext,
    drive::settings::WarpDriveSettings,
    search::{
        command_search::searcher::{CommandSearchItemAction, CommandSearchMixer},
        result_renderer::{QueryResultRenderer, QueryResultRendererStyles},
        search_bar::{SearchBar, SearchBarEvent, SearchBarState, SearchResultOrdering},
        QueryFilter,
    },
    send_telemetry_from_ctx,
    server::{ids::ServerId, server_api::ai::AIClient, telemetry::TelemetryEvent},
    settings::AISettings,
    terminal::{
        input::MenuPositioning,
        model::session::SessionId,
        resizable_data::{ModalType, ResizableData, DEFAULT_UNIVERSAL_SEARCH_WIDTH},
        History, HistoryEvent,
    },
    workspaces::user_workspaces::UserWorkspaces,
};

use super::{
    ai_queries::AIQueriesDataSource,
    env_var_collections::EnvVarCollectionDataSource,
    history::history_data_source_for_session,
    notebooks::notebooks_data_source,
    warp_ai::WarpAIDataSource,
    workflows::{cloud_workflows_data_source, WorkflowsDataSource},
    zero_state::{CommandSearchZeroStateEvent, CommandSearchZeroStateView},
};

const DEFAULT_PLACEHOLDER_TEXT: &str = "Search your history, workflows, and more";
const PANEL_POSITION_ID: &str = "CommandSearchViewPanel";
const DETAILS_PANEL_MARGIN: f32 = 4.;
const MIN_WIDTH_RATIO: f32 = 0.25;
const MAX_WIDTH_RATIO: f32 = 1.0;

lazy_static! {
    static ref QUERY_RESULT_RENDERER_STYLES: QueryResultRendererStyles =
        QueryResultRendererStyles {
            result_item_height_fn: |appearance| {
                styles::line_height_sensitive_vertical_padding(appearance)
                    + appearance.monospace_font_size()
            },
            panel_border_fn: styles::panel_border,
            ..Default::default()
        };
}

/// The events that `CommandSearchView` emits to its parent view.
pub enum CommandSearchEvent {
    ItemSelected {
        query: String,
        payload: Box<CommandSearchItemAction>,
    },
    Close {
        /// The query when Command Search was closed.
        query: String,

        /// The filter when Command Search was closed, if any.
        filter: Option<QueryFilter>,
    },
    Blur,
    Resize,
}

/// The actions that internal elements (e.g.: `Dismiss`) produce for consumption
/// by `CommandSearchView` itself.
#[derive(Clone, Debug)]
pub enum CommandSearchAction {
    ResultClicked {
        result_index: usize,
        result_action: Box<CommandSearchItemAction>,
    },
    Close,
    Resize,
    OpenUpgradeLink(String),
    AttemptLoginGatedUpgrade,
}

struct CommandSearchViewState {
    list_state: UniformListState,
    scroll_state: ScrollStateHandle,

    /// Range of indices corresponding to the indices of the query results visible in the results
    /// list.
    visible_results_range: Option<Range<usize>>,
}

/// A panel that allows the user to search for a command to execute next.
pub struct CommandSearchView {
    zero_state_handle: ViewHandle<CommandSearchZeroStateView>,
    handle: WeakViewHandle<Self>,
    menu_positioning: MenuPositioning,
    auth_state: Arc<AuthState>,
    ai_client: Arc<dyn AIClient>,
    state: CommandSearchViewState,
    visible_results_range_sender: Sender<Range<usize>>,
    resizable_state_handle: ResizableStateHandle,
    search_bar: ViewHandle<SearchBar<CommandSearchItemAction>>,
    search_bar_state: ModelHandle<SearchBarState<CommandSearchItemAction>>,
    mixer: ModelHandle<CommandSearchMixer>,
    upgrade_link: MouseStateHandle,
}

impl CommandSearchView {
    pub fn new(ai_client: Arc<dyn AIClient>, ctx: &mut ViewContext<Self>) -> Self {
        let search_bar_state =
            ctx.add_model(|_| SearchBarState::new(SearchResultOrdering::BottomUp));

        ctx.observe(&search_bar_state, |_, _, ctx| {
            ctx.notify();
        });

        let mixer = ctx.add_model(|_| CommandSearchMixer::new());

        let search_bar = ctx.add_typed_action_view(|ctx| {
            SearchBar::new(
                mixer.clone(),
                search_bar_state.clone(),
                DEFAULT_PLACEHOLDER_TEXT,
                |result_index, result| {
                    QueryResultRenderer::new(
                        result,
                        format!("QueryResultRenderer:{result_index}"),
                        |result_index, result_action, event_ctx| {
                            event_ctx.dispatch_typed_action(CommandSearchAction::ResultClicked {
                                result_index,
                                result_action: Box::new(result_action),
                            })
                        },
                        *QUERY_RESULT_RENDERER_STYLES,
                    )
                },
                ctx,
            )
        });

        ctx.subscribe_to_view(&search_bar, |me, _handle, event, ctx| {
            me.handle_search_bar_event(event, ctx);
        });

        ctx.subscribe_to_model(&search_bar_state, |me, _handle, event, ctx| {
            me.handle_search_bar_event(event, ctx);
        });

        let zero_state_handle = ctx.add_typed_action_view(CommandSearchZeroStateView::new);
        ctx.subscribe_to_view(&zero_state_handle, |me, _handle, event, ctx| {
            me.handle_zero_state_event(event, ctx);
        });

        let (visible_results_range_sender, visible_results_range_receiver) =
            async_channel::unbounded();
        let _ = ctx.spawn_stream_local(
            visible_results_range_receiver,
            Self::update_visible_results_range,
            |_, _| {},
        );

        let resizable_data_handle = ResizableData::handle(ctx);
        let resizable_state_handle = resizable_data_handle
            .as_ref(ctx)
            .get_handle(ctx.window_id(), ModalType::UniversalSearchWidth)
            .unwrap_or_else(|| {
                log::error!("Couldn't retrieve universal search resizable state handle.");
                resizable_state_handle(DEFAULT_UNIVERSAL_SEARCH_WIDTH)
            });

        Self {
            auth_state: AuthStateProvider::as_ref(ctx).get().clone(),
            ai_client,
            zero_state_handle,
            menu_positioning: Default::default(),
            handle: ctx.handle(),
            visible_results_range_sender,
            state: CommandSearchViewState {
                scroll_state: Default::default(),
                list_state: Default::default(),
                visible_results_range: None,
            },
            resizable_state_handle,
            search_bar,
            search_bar_state,
            mixer,
            upgrade_link: Default::default(),
        }
    }

    /// Resets the mixer with the relevant data sources for Command Search registered.
    fn reset_command_search_mixer(
        &mut self,
        session_id: SessionId,
        session_context: Option<SessionContext>,
        ai_execution_context: Option<WarpAiExecutionContext>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.mixer.update(ctx, |mixer, ctx| {
            mixer.reset(ctx);

            // Add data sources in lowest->highest priority order.  If results from two
            // data sources produce the same ranking score, the data source added first
            // will show up higher in the list (i.e.: further away from the input).
            if AISettings::as_ref(ctx).is_any_ai_enabled(ctx) {
                mixer.add_sync_source(
                    WarpAIDataSource::new(self.ai_client.clone(), None),
                    HashSet::from([QueryFilter::NaturalLanguage]),
                );
                mixer.add_async_source(
                    WarpAIDataSource::new(self.ai_client.clone(), ai_execution_context),
                    HashSet::from([QueryFilter::NaturalLanguage]),
                    AddAsyncSourceOptions {
                        debounce_interval: Some(Duration::from_millis(50)),
                        run_in_zero_state: false,
                        run_when_unfiltered: false,
                    },
                    ctx,
                );
            }

            if WarpDriveSettings::is_warp_drive_enabled(ctx) {
                mixer.add_sync_source(
                    WorkflowsDataSource::new(session_context.as_ref(), ctx),
                    HashSet::from([QueryFilter::Workflows]),
                );

                let mut workflows_filters = HashSet::from([QueryFilter::Workflows]);
                if AISettings::as_ref(ctx).is_any_ai_enabled(ctx) {
                    workflows_filters.insert(QueryFilter::AgentModeWorkflows);
                }

                mixer.add_async_source(
                    cloud_workflows_data_source(),
                    workflows_filters,
                    AddAsyncSourceOptions {
                        debounce_interval: Some(Duration::from_millis(50)),
                        run_in_zero_state: true,
                        run_when_unfiltered: true,
                    },
                    ctx,
                );

                mixer.add_async_source(
                    notebooks_data_source(),
                    HashSet::from([QueryFilter::Notebooks]),
                    AddAsyncSourceOptions {
                        debounce_interval: Some(Duration::from_millis(50)),
                        run_in_zero_state: true,
                        run_when_unfiltered: true,
                    },
                    ctx,
                );

                // EnvVarCollectionDataSource stays synchronous because each match target is
                // structurally short (title, variable name, description). The per-item fuzzy
                // match cost is negligible, so offloading to an async task would add complexity
                // without meaningful performance benefit.
                mixer.add_sync_source(
                    EnvVarCollectionDataSource::new(),
                    HashSet::from([QueryFilter::EnvironmentVariables]),
                );
            }

            if FeatureFlag::AgentMode.is_enabled() && AISettings::as_ref(ctx).is_any_ai_enabled(ctx)
            {
                mixer.add_sync_source(
                    AIQueriesDataSource::new(),
                    HashSet::from([QueryFilter::PromptHistory]),
                );
            }

            if History::as_ref(ctx).is_queryable(&session_id) {
                let source = History::handle(ctx).read(ctx, |history_model, app| {
                    history_data_source_for_session(session_id, history_model, app)
                });
                mixer.add_async_source(
                    source,
                    HashSet::from([QueryFilter::History]),
                    AddAsyncSourceOptions {
                        debounce_interval: Some(Duration::from_millis(50)),
                        run_in_zero_state: true,
                        run_when_unfiltered: true,
                    },
                    ctx,
                );
            } else {
                ctx.subscribe_to_model(&History::handle(ctx), move |mixer, history_event, ctx| {
                    match history_event {
                        HistoryEvent::Initialized(id) => {
                            if id == &session_id {
                                let source = history_data_source_for_session(
                                    session_id,
                                    History::as_ref(ctx),
                                    ctx,
                                );
                                mixer.add_async_source(
                                    source,
                                    HashSet::from([QueryFilter::History]),
                                    AddAsyncSourceOptions {
                                        debounce_interval: Some(Duration::from_millis(50)),
                                        run_in_zero_state: true,
                                        run_when_unfiltered: true,
                                    },
                                    ctx,
                                );
                                if let Some(query) = mixer.current_query().cloned() {
                                    mixer.run_query(query, ctx);
                                }
                                ctx.notify();
                            }
                        }
                    }
                });
            }
        })
    }

    /// Resets view state when the search panel is shown.
    #[allow(clippy::too_many_arguments)]
    pub fn reset_state(
        &mut self,
        session_id: SessionId,
        session_context: Option<SessionContext>,
        initial_query: String,
        query_filter: Option<QueryFilter>,
        menu_positioning: MenuPositioning,
        ai_execution_context: Option<WarpAiExecutionContext>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.reset_command_search_mixer(session_id, session_context, ai_execution_context, ctx);
        let ordering = match menu_positioning {
            MenuPositioning::AboveInputBox => SearchResultOrdering::BottomUp,
            MenuPositioning::BelowInputBox => SearchResultOrdering::TopDown,
        };

        self.menu_positioning = menu_positioning;

        self.search_bar.update(ctx, |search_bar, ctx| {
            search_bar.reset(Some(initial_query), query_filter, ordering, ctx);
            ctx.notify();
        });
    }

    /// Updates the range of indices of visible query results in the list.
    fn update_visible_results_range(
        &mut self,
        visible_results_range: Range<usize>,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(current_visible_results_range) = &self.state.visible_results_range {
            if current_visible_results_range == &visible_results_range {
                return;
            }
        }
        self.state.visible_results_range = Some(visible_results_range);
        ctx.notify();
    }

    /// Handles events emitted by the zero state UI.
    fn handle_zero_state_event(
        &mut self,
        event: &CommandSearchZeroStateEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            CommandSearchZeroStateEvent::FilterChipSelected(filter) => self
                .set_active_query_filter(Some((*filter, filter.filter_atom().primary_text)), ctx),
            CommandSearchZeroStateEvent::SampleQuerySelected(filter) => self
                .set_active_query_filter(Some((*filter, filter.filter_atom().primary_text)), ctx),
        }
    }

    fn close(&self, ctx: &mut ViewContext<Self>) {
        let query = self.search_bar.as_ref(ctx).query(ctx);
        let filter = self.search_bar_state.as_ref(ctx).active_query_filter();
        ctx.emit(CommandSearchEvent::Close { query, filter });
    }

    fn blur(&self, ctx: &mut ViewContext<Self>) {
        let buffer_length = self.search_bar.as_ref(ctx).query(ctx).len();
        send_telemetry_from_ctx!(
            TelemetryEvent::CommandSearchExited {
                query_filter: self.active_query_filter(ctx),
                buffer_length
            },
            ctx
        );
        ctx.emit(CommandSearchEvent::Blur);
    }

    /// Handles events emitted by the search bar.
    fn handle_search_bar_event(
        &mut self,
        event: &SearchBarEvent<CommandSearchItemAction>,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SearchBarEvent::Close => {
                let buffer_length = self.search_bar.as_ref(ctx).query(ctx).len();
                send_telemetry_from_ctx!(
                    TelemetryEvent::CommandSearchExited {
                        query_filter: self.active_query_filter(ctx),
                        buffer_length
                    },
                    ctx
                );
                self.close(ctx);
            }
            // ctrl-c should close the command search view
            SearchBarEvent::BufferCleared { buffer_len } => {
                send_telemetry_from_ctx!(
                    TelemetryEvent::CommandSearchExited {
                        query_filter: self.active_query_filter(ctx),
                        buffer_length: *buffer_len
                    },
                    ctx
                );
                self.close(ctx);
            }
            SearchBarEvent::ResultAccepted { index, action } => {
                self.handle_result_selected(*index, action.clone(), ctx);
            }
            SearchBarEvent::ResultSelected { index } => {
                self.state.list_state.scroll_to(*index);
                ctx.notify();
            }
            SearchBarEvent::QueryFilterChanged { new_filter } => {
                send_telemetry_from_ctx!(
                    TelemetryEvent::CommandSearchFilterChanged {
                        new_filter: *new_filter
                    },
                    ctx
                );
            }
            SearchBarEvent::SelectionUpdateInZeroState { .. } => {}
            SearchBarEvent::EnterInZeroState { .. } => {}
        }
    }

    /// Updates the active filter and re-runs the query, since results may be affected by the newly
    /// active filter.
    ///
    /// This method also updates UI state including the placeholder text and the show/hide
    /// zero_state flag.
    fn set_active_query_filter(
        &mut self,
        filter_and_atom_text: Option<(QueryFilter, &'static str)>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.search_bar.update(ctx, |search_bar, ctx| {
            search_bar.set_query_filter(filter_and_atom_text, ctx);
        });
    }

    /// Returns the active query filters
    fn active_query_filter(&self, app: &AppContext) -> Option<QueryFilter> {
        self.search_bar_state.as_ref(app).active_query_filter()
    }

    /// Emits the `ItemSelected` event containing the passed `CommandSearchEventPayload` and closes
    /// the search panel.
    fn handle_result_selected(
        &self,
        result_index: usize,
        result_action: CommandSearchItemAction,
        ctx: &mut ViewContext<Self>,
    ) {
        {
            use CommandSearchItemAction::*;
            let was_immediately_executed = match &result_action {
                ExecuteHistory(_) | RunAIQuery(_) => true,

                AcceptHistory(_)
                | AcceptWorkflow(_)
                | AcceptNotebook(_)
                | OpenWarpAI
                | AcceptEnvVarCollection(_)
                | TranslateUsingWarpAI
                | AcceptAIQuery(_) => false,
            };

            let (a11y_content, a11y_help_content) = if was_immediately_executed {
                (
                    "Result executed".to_owned(),
                    "Press Cmd-Up to navigate to the command's output.".to_owned(),
                )
            } else {
                (
                    "Result accepted.".to_owned(),
                    "You can edit the command here before pressing Enter to execute it.".to_owned(),
                )
            };
            ctx.emit_a11y_content(AccessibilityContent::new(
                a11y_content,
                a11y_help_content,
                WarpA11yRole::UserAction,
            ));

            // Recompute the result index - the incoming index is the index in the
            // uniform list, but what we want is the "distance from first result".
            let result_index = match self.search_bar_state.as_ref(ctx).query_result_renderers() {
                Some(renderers) => renderers.len() - result_index - 1,
                None => result_index,
            };

            send_telemetry_from_ctx!(
                TelemetryEvent::CommandSearchResultAccepted {
                    result_index,
                    result_type: (&result_action).into(),
                    query_filter: self.search_bar_state.as_ref(ctx).active_query_filter(),
                    buffer_length: self.search_bar.as_ref(ctx).query(ctx).len(),
                    was_immediately_executed,
                },
                ctx
            );
        }

        let query = self.search_bar.as_ref(ctx).query(ctx);

        ctx.emit(CommandSearchEvent::ItemSelected {
            query,
            payload: Box::new(result_action),
        });
        self.close(ctx);
    }

    /// Returns an `Option` containing the [`QueryResultRenderer`] that owns the currently selected
    /// result, if any.
    fn selected_result_renderer<'a>(
        &self,
        app: &'a AppContext,
    ) -> Option<&'a QueryResultRenderer<CommandSearchItemAction>> {
        let mixer = self.mixer.as_ref(app);
        if mixer.is_loading() && mixer.are_results_empty() {
            return None;
        }
        self.search_bar_state.as_ref(app).selected_result_renderer()
    }

    fn render_loading_state(&self, appearance: &Appearance) -> Box<dyn Element> {
        let muted_color: ColorU = appearance.theme().nonactive_ui_text_color().into();
        let text = appearance
            .ui_builder()
            .span("Loading...")
            .with_style(UiComponentStyles {
                font_size: Some(appearance.monospace_font_size()),
                font_family_id: Some(appearance.ui_font_family()),
                font_color: Some(muted_color),
                ..Default::default()
            })
            .build()
            .finish();
        let row = Flex::row()
            .with_main_axis_size(warpui::elements::MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Shrinkable::new(1., text).finish());

        Container::new(row.finish())
            .with_uniform_padding(8.)
            .with_padding_bottom(10.)
            .finish()
    }

    fn render_error_header(
        &self,
        app: &AppContext,
        message: String,
        is_ratelimit_error: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        if is_ratelimit_error {
            let current_user_id = self.auth_state.user_id().unwrap_or_default();
            if let Some(team) = UserWorkspaces::as_ref(app).current_team() {
                let current_user_email = self.auth_state.user_email().unwrap_or_default();
                let has_admin_permissions = team.has_admin_permissions(&current_user_email);
                if team.billing_metadata.can_upgrade_to_higher_tier_plan() {
                    if has_admin_permissions {
                        self.render_error_header_with_upgrade_link(
                            app,
                            appearance,
                            Some(team.uid),
                            current_user_id,
                        )
                    } else {
                        self.render_error_header_text("Looks like you're out of credits. Contact a team admin to upgrade for more credits.".to_string(), appearance)
                    }
                } else {
                    self.render_error_header_text(message, appearance)
                }
            } else {
                self.render_error_header_with_upgrade_link(app, appearance, None, current_user_id)
            }
        } else {
            self.render_error_header_text(message, appearance)
        }
    }

    fn render_error_header_text(
        &self,
        message: String,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let text = appearance
            .ui_builder()
            .span(message)
            .with_style(UiComponentStyles {
                font_size: Some(appearance.monospace_font_size()),
                font_family_id: Some(appearance.ui_font_family()),
                font_color: Some(appearance.theme().nonactive_ui_text_color().into()),
                ..Default::default()
            })
            .build()
            .finish();

        Container::new(
            Flex::row()
                .with_main_axis_size(warpui::elements::MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Shrinkable::new(1., text).finish())
                .finish(),
        )
        .with_horizontal_padding(16.)
        .with_padding_bottom(10.)
        .with_padding_top(4.)
        .finish()
    }

    fn render_error_header_with_upgrade_link(
        &self,
        app: &AppContext,
        appearance: &Appearance,
        team_uid: Option<ServerId>,
        user_id: UserUid,
    ) -> Box<dyn Element> {
        let mut row = Flex::row()
            .with_main_axis_size(warpui::elements::MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        let upgrade_link = team_uid
            .map(UserWorkspaces::upgrade_link_for_team)
            .unwrap_or_else(|| UserWorkspaces::upgrade_link(user_id));

        let link = if AuthStateProvider::as_ref(app)
            .get()
            .is_anonymous_or_logged_out()
        {
            appearance
                .ui_builder()
                .link(
                    "Upgrade".into(),
                    None,
                    Some(Box::new(move |ctx| {
                        ctx.dispatch_typed_action(CommandSearchAction::AttemptLoginGatedUpgrade);
                    })),
                    self.upgrade_link.clone(),
                )
                .soft_wrap(false)
        } else {
            appearance
                .ui_builder()
                .link(
                    "Upgrade".into(),
                    None,
                    Some(Box::new(move |ctx| {
                        ctx.dispatch_typed_action(CommandSearchAction::OpenUpgradeLink(
                            upgrade_link.clone(),
                        ));
                    })),
                    self.upgrade_link.clone(),
                )
                .soft_wrap(false)
        };

        row.add_child(
            appearance
                .ui_builder()
                .span("Looks like you're out of credits. ")
                .with_style(UiComponentStyles {
                    font_size: Some(appearance.monospace_font_size()),
                    font_family_id: Some(appearance.ui_font_family()),
                    font_color: Some(appearance.theme().nonactive_ui_text_color().into()),
                    ..Default::default()
                })
                .build()
                .finish(),
        );
        row.add_child(
            link.with_style(UiComponentStyles {
                font_size: Some(appearance.monospace_font_size()),
                font_family_id: Some(appearance.ui_font_family()),
                ..Default::default()
            })
            .build()
            .finish(),
        );
        row.add_child(
            appearance
                .ui_builder()
                .span(" for more credits.")
                .with_style(UiComponentStyles {
                    font_size: Some(appearance.monospace_font_size()),
                    font_family_id: Some(appearance.ui_font_family()),
                    font_color: Some(appearance.theme().nonactive_ui_text_color().into()),
                    ..Default::default()
                })
                .build()
                .finish(),
        );

        Container::new(row.finish())
            .with_horizontal_padding(16.)
            .with_padding_bottom(10.)
            .with_padding_top(4.)
            .finish()
    }

    /// Renders the results pane.
    fn render_results(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let query_result_renderers = self.search_bar_state.as_ref(app).query_result_renderers();
        let selected_index = self.search_bar_state.as_ref(app).selected_index();
        match (&query_result_renderers, selected_index) {
            (Some(query_result_renderers), _) if query_result_renderers.is_empty() => {
                // There are no results to display, so notify the user of that fact.
                let text = appearance
                    .ui_builder()
                    .span("No results found.")
                    .with_style(UiComponentStyles {
                        font_size: Some(appearance.monospace_font_size()),
                        font_family_id: Some(appearance.ui_font_family()),
                        font_color: Some(appearance.theme().nonactive_ui_text_color().into()),
                        ..Default::default()
                    })
                    .build()
                    .finish();

                let vertical_padding = styles::line_height_sensitive_vertical_padding(appearance);
                Container::new(
                    ConstrainedBox::new(Align::new(text).finish())
                        // Make the height the same as a single item, but adjust by the panel padding so
                        // the text is centered within the panel.
                        .with_height(
                            appearance.monospace_font_size() + vertical_padding
                                - styles::TOP_PADDING,
                        )
                        .finish(),
                )
                .with_margin_bottom(styles::TOP_PADDING)
                .finish()
            }
            (Some(query_result_renderers), Some(selected_index)) => {
                let view_handle = self.handle.clone();
                let build_items = move |range: Range<usize>, app: &AppContext| {
                    let command_search_view = view_handle
                        .upgrade(app)
                        .expect("View handle should upgradeable.")
                        .as_ref(app);
                    let query_result_renderers = command_search_view
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
                                    // Convert the index from "visible item index" to index within
                                    // the full list.
                                    let result_index = result_index + range.start;
                                    SavePosition::new(
                                        result_renderer.render(
                                            result_index,
                                            result_index == selected_index,
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

                let mut column = Flex::column();
                if let Some(error) = self
                    .mixer
                    .as_ref(app)
                    .first_data_source_error()
                    .map(|(.., e)| e)
                {
                    let is_ratelimit_error = error
                        .as_any()
                        .downcast_ref::<GenerateCommandsFromNaturalLanguageError>()
                        .map(|generate_commands_error| {
                            matches!(
                                generate_commands_error,
                                GenerateCommandsFromNaturalLanguageError::RateLimited
                            )
                        })
                        .unwrap_or(false);
                    column.add_child(self.render_error_header(
                        app,
                        error.user_facing_error(),
                        is_ratelimit_error,
                        appearance,
                    ));
                }

                let scrollable_results = Scrollable::vertical(
                    self.state.scroll_state.clone(),
                    UniformList::new(
                        self.state.list_state.clone(),
                        query_result_renderers.len(),
                        build_items,
                    )
                    .notify_visible_items(self.visible_results_range_sender.clone())
                    .finish_scrollable(),
                    styles::SCROLLBAR_WIDTH,
                    appearance.theme().nonactive_ui_detail().into(),
                    appearance.theme().active_ui_detail().into(),
                    Fill::None, // Leave the background transparent
                )
                .finish();

                column
                    .with_child(
                        ConstrainedBox::new(scrollable_results)
                            .with_max_height(styles::VIEW_HEIGHT)
                            .finish(),
                    )
                    .finish()
            }
            _ => self.render_loading_state(appearance),
        }
    }

    /// Renders the input editor and surrounding UI.
    fn render_input_area(&self) -> Box<dyn Element> {
        Container::new(ChildView::new(&self.search_bar).finish())
            .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(
                styles::CORNER_RADIUS - 1.,
            )))
            // Make sure that the search input cursor perfectly aligns with the cursor for
            // the terminal input.
            // TODO(vorporeal): Figure out a way to make this not a magic number.
            .with_uniform_padding(15.)
            .finish()
    }

    /// Returns the `OffsetPositioning` to be used to position the selected result details panel.
    ///
    /// If the selected result is visible within the list, display the details panel to its right,
    /// vertically aligned. If the selected result is below the visible range in the list, display
    /// it aligned to the bottom of the results list.  If the selected result is above the visible
    /// range in the list, display it aligned to the top of the results list.
    fn offset_positioning_for_details_panel(&self, app: &AppContext) -> Option<OffsetPositioning> {
        let selected_result_index = self.search_bar_state.as_ref(app).selected_index()?;
        let selected_result_renderer = self.selected_result_renderer(app)?;
        let visible_results_range = self.state.visible_results_range.as_ref()?;
        let x_axis_positioning = PositioningAxis::relative_to_stack_child(
            PANEL_POSITION_ID,
            PositionedElementOffsetBounds::WindowBySize,
            OffsetType::Pixel(DETAILS_PANEL_MARGIN),
            AnchorPair::new(XAxisAnchor::Right, XAxisAnchor::Left),
        );
        if visible_results_range.contains(&selected_result_index) {
            Some(OffsetPositioning::from_axes(
                x_axis_positioning,
                PositioningAxis::relative_to_stack_child(
                    selected_result_renderer.position_id.clone(),
                    PositionedElementOffsetBounds::WindowByPosition,
                    OffsetType::Pixel(0.),
                    AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Top),
                ),
            ))
        } else {
            let is_result_above_viewport = selected_result_index < visible_results_range.start;
            let vertical_offset = if is_result_above_viewport {
                styles::TOP_PADDING
            } else {
                0.
            };
            let y_axis_anchor = if is_result_above_viewport {
                YAxisAnchor::Top
            } else {
                YAxisAnchor::Bottom
            };
            Some(OffsetPositioning::from_axes(
                x_axis_positioning,
                PositioningAxis::relative_to_stack_child(
                    PANEL_POSITION_ID,
                    PositionedElementOffsetBounds::WindowByPosition,
                    OffsetType::Pixel(vertical_offset),
                    AnchorPair::new(y_axis_anchor, y_axis_anchor),
                ),
            ))
        }
    }

    pub fn menu_positioning(&self) -> MenuPositioning {
        self.menu_positioning
    }

    /// Callback for computing width bounds of the universal search panel (min, max)
    /// Takes window size and returns (min, max) bounds to the resizable element.
    fn compute_panel_width_bounds(window_bounds: Vector2F) -> (f32, f32) {
        (
            window_bounds.x() * MIN_WIDTH_RATIO,
            window_bounds.x() * MAX_WIDTH_RATIO,
        )
    }
}

impl Entity for CommandSearchView {
    type Event = CommandSearchEvent;
}

impl TypedActionView for CommandSearchView {
    type Action = CommandSearchAction;

    fn handle_action(&mut self, action: &CommandSearchAction, ctx: &mut ViewContext<Self>) {
        use CommandSearchAction::*;

        match action {
            Close => self.blur(ctx),
            ResultClicked {
                result_index,
                result_action,
            } => self.handle_result_selected(*result_index, *result_action.clone(), ctx),
            Resize => ctx.emit(CommandSearchEvent::Resize),
            OpenUpgradeLink(upgrade_link) => {
                ctx.open_url(upgrade_link);
            }
            AttemptLoginGatedUpgrade => {
                AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                    auth_manager.attempt_login_gated_feature(
                        "Upgrade AI Usage",
                        AuthViewVariant::RequireLoginCloseable,
                        ctx,
                    )
                });
            }
        }
    }
}

impl View for CommandSearchView {
    fn ui_name() -> &'static str {
        "CommandSearchView"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.search_bar);
        }
    }

    fn accessibility_contents(&self, _ctx: &AppContext) -> Option<AccessibilityContent> {
        Some(AccessibilityContent::new(
            "Command Search".to_owned(),
            "Search your history, workflows, and more.  Use the Up and Down arrows to browse search results after typing.  Press Enter to accept a selected result, inserting it into the terminal input.  Press Escape to close.".to_owned(),
            WarpA11yRole::MenuRole,
        ))
    }

    fn render(&self, app: &AppContext) -> Box<dyn warpui::Element> {
        let appearance = Appearance::as_ref(app);
        let mixer = self.mixer.as_ref(app);

        let should_show_zero_state = self.search_bar.as_ref(app).should_show_zero_state(app);
        let panel_contents_body = if should_show_zero_state {
            ChildView::new(&self.zero_state_handle).finish()
        } else if mixer.is_loading() && mixer.are_results_empty() {
            self.render_loading_state(appearance)
        } else {
            self.render_results(appearance, app)
        };

        let mut panel_children = vec![
            Shrinkable::new(
                1.,
                Container::new(panel_contents_body)
                    .with_padding_top(styles::TOP_PADDING)
                    .with_border(Border::bottom(1.).with_border_fill(appearance.theme().outline()))
                    .with_background(styles::panel_background_fill(appearance))
                    .with_corner_radius(CornerRadius::with_top(Radius::Pixels(
                        styles::CORNER_RADIUS,
                    )))
                    .finish(),
            )
            .finish(),
            self.render_input_area(),
        ];
        if matches!(self.menu_positioning, MenuPositioning::BelowInputBox) {
            panel_children.reverse();
        }
        let panel_contents = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_children(panel_children)
            .finish();

        let panel = Container::new(panel_contents)
            .with_background(appearance.theme().surface_1())
            .with_border(styles::panel_border(appearance))
            .with_drop_shadow(styles::panel_drop_shadow())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                styles::CORNER_RADIUS,
            )))
            .finish();

        let resizable_panel = Resizable::new(self.resizable_state_handle.clone(), panel)
            .on_resize(move |ctx, _| ctx.notify())
            .on_end_resizing(move |ctx, _| ctx.dispatch_typed_action(CommandSearchAction::Resize))
            .with_bounds_callback(Box::new(Self::compute_panel_width_bounds))
            .finish();

        let y_anchor = match self.menu_positioning {
            MenuPositioning::AboveInputBox => {
                AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Bottom)
            }
            MenuPositioning::BelowInputBox => {
                AnchorPair::new(YAxisAnchor::Bottom, YAxisAnchor::Top)
            }
        };

        let mut stack = Stack::new();
        stack.add_positioned_overlay_child(
            SavePosition::new(resizable_panel, PANEL_POSITION_ID).finish(),
            OffsetPositioning::from_axes(
                PositioningAxis::relative_to_parent(
                    ParentOffsetBounds::WindowByPosition,
                    OffsetType::Pixel(0.),
                    AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
                ),
                PositioningAxis::relative_to_parent(
                    ParentOffsetBounds::Unbounded,
                    OffsetType::Pixel(0.),
                    y_anchor,
                ),
            ),
        );

        if !should_show_zero_state {
            if let (Some(selected_result_renderer), Some(details_panel_positioning)) = (
                self.selected_result_renderer(app),
                self.offset_positioning_for_details_panel(app),
            ) {
                if let Some(details) = selected_result_renderer.render_details(app) {
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

        Dismiss::new(Container::new(stack.finish()).with_margin_top(36.).finish())
            .on_dismiss(|ctx, _app| {
                ctx.dispatch_typed_action(CommandSearchAction::Close);
            })
            .finish()
    }
}

#[cfg(feature = "integration_tests")]
impl CommandSearchView {
    pub fn search_bar(&self) -> &ViewHandle<SearchBar<CommandSearchItemAction>> {
        &self.search_bar
    }
}

pub mod styles {
    use lazy_static::lazy_static;
    use pathfinder_color::ColorU;
    use warpui::elements::{Border, DropShadow, ScrollbarWidth};

    use crate::{appearance::Appearance, themes::theme::Fill};

    pub const CORNER_RADIUS: f32 = 8.;
    pub const VIEW_WIDTH: f32 = 700.;
    pub const VIEW_HEIGHT: f32 = 450.;
    pub const TOP_PADDING: f32 = CORNER_RADIUS;
    pub const SCROLLBAR_WIDTH: ScrollbarWidth = ScrollbarWidth::Auto;

    lazy_static! {
        pub static ref SEARCH_ICON_COLOR: ColorU = ColorU::new(255, 255, 255, 204);
        pub static ref INPUT_FIELD_BG_COLOR: ColorU = ColorU::new(255, 255, 255, 50);
    }

    /// Returns the `Fill` to be used as the background of the search results panel and details
    /// panel.
    pub fn panel_background_fill(appearance: &Appearance) -> Fill {
        appearance.theme().surface_2()
    }

    /// Returns the `DropShadow` for both the search results panel and details panel.
    pub fn panel_drop_shadow() -> DropShadow {
        DropShadow::default()
    }

    /// Returns the `Border` for both the search results panel and details panel.
    pub fn panel_border(appearance: &Appearance) -> Border {
        Border::all(1.).with_border_fill(appearance.theme().outline())
    }

    /// Returns a vertical padding value that is sensitive to the user's line height setting. This
    /// value is used to determine the height of each result in the panel.
    pub fn line_height_sensitive_vertical_padding(appearance: &Appearance) -> f32 {
        appearance.line_height_ratio() * appearance.monospace_font_size() * 1.5
    }
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
