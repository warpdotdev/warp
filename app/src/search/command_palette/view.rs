use crate::appearance::Appearance;
use crate::drive::CloudObjectTypeAndId;
use crate::search::binding_source::{BindingFilterFn, BindingSource};
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::SelectedItems;
use crate::search::result_renderer::QueryResultRenderer;
use crate::search::search_bar::SelectionUpdate;
use crate::search::search_bar::{SearchBar, SearchBarEvent, SearchBarState, SearchResultOrdering};
use crate::search::QueryFilter;
use crate::send_telemetry_from_ctx;
use crate::server::telemetry::LaunchConfigUiLocation;
use crate::server::telemetry::TelemetryEvent;
use crate::settings::CtrlTabBehavior;
use crate::terminal::keys_settings::KeysSettings;
use crate::themes::theme::WarpTheme;
use crate::view_components::DismissibleToast;
use crate::ToastStack;
use lazy_static::lazy_static;
use warp_core::send_telemetry_from_app_ctx;
use warp_util::path::LineAndColumnArg;

use crate::search::action::search_item::MatchedBinding;
use itertools::Itertools;
use warpui::elements::DispatchEventResult;
use warpui::elements::EventHandler;
use warpui::event::KeyState;
use warpui::platform::keyboard::KeyCode;
use warpui::FocusContext;

use crate::search::command_palette::zero_state::{self, Event as ZeroStateEvent, ZeroState};
use crate::search::data_source::QueryResult;

use std::collections::HashSet;
use std::ops::Deref;
use std::sync::Arc;

use crate::features::FeatureFlag;
use crate::palette::PaletteMode;
use crate::root_view::OpenLaunchConfigArg;
use crate::search::command_palette::data_sources::DataSourceStore;
use crate::server::ids::SyncId;
use crate::session_management::SessionSource;
use crate::workspace::{active_terminal_in_window, ForkedConversationDestination, WorkspaceAction};
use warpui::elements::{
    Align, Border, ChildView, Clipped, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox,
    Container, CornerRadius, Dismiss, Empty, Fill, Flex, ParentElement, Radius, SavePosition,
    Shrinkable,
};
use warpui::keymap::BindingId;
use warpui::units::{IntoPixels, Pixels};
use warpui::{
    AppContext, Element, Entity, EntityId, ModelHandle, SingletonEntity, TypedActionView,
    ViewContext, ViewHandle, WindowId,
};

use super::super::palette_styles as styles;
use super::CommandPaletteMixer;

lazy_static! {
    /// Set of hardcoded action names that we want to show in the command palette zero state.
    static ref SUGGESTED_ACTIONS: HashSet<&'static str> = HashSet::from_iter(
        [
            if FeatureFlag::AgentMode.is_enabled() { "input:toggle_input_type" } else { "workspace:toggle_ai_assistant" },
            "workspace:show_theme_chooser",
            "workspace:create_personal_workflow",
        ]
    );
}

/// Position ID for the command palette list.
const PALETTE_LIST_SAVE_POSITION_ID: &str = "command_palette:list";

/// Max number of results to be returned by the search mixer. We set this to an arbitrarily
/// large size to minimize performances issues caused by rendering the elements of the palette
/// using a [`ClippedScrollable`].
// TODO(alokedesai): Remove once we add a properly viewported element.
const MAX_SEARCH_RESULTS: usize = 250;

/// Number of recently selected items to show in the zero state.
const NUM_RECENT_ITEMS_IN_ZERO_STATE: usize = 3;

struct ViewState {
    clipped_scroll_state: ClippedScrollStateHandle,
}

#[derive(Debug)]
pub enum Action {
    ResultClicked { action: CommandPaletteItemAction },
    Close,
    CtrlPressed(bool),
}

#[derive(Debug)]
pub enum Event {
    Close {
        accepted_action_type: Option<&'static str>,
    },
    /// Execute the workflow identified by `id`.
    ExecuteWorkflow { id: SyncId },
    /// Invoke the env vars identified by `id`.
    InvokeEnvironmentVariables { id: SyncId },
    /// Open a notebook identified by `id`.
    OpenNotebook { id: SyncId },
    /// View the relevant object in the Warp Drive sidebar.
    ViewInWarpDrive { id: CloudObjectTypeAndId },
    /// Open a file at the given path.
    OpenFile {
        path: String,
        line_and_column_arg: Option<LineAndColumnArg>,
    },
    /// Open a directory at the given path.
    OpenDirectory { path: String },
}

