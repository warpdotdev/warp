use std::collections::HashSet;
use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use async_channel::Sender;
use itertools::Itertools;
use lazy_static::lazy_static;
use warp_core::r#async::debounce;
use warp_core::send_telemetry_from_app_ctx;
use warp_util::path::LineAndColumnArg;
use warpui::elements::{
    Align, Border, ChildView, Clipped, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox,
    Container, CornerRadius, Dismiss, DispatchEventResult, Empty, EventHandler, Fill, Flex,
    ParentElement, Radius, SavePosition, Shrinkable, Text,
};
use warpui::event::KeyState;
use warpui::keymap::BindingId;
use warpui::platform::keyboard::KeyCode;
use warpui::units::{IntoPixels, Pixels};
use warpui::{
    AppContext, Element, Entity, EntityId, FocusContext, ModelHandle, SingletonEntity,
    TypedActionView, ViewContext, ViewHandle, WindowId,
};

use super::super::palette_styles as styles;
use super::CommandPaletteMixer;
use crate::appearance::Appearance;
use crate::drive::CloudObjectTypeAndId;
use crate::features::FeatureFlag;
use crate::palette::PaletteMode;
use crate::root_view::OpenLaunchConfigArg;
use crate::search::QueryFilter;
use crate::search::action::search_item::MatchedBinding;
use crate::search::binding_source::{BindingFilterFn, BindingSource};
use crate::search::command_palette::SelectedItems;
use crate::search::command_palette::data_sources::DataSourceStore;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::zero_state::{self, Event as ZeroStateEvent, ZeroState};
use crate::search::data_source::QueryResult;
use crate::search::result_renderer::QueryResultRenderer;
use crate::search::search_bar::{
    SearchBar, SearchBarEvent, SearchBarState, SearchResultOrdering, SelectionUpdate,
};
use crate::server::ids::SyncId;
use crate::server::telemetry::{LaunchConfigUiLocation, TelemetryEvent};
use crate::session_management::SessionSource;
use crate::settings::CtrlTabBehavior;
use crate::terminal::cli_agent_sessions::transcript_digest::{DigestStatus, TranscriptDigestModel};
use crate::terminal::keys_settings::KeysSettings;
use crate::themes::theme::WarpTheme;
use crate::ui_components::blended_colors;
use crate::view_components::DismissibleToast;
use crate::workspace::{ForkedConversationDestination, WorkspaceAction, active_terminal_in_window};
use crate::{ToastStack, send_telemetry_from_ctx};

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

/// How long typing must pause before the session-search popup asks the digest
/// for transcript content.
///
/// The name half of the popup is **not** debounced — it is an in-memory fuzzy
/// match and runs on every keystroke. This delay is only for the half that
/// reads the disk, and matches the one global search settled on for the same
/// job.
const CONTENT_SEARCH_DEBOUNCE_PERIOD: Duration = Duration::from_millis(300);