#[derive(Debug, Clone, Default)]
pub enum NavigationMode {
    #[default]
    Normal,

    // Palette was entered via ctrl-tab for quick session switching.
    CtrlTab,
}

/// A view that renders the command palette and allows users to optionally apply a [`QueryFilter`]
/// to filter results.
pub struct View {
    pub search_bar: ViewHandle<SearchBar<CommandPaletteItemAction>>,
    search_bar_state: ModelHandle<SearchBarState<CommandPaletteItemAction>>,
    state: ViewState,
    binding_source: ModelHandle<BindingSource>,
    /// Model to lists the current active session.
    session_source: ModelHandle<SessionSource>,
    zero_state_handle: ViewHandle<ZeroState>,
    /// Placeholder element to render when no results are found.
    placeholder_query_renderer: QueryResultRenderer<CommandPaletteItemAction>,
    /// List of [`BindingId`]s that should be shown in the zero state as "suggested" items.
    suggested_binding_ids: Vec<BindingId>,
    /// Store of all the data sources that should be used for the [`SearchMixer`].
    pub data_source_store: ModelHandle<DataSourceStore>,
    zero_state_items: ModelHandle<zero_state::Items>,

    /// The current navigation mode.
    navigation_mode: NavigationMode,

    /// Whether the active session is a shared session viewer.
    /// This is set by the workspace when opening the palette.
    is_shared_session_viewer: bool,
}

impl Entity for View {
    type Event = Event;
}

impl TypedActionView for View {
    type Action = Action;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            Action::ResultClicked { action } => {
                self.handle_result_accepted(action.clone(), ctx);
            }
            Action::Close => self.close(ctx, None),
            Action::CtrlPressed(pressed) => {
                if !*pressed && matches!(self.navigation_mode, NavigationMode::CtrlTab) {
                    // Accept the selected item and reset the navigation mode on release of Ctrl key.
                    self.accept_selected_item(ctx);
                }
            }
        }
    }
}

impl warpui::View for View {
    fn ui_name() -> &'static str {
        "CommandPaletteView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let body = if self.search_bar.as_ref(app).should_show_zero_state(app) {
            ChildView::new(&self.zero_state_handle).finish()
        } else {
            self.render_palette_list(theme, app)
        };

        let mut palette = Flex::column();
        if matches!(self.navigation_mode, NavigationMode::Normal) {
            // Don't show the search bar when navigating with ctrl-tab.
            palette.add_child(self.render_search_bar());
        }
        palette.add_child(Shrinkable::new(1., body).finish());

        EventHandler::new(
            Align::new(
                Dismiss::new(
                    Container::new(
                        ConstrainedBox::new(palette.finish())
                            .with_width(styles::PALETTE_WIDTH)
                            .with_max_height(styles::PALETTE_HEIGHT)
                            .finish(),
                    )
                    .with_background(theme.surface_2())
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                    .with_border(Border::all(1.0).with_border_fill(theme.outline()))
                    .with_margin_top(117.)
                    .with_padding_bottom(10.)
                    .with_drop_shadow(*styles::DROP_SHADOW)
                    .finish(),
                )
                .on_dismiss(|ctx, _app| ctx.dispatch_typed_action(Action::Close))
                .prevent_interaction_with_other_elements()
                .finish(),
            )
            .top_center()
            .finish(),
        )
        .on_modifier_state_changed(|ctx, _, key_code, state| {
            if matches!(key_code, KeyCode::ControlLeft | KeyCode::ControlRight) {
                ctx.dispatch_typed_action(Action::CtrlPressed(matches!(state, KeyState::Pressed)));
            }
            DispatchEventResult::StopPropagation
        })
        .finish()
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.search_bar);
        }
    }
}

impl View {
    pub fn new(navigation_mode: NavigationMode, ctx: &mut ViewContext<Self>) -> Self {
        let search_bar_state = ctx.add_model(|_ctx| {
            SearchBarState::new(SearchResultOrdering::TopDown).with_max_results(MAX_SEARCH_RESULTS)
        });

        ctx.subscribe_to_model(&search_bar_state, |me, _, event, ctx| {
            me.handle_search_bar_event(event, ctx);
        });

        let binding_source = ctx.add_model(|_| BindingSource::None);
        let session_source = ctx.add_model(|_| SessionSource::None);

        let data_source_store = ctx.add_model(|ctx| {
            DataSourceStore::new(binding_source.clone(), session_source.clone(), ctx)
        });

        ctx.observe(&binding_source, |me, _, ctx| {
            me.on_binding_source_changed(ctx)
        });

        ctx.observe(&session_source, |me, _, ctx| {
            me.on_session_source_changed(ctx)
        });

        let zero_state_items = ctx.add_model(|_| zero_state::Items::new());
        let zero_state =
            ctx.add_typed_action_view(|ctx| ZeroState::new(zero_state_items.clone(), ctx));

        ctx.subscribe_to_view(&zero_state, |me, _, event, ctx| {
            me.handle_zero_state_event(event, ctx);
        });

        ctx.observe(&search_bar_state, |_, _, ctx| ctx.notify());

        // Compute the list of binding IDs that we should show the suggested actions for based. Key
        // bindings are only registered once, so we only need to do this in the constructor.
        let suggested_binding_ids = SUGGESTED_ACTIONS
            .iter()
            .flat_map(|name| ctx.get_binding_by_name(name).map(|binding| binding.id))
            .collect_vec();

        let mixer = ctx.add_model(|_| CommandPaletteMixer::new());
        data_source_store.update(ctx, |store, ctx| {
            store.reset_search_mixer(mixer.clone(), false, ctx);
            ctx.notify();
        });

        let ui_font_family = Appearance::as_ref(ctx).ui_font_family();

        let search_bar = ctx.add_typed_action_view(|ctx| {
            SearchBar::new(
                mixer.clone(),
                search_bar_state.clone(),
                "Search for a command",
                Self::create_query_result_renderer,
                ctx,
            )
            .with_font_family(ui_font_family, ctx)
        });

        ctx.subscribe_to_view(&search_bar, |me, _, event, ctx| {
            me.handle_search_bar_event(event, ctx);
        });

        let placeholder_element = QueryResultRenderer::new(
            MatchedBinding::placeholder("No results found".into()).into(),
            "command_palette:no_results".into(),
            |_, _, _| {},
            *styles::QUERY_RESULT_RENDERER_STYLES,
        );

        Self {
            navigation_mode,
            search_bar,
            search_bar_state,
            state: ViewState {
                clipped_scroll_state: Default::default(),
            },
            binding_source,
            session_source,
            data_source_store,
            zero_state_handle: zero_state,
            placeholder_query_renderer: placeholder_element,
            suggested_binding_ids,
            zero_state_items,
            is_shared_session_viewer: false,
        }
    }