/// What the session-search popup's content search can and cannot see.
///
/// Not optional and not decoration. The digest holds conversation text only —
/// tool output and pasted file bodies are deliberately excluded — it is built
/// from this machine's disk, Codex and the other CLI agents keep no per-cwd
/// transcript store to read, and the scan behind the corpus lists the sixteen
/// newest sessions per project. A search that silently half-answers is worse
/// than one that says what it covers.
const CONTENT_SEARCH_SCOPE_LABEL: &str =
    "conversation text · this device only · Claude sessions · 16 most recent per project";

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

    /// Palette is the session-search popup: names from memory, transcript
    /// content from the digest model. Everything the digest drives — the
    /// debounced query, the footer, the suppressed placeholder — is gated on
    /// this, so the two ordinary palettes carry none of it.
    SessionSearch,
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

    /// Pings the debounced transcript-content search. Carries `()` rather than
    /// the query text so the handler reads whatever is in the search box when
    /// it fires — the debounce can only ever be behind the editor, never ahead
    /// of it.
    content_search_tx: Sender<()>,

    /// The offset [`SelectionUpdate::Top`] lands on, remembered so the
    /// digest-finished re-run can borrow it for exactly one pass.
    initial_selection_offset: isize,

    /// Set while a digest-finished re-run is in flight. See
    /// [`Self::handle_transcript_digest_finished`].
    restore_selection_offset: bool,
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
        if matches!(
            self.navigation_mode,
            NavigationMode::Normal | NavigationMode::SessionSearch
        ) {
            // Don't show the search bar when navigating with ctrl-tab. Session
            // search is driven entirely by typing, so it needs the bar as much
            // as the normal palette does.
            palette.add_child(self.render_search_bar());
        }
        palette.add_child(Shrinkable::new(1., body).finish());
        if let Some(footer) = self.render_session_search_footer(app) {
            palette.add_child(footer);
        }

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

        let window_id = ctx.window_id();
        let data_source_store = ctx.add_model(|ctx| {
            DataSourceStore::new(
                binding_source.clone(),
                session_source.clone(),
                window_id,
                ctx,
            )
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

        ctx.observe(&search_bar_state, |me, _, ctx| {
            me.on_search_bar_state_changed(ctx)
        });

        // The session-search popup's second half: transcript content, searched
        // by a model of its own so this palette's mixer can stay entirely
        // synchronous. Only that popup pays for any of it.
        let (content_search_tx, content_search_rx) = async_channel::unbounded();
        if matches!(navigation_mode, NavigationMode::SessionSearch) {
            ctx.spawn_stream_local(
                debounce(CONTENT_SEARCH_DEBOUNCE_PERIOD, content_search_rx),
                Self::handle_debounced_content_search,
                |_, _| {},
            );

            if ctx.has_singleton_model::<TranscriptDigestModel>() {
                let digest = TranscriptDigestModel::handle(ctx);
                // Notify only: the footer's "searching…" line has to appear
                // when a pass starts, and the model notifies then.
                ctx.observe(&digest, |_, _, ctx| ctx.notify());
                ctx.subscribe_to_model(&digest, |me, _, _event, ctx| {
                    me.handle_transcript_digest_finished(ctx);
                });
            }
        }

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
            content_search_tx,
            initial_selection_offset: 0,
            restore_selection_offset: false,
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
        self.initial_selection_offset = offset;
        self.apply_selection_offset(offset, ctx);
    }

    /// Applies an offset without recording it as the palette's own — used to
    /// lend it out for one re-run and take it back.
    fn apply_selection_offset(&self, offset: isize, ctx: &mut ViewContext<Self>) {
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
                | (PaletteMode::SessionSearch, QueryFilter::AgentSessions)
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

    /// The results — and therefore, one keystroke earlier, the query — changed.
    ///
    /// The search bar has no "query changed" event, but it re-runs its query on
    /// every keystroke and every run ends here, so this is the hook the content
    /// search hangs off.
    fn on_search_bar_state_changed(&mut self, ctx: &mut ViewContext<Self>) {
        if matches!(self.navigation_mode, NavigationMode::SessionSearch) {
            // The offset was lent to the digest's re-run and this is the
            // results change that re-run produced, so take it back — otherwise
            // the *next* keystroke would select a row from the middle of the
            // list instead of the first one.
            if self.restore_selection_offset {
                self.restore_selection_offset = false;
                self.apply_selection_offset(self.initial_selection_offset, ctx);
            }
            // The digest drops a query it has already answered, so the re-run
            // this ping will eventually cause cannot start another search.
            let _ = self.content_search_tx.try_send(());
        }
        ctx.notify();
    }

    /// Typing has paused: search transcript content for whatever is in the box.
    ///
    /// Reads the query here rather than carrying it through the channel, so a
    /// ping that was queued behind two more keystrokes still asks about the
    /// text the user is actually looking at.
    fn handle_debounced_content_search(&mut self, _ping: (), ctx: &mut ViewContext<Self>) {
        if !ctx.has_singleton_model::<TranscriptDigestModel>() {
            return;
        }
        let query = self.search_bar.as_ref(ctx).query(ctx);
        TranscriptDigestModel::handle(ctx).update(ctx, |digest, ctx| {
            digest.set_query(query, ctx);
        });
    }

    /// A content search finished: publish its hits and show them.
    ///
    /// The mixer has no idea the digest exists — the content data source only
    /// ever serves what it has been handed — so the hits are pushed in and the
    /// query is re-run once. This is the only re-run the digest ever causes.
    fn handle_transcript_digest_finished(&mut self, ctx: &mut ViewContext<Self>) {
        if !matches!(self.navigation_mode, NavigationMode::SessionSearch)
            || !ctx.has_singleton_model::<TranscriptDigestModel>()
        {
            return;
        }

        let digest = TranscriptDigestModel::as_ref(ctx);
        let (query, hits) = (digest.query().to_owned(), digest.hits().to_vec());
        self.data_source_store.update(ctx, |store, ctx| {
            store.set_agent_session_content_results(query, hits, ctx);
        });

        // Re-running the query makes the search bar reselect its top row, which
        // would yank the selection out from under a user who has already
        // arrowed down. Content rows sort strictly *below* the name rows, so an
        // index that named a row before the re-run names the same row after it
        // — point the offset at that row and the reselect becomes a no-op.
        // `on_search_bar_state_changed` takes the offset back on the very
        // results change this re-run produces. That handshake is safe in both
        // orderings: if the observer runs *before* the deferred selection
        // update, the offset is already back and the selection resets exactly
        // as it did before this phase existed — it can never land on an
        // unrelated row, and the offset can never be left lent out.
        if let Some(selected_index) = self.search_bar_state.as_ref(ctx).selected_index()
            && selected_index as isize != self.initial_selection_offset
        {
            self.restore_selection_offset = true;
            self.apply_selection_offset(selected_index as isize, ctx);
        }

        self.search_bar.update(ctx, |search_bar, ctx| {
            search_bar.run_query(ctx);
        });
    }

    /// Whether the transcript-content search is still running.
    ///
    /// While it is, the palette must not claim there is nothing to find.
    fn is_content_search_running(&self, app: &AppContext) -> bool {
        matches!(self.navigation_mode, NavigationMode::SessionSearch)
            && app.has_singleton_model::<TranscriptDigestModel>()
            && matches!(
                TranscriptDigestModel::as_ref(app).status(),
                DigestStatus::Searching
            )
    }

    /// The session-search popup's footer: what the content search is doing, and
    /// then what it can see.
    fn render_session_search_footer(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        if !matches!(self.navigation_mode, NavigationMode::SessionSearch) {
            return None;
        }
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let progress = app.has_singleton_model::<TranscriptDigestModel>().then(|| {
            let digest = TranscriptDigestModel::as_ref(app);
            (digest.status(), digest.progress())
        });
        let label = match progress {
            // `scanned` steps straight from 0 to the total: this phase publishes
            // once, at completion, and throttled progress milestones are a
            // deliberate later phase.
            Some((DigestStatus::Searching, (scanned, total))) => {
                format!("searching… {scanned}/{total}")
            }
            Some((DigestStatus::Idle | DigestStatus::Finished, _)) | None => {
                CONTENT_SEARCH_SCOPE_LABEL.to_owned()
            }
        };

        Some(
            Container::new(
                Text::new_inline(
                    label,
                    appearance.ui_font_family(),
                    appearance.monospace_font_size() - 2.,
                )
                .with_color(blended_colors::text_sub(theme, theme.surface_2()))
                .finish(),
            )
            .with_padding_top(6.)
            .with_horizontal_padding(styles::RESULT_PADDING_HORIZONTAL)
            .finish(),
        )
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
            // "No results found" is a claim, and while the content search is
            // still reading transcripts the palette is not entitled to make it:
            // rows may be about to appear underneath. Say nothing instead — the
            // footer is already counting.
            Some(renderers) if renderers.is_empty() && self.is_content_search_running(app) => {
                Empty::new().finish()
            }
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
        //
        // Agent sessions are excluded for the same reason and one more: their
        // data source only exists while the session-search popup is open, so
        // `query_result_from_summary` cannot rebuild one — a remembered session
        // would occupy a recent slot and then render as nothing at all.
        if !matches!(
            result_action,
            CommandPaletteItemAction::NavigateToTab { .. }
                | CommandPaletteItemAction::ResumeAgentSession { .. }
        ) {
            let selected_items_handle = SelectedItems::handle(ctx);
            selected_items_handle.update(ctx, |selected_items, _ctx| {
                selected_items.enqueue(result_action.to_summary())
            });
        }

        if let CommandPaletteItemAction::AcceptBinding { binding } = &result_action
            && let Some(action) = &binding.action
        {
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
                Some(WorkspaceAction::TogglePalette {
                    mode: mode @ PaletteMode::SessionSearch,
                    source,
                }) => {
                    // Not a filter switch like the arms above: session search
                    // lives in its own palette instance, and this one has no
                    // data source for it.
                    //
                    // The open is deferred so it lands *after* this palette's
                    // close. Dispatching it inline would run the workspace's
                    // `open_palette` first and its `close_palette` second,
                    // and the close clears the flag the open just set — the
                    // popup would never appear.
                    let action = WorkspaceAction::TogglePalette {
                        mode: *mode,
                        source: *source,
                    };
                    self.close(ctx, Some(result_action.result_type()));
                    ctx.dispatch_typed_action_deferred(action);
                    return;
                }
                _ => {}
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
                    initial_attachments: vec![],
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

                if let Err(e) = std::fs::File::create_new(&file_path)
                    && e.kind() != std::io::ErrorKind::AlreadyExists
                {
                    log::warn!("Failed to create file {}: {e}", file_path.display());
                    return;
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
            CommandPaletteItemAction::ResumeAgentSession { agent, session_id } => {
                // The workspace owns the activate-existing-tab vs. new-tab
                // decision (and the feature-flag gate), so this dispatches the
                // same action the project rail's rows do rather than deciding
                // anything here.
                ctx.dispatch_typed_action(&WorkspaceAction::ResumeDormantAgentTask {
                    agent,
                    session_id,
                });
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