    #[cfg(feature = "integration_tests")]
    /// Returns the current search results within the command palette. Used within integration tests
    /// to verify the command palette returns the correct results when launch configurations or the
    /// current session changes.
    pub fn search_results<'a>(
        &'a self,
        app: &'a AppContext,
    ) -> impl Iterator<Item = &'a QueryResult<CommandPaletteItemAction>> + 'a {
        let query_results = self.search_bar_state.as_ref(app).query_result_renderers();
        query_results
            .into_iter()
            .flat_map(|results| results.iter())
            .map(|item| &item.search_result)
    }

    /// Set the active query filter in the search bar to be `filter`.
    pub fn set_active_query_filter(&mut self, filter: QueryFilter, ctx: &mut ViewContext<Self>) {
        self.search_bar.update(ctx, |view, ctx| {
            view.set_query_filter(Some((filter, filter.filter_atom().primary_text)), ctx)
        });
        ctx.notify();
    }

    pub fn set_initial_selection_offset(&mut self, offset: isize, ctx: &mut ViewContext<Self>) {
        self.search_bar_state.update(ctx, move |state, _ctx| {
            state.offset_initial_selection_by(offset);
        });
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

    fn accept_selected_item(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(result) = self.search_bar_state.as_ref(ctx).selected_result() {
            self.handle_result_accepted(result.accept_result().clone(), ctx);
        }
    }

    /// Returns the active query filters
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
                | (PaletteMode::Files, QueryFilter::Files)
                | (PaletteMode::Conversations, QueryFilter::Conversations)
                | (PaletteMode::WarpDrive, QueryFilter::Drive)
        )
    }

    /// Sets the new [`SessionSource`].
    pub fn set_session_source(
        &mut self,
        session_source: SessionSource,
        ctx: &mut ViewContext<Self>,
    ) {
        self.session_source.update(ctx, |binding_source, ctx| {
            *binding_source = session_source;
            ctx.notify();
        });
    }

    /// Sets whether the active session is a shared session viewer.
    /// This should be called by the workspace before opening the palette.
    pub fn set_is_shared_session_viewer(&mut self, is_viewer: bool, ctx: &mut ViewContext<Self>) {
        self.is_shared_session_viewer = is_viewer;

        let mixer = self.search_bar.as_ref(ctx).mixer().clone();
        self.data_source_store.update(ctx, |store, ctx| {
            store.reset_search_mixer(mixer.clone(), self.is_shared_session_viewer, ctx);
            ctx.notify();
        });
    }

    fn handle_zero_state_event(&mut self, event: &ZeroStateEvent, ctx: &mut ViewContext<Self>) {
        match event {
            ZeroStateEvent::FilterChipSelected { filter } => {
                self.set_active_query_filter(*filter, ctx);
            }
        }
    }

    fn create_query_result_renderer(
        index: usize,
        result: QueryResult<CommandPaletteItemAction>,
    ) -> QueryResultRenderer<CommandPaletteItemAction> {
        QueryResultRenderer::new(
            result,
            Self::query_result_save_position_id(index),
            |_result_index, action, event_ctx| {
                event_ctx.dispatch_typed_action(Action::ResultClicked { action })
            },
            *styles::QUERY_RESULT_RENDERER_STYLES,
        )
    }

    /// Returns the position ID for a query result at `index`.
    fn query_result_save_position_id(index: usize) -> String {
        format!("command_palette:query_result:{index}")
    }

    /// Sets the set the binding source to produce the list of command bindings in the current
    /// context.
    pub fn set_binding_source(
        &mut self,
        window_id: WindowId,
        view_id: EntityId,
        ctx: &mut ViewContext<Self>,
    ) {
        let ctrl_tab_behavior = *KeysSettings::as_ref(ctx).ctrl_tab_behavior;
        let binding_filter_fn: BindingFilterFn =
            if matches!(ctrl_tab_behavior, CtrlTabBehavior::CycleMostRecentSession) {
                Some(Arc::new(|binding| {
                    if let Some(action) = &binding.action {
                        // Filter out the cycle next/prev session actions from the palette if ctrl-tab
                        // behavior is set to cycle most/least recent session. Clicking on them or hitting enter
                        // doesn't make sense because the action needs to be triggered from a ctrl-tab only (with
                        // ctrl key held down).
                        !matches!(
                            action.as_any().downcast_ref::<WorkspaceAction>(),
                            Some(WorkspaceAction::CycleNextSession)
                                | Some(WorkspaceAction::CyclePrevSession)
                        )
                    } else {
                        true
                    }
                }))
            } else {
                None
            };
        self.binding_source.update(ctx, move |binding_source, ctx| {
            *binding_source = BindingSource::View {
                window_id,
                view_id,
                binding_filter_fn,
            };
            ctx.notify();
        });
    }

    fn on_binding_source_changed(&mut self, ctx: &mut ViewContext<Self>) {
        let data_source_store = self.data_source_store.as_ref(ctx);

        // The binding source changed, recompute the bindings that could be suggested given the
        // current set of bindings that are focused.
        let suggested_query_renderers = self
            .suggested_binding_ids
            .iter()
            .filter_map(|binding_id| {
                data_source_store.query_result_for_binding_id(*binding_id, ctx)
            })
            .enumerate()
            .map(|(idx, item)| Self::create_query_result_renderer(idx, item))
            .collect_vec();

        self.zero_state_items.update(ctx, |items, ctx| {
            items.set_suggested_items(suggested_query_renderers, ctx);
        });

        self.compute_recent_items_for_zero_state(ctx);
    }

    fn on_session_source_changed(&mut self, ctx: &mut ViewContext<Self>) {
        self.compute_recent_items_for_zero_state(ctx);

        self.search_bar.update(ctx, |search_bar, ctx| {
            search_bar.run_query(ctx);
        });
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
            // The QueryFilterChanged event is deferred (fires after the current
            // view update returns). When switching to the Files filter,
            // open_files_palette has already called reset_search_mixer and
            // run_query.  Resetting the mixer here would abort the in-flight
            // async file search without re-running it, leaving the palette
            // empty.
            SearchBarEvent::QueryFilterChanged { .. } => {}
            SearchBarEvent::SelectionUpdateInZeroState { selection_update } => {
                self.zero_state_items.update(ctx, |items, ctx| {
                    items.handle_selection_update(*selection_update, ctx);
                })
            }
            SearchBarEvent::EnterInZeroState { modified_enter } => {
                if let Some(query_result) = self.zero_state_items.as_ref(ctx).selected_item() {
                    let action = if *modified_enter {
                        query_result.search_result.execute_result()
                    } else {
                        query_result.search_result.accept_result()
                    };

                    self.handle_result_accepted(action, ctx);
                }
            }
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

        let scroll_top = self.state.clipped_scroll_state.scroll_start();
        self.state
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

        self.state.clipped_scroll_state = Default::default();
        self.reset(ctx);

        // Some of the actions that are dispatched before closing can close the Window (e.g. "Close
        // Tab" on the final tab of the window). Confirm that the Window still exists before trying
        // to update the view.
        if ctx.root_view_id(ctx.window_id()).is_some() {
            ctx.emit(Event::Close {
                accepted_action_type,
            });
        }
    }

    pub fn reset(&mut self, ctx: &mut ViewContext<Self>) {
        self.state.clipped_scroll_state.scroll_to(Pixels::zero());
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

    /// Recompute the items shown in the "recent" section for the zero state. We compute this after
    /// the [`BindingSource`] and [`SessionSource`] models have changed since both data sources
    /// require reading state about a current view from the UI framework. However, a quirk of the UI
    /// framework is that the framework _removes_ a view from its internal state before calling an
    /// action handler to get around Rust lifetime issues (it reinserts the view after the handler
    /// has been called). This creates a dependency where we would compute an incorrect set of
    /// recent results if we tried to call this function from a `Workspace` handler -- the app would
    /// not know about the `Workspace` and would return an incomplete set of bindings and sessions.
    ///
    /// As a workaround, we recompute these when the [`BindingSource`] and [`SessionSource`] models
    /// change. The model update handler is called after any view handlers, so it won't run into the
    /// same restrictions.
    fn compute_recent_items_for_zero_state(&mut self, ctx: &mut ViewContext<View>) {
        let data_source_store = self.data_source_store.as_ref(ctx);
        let selected_items = SelectedItems::as_ref(ctx);

        let query_results = selected_items
            .iter()
            .filter_map(|summary| data_source_store.query_result_from_summary(summary, ctx))
            .enumerate()
            .map(|(idx, item)| Self::create_query_result_renderer(idx, item))
            .take(NUM_RECENT_ITEMS_IN_ZERO_STATE)
            .collect_vec();

        self.zero_state_items.update(ctx, |items, ctx| {
            items.set_recent_items(query_results, ctx);
        });
    }

    /// Inserts `query` into the search bar.
    pub fn insert_query_text(&mut self, query: &str, ctx: &mut ViewContext<Self>) {
        self.search_bar.update(ctx, |search_bar, ctx| {
            search_bar.insert_query_text(query, ctx);
        })
    }

    fn render_palette_list(&self, theme: &WarpTheme, app: &AppContext) -> Box<dyn Element> {
        match self.search_bar_state.as_ref(app).query_result_renderers() {
            None => Empty::new().finish(),
            Some(renderers) if renderers.is_empty() => {
                self.placeholder_query_renderer
                    .render(0, true /* is_selected */, app)
            }
            Some(renderers) => {
                let selected_index = self.search_bar_state.as_ref(app).selected_index();
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
                        self.state.clipped_scroll_state.clone(),
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
        }
    }

    /// Handles the `CommandPaletteItemAction` action and closes the search panel.
    fn handle_result_accepted(
        &mut self,
        result_action: CommandPaletteItemAction,
        ctx: &mut ViewContext<Self>,
    ) {
        // Tab navigations don't appear in the main command palette to avoid confusion with session
        // navigations, so they can't evict real recent items from SelectedItems.
        if !matches!(
            result_action,
            CommandPaletteItemAction::NavigateToTab { .. }
        ) {
            let selected_items_handle = SelectedItems::handle(ctx);
            selected_items_handle.update(ctx, |selected_items, _ctx| {
                selected_items.enqueue(result_action.to_summary())
            });
        }

        if let CommandPaletteItemAction::AcceptBinding { binding } = &result_action {
            if let Some(action) = &binding.action {
                match action.as_any().downcast_ref::<WorkspaceAction>() {
                    Some(WorkspaceAction::TogglePalette {
                        mode: PaletteMode::LaunchConfig,
                        source: _,
                    }) => {
                        self.reset(ctx);
                        self.set_active_query_filter(QueryFilter::LaunchConfigurations, ctx);
                        return;
                    }
                    Some(WorkspaceAction::TogglePalette {
                        mode: PaletteMode::Navigation,
                        source: _,
                    }) => {
                        self.reset(ctx);
                        self.set_active_query_filter(QueryFilter::Sessions, ctx);
                        return;
                    }
                    Some(WorkspaceAction::TogglePalette {
                        mode: PaletteMode::Files,
                        source: _,
                    }) => {
                        self.reset(ctx);
                        self.set_active_query_filter(QueryFilter::Files, ctx);
                        return;
                    }
                    Some(WorkspaceAction::TogglePalette {
                        mode: PaletteMode::Conversations,
                        source: _,
                    }) => {
                        self.reset(ctx);
                        self.set_active_query_filter(QueryFilter::Conversations, ctx);
                        return;
                    }
                    Some(WorkspaceAction::TogglePalette {
                        mode: PaletteMode::Command,
                        source: _,
                    }) => {
                        self.close(ctx, Some(result_action.result_type()));
                        return;
                    }
                    _ => {}
                }
            }
        }

        match result_action.clone() {
            CommandPaletteItemAction::AcceptBinding { binding } => {
                if let Some(action) = binding.action.as_deref() {
                    self.dispatch_typed_action_on_view(action, ctx);
                };
            }
            CommandPaletteItemAction::NavigateToSession {
                pane_view_locator,
                window_id,
            } => {
                if let Some(root_view_id) = ctx.root_view_id(window_id) {
                    ctx.dispatch_action_for_view(
                        window_id,
                        root_view_id,
                        "root_view:handle_pane_navigation_event",
                        &pane_view_locator,
                    );
                }

                send_telemetry_from_ctx!(TelemetryEvent::SelectNavigationPaletteItem, ctx);
            }
            CommandPaletteItemAction::NavigateToTab {
                pane_group_id,
                window_id,
            } => {
                if let Some(root_view_id) = ctx.root_view_id(window_id) {
                    ctx.dispatch_action_for_view(
                        window_id,
                        root_view_id,
                        "root_view:activate_tab_by_pane_group_id",
                        &pane_group_id,
                    );
                }
                send_telemetry_from_ctx!(TelemetryEvent::SelectNavigationPaletteItem, ctx);
            }
            CommandPaletteItemAction::NavigateToConversation {
                pane_view_locator,
                window_id,
                conversation_id,
                terminal_view_id,
            } => {
                let should_block = {
                    window_id
                        .and_then(|window_id| {
                            active_terminal_in_window(window_id, ctx, |terminal_view, ctx| {
                                !terminal_view
                                    .ai_context_model()
                                    .as_ref(ctx)
                                    .can_start_new_conversation()
                            })
                        })
                        .unwrap_or(false)
                };

                if should_block {
                    if let Some(window_id) = window_id {
                        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                            toast_stack.add_ephemeral_toast(
                                DismissibleToast::error(
                                    "Cannot switch conversations while agent is monitoring a command."
                                        .to_string(),
                                ),
                                window_id,
                                ctx,
                            );
                        });
                    }
                    return;
                }

                ctx.dispatch_typed_action(&WorkspaceAction::RestoreOrNavigateToConversation {
                    pane_view_locator,
                    window_id,
                    conversation_id,
                    terminal_view_id,
                    restore_layout: None,
                });
                send_telemetry_from_app_ctx!(TelemetryEvent::SelectNavigationPaletteItem, ctx);
            }
            CommandPaletteItemAction::ForkConversation { conversation_id } => {
                ctx.dispatch_typed_action(&WorkspaceAction::ForkAIConversation {
                    conversation_id,
                    fork_from_exchange: None,
                    summarize_after_fork: false,
                    summarization_prompt: None,
                    initial_prompt: None,
                    destination: ForkedConversationDestination::SplitPane,
                });
            }
            CommandPaletteItemAction::OpenLaunchConfiguration {
                open_in_active_window,
                config,
            } => {
                ctx.dispatch_global_action(
                    "root_view:open_launch_config",
                    OpenLaunchConfigArg {
                        open_in_active_window,
                        launch_config: config.deref().clone(),
                        ui_location: LaunchConfigUiLocation::CommandPalette,
                    },
                );
            }
            CommandPaletteItemAction::ExecuteWorkflow { id } => {
                ctx.emit(Event::ExecuteWorkflow { id })
            }
            CommandPaletteItemAction::InvokeEnvironmentVariables { id } => {
                ctx.emit(Event::InvokeEnvironmentVariables { id })
            }
            CommandPaletteItemAction::OpenNotebook { id } => ctx.emit(Event::OpenNotebook { id }),
            CommandPaletteItemAction::ViewInWarpDrive { id } => {
                ctx.emit(Event::ViewInWarpDrive { id })
            }
            CommandPaletteItemAction::NewSession { source } => {
                self.dispatch_typed_action_on_view(source.action().deref(), ctx);
            }
            CommandPaletteItemAction::OpenFile {
                path,
                project_directory,
                line_and_column_arg,
            } => {
                let absolute_path = std::path::Path::new(&project_directory)
                    .join(&path)
                    .to_string_lossy()
                    .to_string();

                ctx.emit(Event::OpenFile {
                    path: absolute_path,
                    line_and_column_arg,
                });
            }
            CommandPaletteItemAction::OpenDirectory {
                path,
                project_directory,
            } => {
                let absolute_path = std::path::Path::new(&project_directory)
                    .join(&path)
                    .to_string_lossy()
                    .to_string();

                ctx.emit(Event::OpenDirectory {
                    path: absolute_path,
                });
            }
            CommandPaletteItemAction::CreateFile {
                file_name,
                current_directory,
            } => {
                let file_path = std::path::Path::new(&current_directory).join(&file_name);

                if let Err(e) = std::fs::File::create_new(&file_path) {
                    if e.kind() != std::io::ErrorKind::AlreadyExists {
                        log::warn!("Failed to create file {}: {e}", file_path.display());
                        return;
                    }
                }

                ctx.emit(Event::OpenFile {
                    path: file_path.to_string_lossy().to_string(),
                    line_and_column_arg: None,
                });
            }
            CommandPaletteItemAction::NewConversationInProject {
                path: _,
                project_name,
            } => {
                // AcceptProject is handled by the welcome palette, not the regular command palette.
                // This case should not normally be reached in the command palette context, but we
                // include it for completeness. If this somehow gets executed, we'll just log it.
                log::warn!(
                    "OpenProjectConvo action unexpectedly handled in command palette for project: {project_name}"
                );
            }
            CommandPaletteItemAction::NewConversation => {
                let window_id = match self.binding_source.as_ref(ctx) {
                    BindingSource::View { window_id, .. } => *window_id,
                    BindingSource::None => return,
                };

                let (terminal_view_id, can_start_new_conversation) = {
                    let terminal_view_id =
                        active_terminal_in_window(window_id, ctx, |terminal_view, _| {
                            terminal_view.id()
                        });

                    let should_block =
                        active_terminal_in_window(window_id, ctx, |terminal_view, ctx| {
                            !terminal_view
                                .ai_context_model()
                                .as_ref(ctx)
                                .can_start_new_conversation()
                        })
                        .unwrap_or(false);

                    (terminal_view_id, should_block)
                };

                if can_start_new_conversation {
                    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                        toast_stack.add_ephemeral_toast(
                            DismissibleToast::error(
                                "Cannot start a new conversation while agent is monitoring a command.".to_string(),
                            ),
                            window_id,
                            ctx,
                        );
                    });
                    return;
                }

                if let Some(terminal_view_id) = terminal_view_id {
                    ctx.dispatch_typed_action(&WorkspaceAction::StartNewConversation {
                        terminal_view_id,
                    });
                }
            }
            CommandPaletteItemAction::NoOp => {
                // No-op action (used for non-interactable separator items that don't do anything on click).
            }
        }

        self.close(ctx, Some(result_action.result_type()));
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
