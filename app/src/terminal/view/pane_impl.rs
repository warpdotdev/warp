//! This module contains the implementation of `BackingView` for `TerminalView`, as well as
//! business logic for integrating the terminal view with the pane infra (`crate::pane_group`).
use super::ambient_agent::is_cloud_agent_pre_first_exchange;
use super::shared_session::adapter::Kind as SharedSessionKind;
use super::{Event, PaneConfiguration, TerminalAction, TerminalViewState, Viewer};
use crate::ai::agent::conversation::{
    AIConversation, ConversationStatus, ServerAIConversationMetadata,
};
use crate::ai::blocklist::agent_view::agent_view_bg_fill;
use crate::ai::blocklist::agent_view::orchestration_conversation_links::parent_conversation_navigation_card;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::appearance::Appearance;
use crate::drive::sharing::ShareableObject;
use crate::features::FeatureFlag;
use crate::menu::{MenuItem, MenuItemFields};
use crate::pane_group::focus_state::{PaneFocusHandle, PaneGroupFocusEvent, PaneGroupFocusState};
use crate::pane_group::pane::view::header::components::{
    header_edge_min_width, render_pane_header_buttons, render_pane_header_title_text,
    render_three_column_header, CenteredHeaderEdgeWidth,
};
use crate::pane_group::pane::view::header::{render_pane_header_draggable, PANE_HEADER_HEIGHT};
use crate::pane_group::pane::PaneStack;
use crate::pane_group::{pane::view, pane::view::PaneHeaderAction, BackingView, SplitPaneState};
use crate::settings::app_installation_detection::{
    UserAppInstallDetectionSettings, UserAppInstallStatus,
};
use crate::tab::{SelectedTabColor, TAB_COLOR_ICON_PATH, TAB_NO_COLOR_ICON_PATH};
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::shared_session::participant_avatar_view::render_participants_and_role_elements;
use crate::terminal::shared_session::render_util::shared_session_indicator_color;
use crate::terminal::shared_session::SharedSessionActionSource;
use crate::terminal::TerminalManager;
use crate::terminal::TerminalView;
use crate::ui_components::agent_icon::terminal_view_agent_icon_variant;
use crate::ui_components::blended_colors;
use crate::ui_components::buttons::icon_button_with_color;
use crate::ui_components::color_dot::TAB_COLOR_OPTIONS;
use crate::ui_components::icon_with_status::render_icon_with_status;
use crate::ui_components::icons;
use crate::workspace::tab_settings::TabSettings;
use settings::Setting as _;
use warp_core::context_flag::ContextFlag;
use warpui::elements::{
    ConstrainedBox, CrossAxisAlignment, Empty, Flex, MainAxisAlignment, MainAxisSize,
    ParentElement, Shrinkable,
};
use warpui::prelude::{ChildView, Container};
use warpui::text_layout::ClipConfig;
use warpui::ui_components::components::UiComponent;
#[cfg(not(target_arch = "wasm32"))]
use warpui::ui_components::components::UiComponentStyles;
use warpui::WeakModelHandle;
use warpui::{AppContext, Element, ModelHandle, SingletonEntity, TypedActionView, ViewContext};

/// Total size of the agent icon-with-status component rendered in the pane header.
/// Sub-components (circle, badge, cloud) are derived inside `render_icon_with_status`.
/// Sized so the component fits comfortably within `PANE_HEADER_HEIGHT` (34px) with a
/// few pixels of vertical buffer.
const PANE_HEADER_AGENT_SIZE: f32 = 26.;

impl TerminalView {
    /// Returns a reference to the focus handle if one has been set.
    pub fn focus_handle(&self) -> Option<&PaneFocusHandle> {
        self.focus_handle.as_ref()
    }

    fn handle_focus_state_event(
        &mut self,
        _focus_state: ModelHandle<PaneGroupFocusState>,
        event: &PaneGroupFocusEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(focus_handle) = &self.focus_handle else {
            return;
        };

        if focus_handle.is_affected(event) {
            self.on_pane_state_change(ctx);
        }
    }

    /// Set the pane configuration for this terminal view.
    pub fn set_pane_configuration(&mut self, pane_configuration: ModelHandle<PaneConfiguration>) {
        self.pane_configuration = pane_configuration;
    }

    /// Respond to changes to the active session or split pane states.
    pub fn on_pane_state_change(&mut self, ctx: &mut ViewContext<Self>) {
        self.refresh_pane_header(ctx);

        // Trigger refresh of the pane header overflow menu to reflect the new pane state
        // (e.g., updating the Maximize/Minimize pane menu item)
        self.pane_configuration.update(ctx, |config, ctx| {
            config.refresh_pane_header_overflow_menu_items(ctx);
        });

        if !self.is_pane_focused(ctx) {
            // Don't need to call ctx.notify here as clear_selected_blocks already
            // calls ctx.notify internally
            self.clear_selected_blocks(ctx);
            self.clear_selected_text(ctx);
        } else {
            ctx.notify();
        }
    }

    pub fn refresh_pane_header(&mut self, ctx: &mut ViewContext<Self>) {
        let is_active_session = self.is_active_session(ctx);
        self.pane_configuration
            .update(ctx, move |pane_config, ctx| {
                pane_config.set_show_active_pane_indicator(is_active_session, ctx);
                pane_config.refresh_pane_header_overflow_menu_items(ctx);
            });
    }

    /// Set the pane title from agent chrome when available, falling back to the regular terminal title.
    pub(super) fn update_pane_configuration(&mut self, ctx: &mut ViewContext<Self>) {
        let is_ambient_agent = self.is_ambient_agent_session(ctx);
        let selected_conversation_title = self.selected_conversation_display_title(ctx);
        let selected_cli_agent_title = self.selected_cli_agent_title_for_chrome(ctx);

        // Prefer CLI agent session text before the terminal title,
        // matching the vertical-tab behavior in terminal_primary_line_data().
        let new_pane_title = if let Some(cli_agent_title) = selected_cli_agent_title {
            self.is_using_conversation_for_pane_header_title = false;
            cli_agent_title
        } else if self.is_long_running_and_user_controlled() && !self.terminal_title.is_empty() {
            self.is_using_conversation_for_pane_header_title = false;
            self.terminal_title.clone()
        } else {
            match selected_conversation_title {
                Some(conversation_title) => {
                    self.is_using_conversation_for_pane_header_title = true;
                    conversation_title
                }
                None => {
                    if is_ambient_agent {
                        default_agent_conversation_title(is_ambient_agent)
                    } else {
                        self.terminal_title.clone()
                    }
                }
            }
        };
        self.pane_configuration.update(ctx, |pane_config, ctx| {
            pane_config.set_title(new_pane_title, ctx);
            if FeatureFlag::AgentView.is_enabled() {
                pane_config.refresh_pane_header_overflow_menu_items(ctx);
            }
            pane_config.notify_header_content_changed(ctx);
        });
        self.update_agent_view_pane_header(ctx);
    }

    /// Returns the shareable object for the active agent view conversation, if any.
    fn agent_view_shareable_object(&self, ctx: &ViewContext<Self>) -> Option<ShareableObject> {
        // Only set shareable object if CloudConversations feature is enabled
        if !FeatureFlag::CloudConversations.is_enabled() {
            return None;
        }

        // If we're in a shared session, prioritize this to share.
        if let Some(shared_session) = &self.shared_session {
            return Some(ShareableObject::Session {
                handle: ctx.handle(),
                session_id: *shared_session.session_id(),
                started_at: *shared_session.started_at(),
            });
        }

        // Check if agent view is active
        let conversation_id = self
            .agent_view_controller
            .as_ref(ctx)
            .agent_view_state()
            .active_conversation_id()?;

        // Don't show share button for empty conversations
        let conversation = BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id)?;
        if conversation.is_empty() {
            return None;
        }
        let exchange_count = conversation.exchange_count();
        // If there's only one exchange, make sure it's completed (not still streaming)
        if exchange_count == 1 {
            if let Some(latest_exchange) = conversation.latest_exchange() {
                if latest_exchange.output_status.is_streaming() {
                    return None;
                }
            }
        }

        // Return the ShareableObject with the conversation ID
        Some(ShareableObject::AIConversation(conversation_id))
    }

    /// Updates the pane header's shareable object based on agent view state.
    /// This should be called when entering/exiting agent view or when the conversation changes.
    pub(super) fn update_agent_view_pane_header(&mut self, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::AgentView.is_enabled() {
            return;
        }

        // In cloud mode, we want to preserve the shared session sharing dialog even after the shared session has ended.
        // We need this to be able to view and change permissions on a cloud mode shared session that failed before
        // any conversation started, to view cloud mode sessions that failed during setup.
        let is_ambient_agent = self.is_ambient_agent_session(ctx);
        if !is_ambient_agent {
            let shareable_object = self.agent_view_shareable_object(ctx);
            self.pane_configuration.update(ctx, |pane_config, ctx| {
                pane_config.set_shareable_object(shareable_object, ctx);
                pane_config.notify_header_content_changed(ctx);
                pane_config.refresh_pane_header_overflow_menu_items(ctx);
            });
        } else {
            self.pane_configuration.update(ctx, |pane_config, ctx| {
                pane_config.notify_header_content_changed(ctx);
                pane_config.refresh_pane_header_overflow_menu_items(ctx);
            });
        }
    }

    pub(super) fn is_pane_focused(&self, app: &AppContext) -> bool {
        self.focus_handle.as_ref().is_none_or(|h| h.is_focused(app))
    }

    pub fn is_active_session(&self, app: &AppContext) -> bool {
        self.focus_handle
            .as_ref()
            .is_some_and(|h| h.is_active_session(app))
    }

    pub(super) fn split_pane_state(&self, app: &AppContext) -> SplitPaneState {
        self.focus_handle
            .as_ref()
            .map_or(SplitPaneState::NotInSplitPane, |h| h.split_pane_state(app))
    }

    /// Renders the back button for the pane header, or an empty element if the
    /// back button should not be shown.
    fn maybe_render_header_back_button(&self, app: &AppContext) -> Box<dyn Element> {
        if !FeatureFlag::AgentView.is_enabled() || warpui::platform::is_mobile_device() {
            return Flex::row().finish();
        }

        let in_nav_stack = self
            .pane_stack
            .as_ref()
            .and_then(|h| h.upgrade(app))
            .is_some_and(|stack| stack.as_ref(app).depth() > 1);

        let is_transcript_viewer = self.model.lock().is_conversation_transcript_viewer();
        let is_ambient_agent = self.is_ambient_agent_session(app);
        let has_parent_terminal = (is_ambient_agent && self.is_nested_cloud_mode(app))
            || (!is_ambient_agent && !is_transcript_viewer);
        let is_fullscreen_agent_view = self.agent_view_controller.as_ref(app).is_fullscreen();

        if in_nav_stack || (is_fullscreen_agent_view && has_parent_terminal) {
            if FeatureFlag::OrchestrationV2.is_enabled() {
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(ChildView::new(&self.agent_view_back_button).finish())
                    .finish()
            } else {
                Flex::column()
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_cross_axis_alignment(CrossAxisAlignment::Start)
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_child(ChildView::new(&self.agent_view_back_button).finish())
                    .finish()
            }
        } else {
            Flex::row().finish()
        }
    }

    fn render_header_title(
        &self,
        is_fullscreen_agent_view: bool,
        header_ctx: &view::HeaderRenderContext,
        app: &AppContext,
    ) -> Box<dyn Element> {
        // V2 swap-panes semantics: every conversation in the orchestration
        // tree (orchestrator + each child) gets the orchestration pill bar
        // rendered above the agent view header, so the pane title here
        // falls back to the regular conversation title. Breadcrumbs used
        // to render here for split-off child views, but the swap-panes
        // refactor removed the split-off code path — the pill bar is now
        // shown on every view, so a breadcrumb row alongside it would
        // double-render the same navigation affordance.

        let appearance = Appearance::as_ref(app);
        let pane_config = self.pane_configuration.as_ref(app);
        let title = pane_config.title().to_owned();
        let clip_config = if self.is_using_conversation_for_pane_header_title {
            ClipConfig::ellipsis()
        } else {
            ClipConfig::start()
        };

        let should_render_ambient_agent_indicator =
            self.ambient_agent_task_id_for_details_panel(app).is_some()
                || self.model.lock().is_shared_ambient_agent_session();
        let theme = appearance.theme();
        let render_agent_circle = |variant| {
            render_icon_with_status(
                variant,
                PANE_HEADER_AGENT_SIZE,
                0.,
                theme,
                theme.background(),
            )
        };
        let pane_indicator = if should_render_ambient_agent_indicator {
            // Shared/viewed ambient session: route through the shared helper so the pane header
            // renders the same brand-color circle + cloud lobe + status as the vertical tab.
            terminal_view_agent_icon_variant(self, app).map(render_agent_circle)
        } else if let Some(shared_session) = self.shared_session.as_ref() {
            if let Some(Viewer {
                sharer: Some(sharer),
                ..
            }) = shared_session.kind().as_viewer()
            {
                Some(
                    Container::new(ChildView::new(&sharer.avatar).finish())
                        .with_margin_right(4.)
                        .finish(),
                )
            } else {
                Some(
                    ConstrainedBox::new(
                        icons::Icon::Sharing
                            .to_warpui_icon(shared_session_indicator_color(appearance).into())
                            .finish(),
                    )
                    .with_height(appearance.ui_font_size())
                    .with_width(appearance.ui_font_size())
                    .finish(),
                )
            }
        } else if self.is_using_conversation_for_pane_header_title
            || (self.is_long_running()
                && self
                    .ai_context_model
                    .as_ref(app)
                    .selected_conversation(app)
                    .is_some())
        {
            // Conversation-bound terminal: same shared helper — produces an OzAgent variant for
            // local conversations and a CLIAgent variant for the (rare) CLI-backed terminal.
            terminal_view_agent_icon_variant(self, app).map(render_agent_circle)
        } else {
            self.render_terminal_mode_indicator(app)
        };

        let is_pane_dragging = header_ctx.draggable_state.is_dragging();
        let mut center_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min);
        if let Some(indicator) = pane_indicator {
            center_row.add_child(Container::new(indicator).with_margin_right(4.).finish());
        }
        let title_text = render_pane_header_title_text(title, appearance, clip_config);
        if is_pane_dragging {
            // During drag, all children must be non-flex to avoid panics
            // from infinite constraints on flex children.
            center_row.add_child(title_text);
        } else {
            let title_element =
                if is_fullscreen_agent_view && self.is_using_conversation_for_pane_header_title {
                    Shrinkable::new(
                        1.0,
                        ConstrainedBox::new(title_text)
                            .with_max_width(400.0)
                            .finish(),
                    )
                    .finish()
                } else {
                    Shrinkable::new(1.0, title_text).finish()
                };
            center_row.add_child(title_element);
        }

        center_row.finish()
    }

    /// Returns the right-column element and the estimated minimum width of
    /// the right-column content (used to set the edge width for centering).
    fn render_header_actions(
        &self,
        header_ctx: &view::HeaderRenderContext,
        app: &AppContext,
    ) -> (Box<dyn Element>, f32) {
        let appearance = Appearance::as_ref(app);
        let is_fullscreen_agent_view = FeatureFlag::AgentView.is_enabled()
            && self.agent_view_controller.as_ref(app).is_fullscreen();
        let icon_color = Some(
            appearance
                .theme()
                .sub_text_color(appearance.theme().background()),
        );
        let button_size = if is_fullscreen_agent_view {
            Some(24.0)
        } else {
            None
        };

        let mut left_of_overflow = self.render_shared_session_header_content(app);

        let mut icon_button_count: u32 = 0;

        // Cloud-mode-only ambient agent cancel button is shown while we're waiting
        // for the session to be ready.
        let is_waiting_for_session = FeatureFlag::CloudMode.is_enabled()
            && self
                .ambient_agent_view_model
                .as_ref()
                .is_some_and(|model| model.as_ref(app).is_waiting_for_session());
        let button_element = if is_waiting_for_session {
            Some(self.render_ambient_agent_cancel_button(app))
        } else if self.can_show_conversation_details_ui(app) {
            #[cfg(not(target_arch = "wasm32"))]
            {
                Some(self.render_conversation_details_toggle_button(app))
            }
            #[cfg(target_arch = "wasm32")]
            {
                None
            }
        } else {
            None
        };

        if let Some(button) = button_element {
            icon_button_count += 1;
            if let Some(existing) = left_of_overflow {
                left_of_overflow =
                    Some(Flex::row().with_child(existing).with_child(button).finish());
            } else {
                left_of_overflow = Some(button);
            }
        }

        let mut right_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min);
        if let Some(content) = left_of_overflow {
            right_row.add_child(content);
        }
        let sharing_element = header_ctx.sharing_controls(app, icon_color, button_size);
        let has_sharing_element = sharing_element.is_some();
        if let Some(sharing) = sharing_element {
            right_row.add_child(sharing);
        }
        let show_close_button = self
            .focus_handle
            .as_ref()
            .is_some_and(|h| h.is_in_split_pane(app));
        right_row.add_child(
            render_pane_header_buttons::<TerminalAction, TerminalAction>(
                header_ctx,
                appearance,
                show_close_button,
                icon_color,
                button_size,
            ),
        );
        icon_button_count += show_close_button as u32
            + header_ctx.has_overflow_items as u32
            + has_sharing_element as u32;

        let min_width = header_edge_min_width(icon_button_count);
        (right_row.finish(), min_width)
    }

    fn render_parent_conversation_header_card(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        if !(FeatureFlag::OrchestrationV2.is_enabled()
            && FeatureFlag::AgentView.is_enabled()
            && self.agent_view_controller.as_ref(app).is_fullscreen())
        {
            return None;
        }

        let active_conversation_id = self
            .agent_view_controller
            .as_ref(app)
            .agent_view_state()
            .active_conversation_id()?;
        let active_conversation =
            BlocklistAIHistoryModel::as_ref(app).conversation(&active_conversation_id)?;
        parent_conversation_navigation_card(
            active_conversation,
            self.mouse_states.parent_conversation_header_link.clone(),
            app,
        )
    }

    fn maybe_add_parent_navigation_card(
        &self,
        header: Box<dyn Element>,
        parent_conversation_header_card: Option<Box<dyn Element>>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        // When `OrchestrationPillBar` is on, the pill bar takes the place of the
        // parent navigation card (the parent pill is the "back to parent" link)
        // and is shown for the orchestrator and the swap-target child panes.
        // Split-off panes ("Open in new pane" / "Open in new tab") instead
        // render a parent→child breadcrumb row so the user has a clear way
        // back to the orchestrator without rendering the full sibling pill
        // list a second time alongside the orchestrator's own pill bar.
        //
        // `OrchestrationViewerPillBar` is the parallel flag for shared
        // session viewers (web + native). Children are registered via the
        // REST data fetch in `OrchestrationViewerModel`; when none have
        // arrived yet, `OrchestrationPillBar::pill_specs` returns `None`
        // and the pill bar's `render` short-circuits to `Empty`, so the
        // gate here is intentionally permissive.
        if (FeatureFlag::OrchestrationPillBar.is_enabled()
            || FeatureFlag::OrchestrationViewerPillBar.is_enabled())
            && FeatureFlag::AgentView.is_enabled()
            && self.agent_view_controller.as_ref(app).is_fullscreen()
        {
            // The wrapping `Flex::column` would otherwise pass an infinite
            // vertical max constraint down to its non-flex children. That
            // breaks the title's vertical centering: with infinite max.y,
            // the centered `Align` inside `render_three_column_header`
            // collapses to the title's own (small) line-box height, and
            // the outer row's `CrossAxisAlignment::Stretch` then pins the
            // title to the top of the row. Pinning the header to its
            // standard `PANE_HEADER_HEIGHT` here restores the finite
            // vertical constraint the centering logic relies on, while
            // letting the pill bar / breadcrumb row sit immediately below
            // at its own height.
            let pinned_header = ConstrainedBox::new(header)
                .with_height(PANE_HEADER_HEIGHT)
                .finish();
            let secondary_row: Box<dyn Element> = if self.is_orchestration_split_off() {
                crate::ai::blocklist::agent_view::render_orchestration_breadcrumbs(
                    self.agent_view_controller.as_ref(app),
                    self.mouse_states.parent_conversation_header_link.clone(),
                    self.mouse_states.breadcrumbs_horizontal_scroll.clone(),
                    app,
                )
                .unwrap_or_else(|| Empty::new().finish())
            } else {
                ChildView::new(&self.orchestration_pill_bar).finish()
            };
            return Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(pinned_header)
                .with_child(secondary_row)
                .finish();
        }

        if !FeatureFlag::OrchestrationV2.is_enabled() {
            return header;
        }

        if let Some(parent_card) = parent_conversation_header_card {
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(
                    Container::new(parent_card)
                        .with_padding_left(4.)
                        .with_padding_right(4.)
                        .with_padding_top(4.)
                        .with_padding_bottom(2.)
                        .finish(),
                )
                .with_child(header)
                .finish()
        } else {
            header
        }
    }

    fn render_terminal_pane_header(
        &self,
        header_ctx: &view::HeaderRenderContext,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let is_fullscreen_agent_view = FeatureFlag::AgentView.is_enabled()
            && self.agent_view_controller.as_ref(app).is_fullscreen();
        let parent_conversation_header_card = self.render_parent_conversation_header_card(app);

        let left = self.maybe_render_header_back_button(app);
        let center = self.render_header_title(is_fullscreen_agent_view, header_ctx, app);
        let (right, min_actions_width) = self.render_header_actions(header_ctx, app);

        let header = render_three_column_header(
            left,
            center,
            right,
            CenteredHeaderEdgeWidth {
                min: min_actions_width,
                max: 200.0,
            },
            header_ctx.header_left_inset,
            header_ctx.draggable_state.is_dragging(),
        );
        // Make only the title row draggable; the secondary row (pill
        // bar / breadcrumbs / navigation card) sits outside the drag
        // region so its own mouse-driven widgets (notably the pill
        // bar's scrollbar thumb) keep their hit-targets.
        let draggable_header = render_pane_header_draggable::<TerminalView>(
            self.pane_configuration.clone(),
            header,
            header_ctx.draggable_state.clone(),
            app,
        );
        let header = self.maybe_add_parent_navigation_card(
            draggable_header,
            parent_conversation_header_card,
            app,
        );

        if is_fullscreen_agent_view {
            Container::new(header)
                .with_background(agent_view_bg_fill(app))
                .finish()
        } else {
            header
        }
    }
}

impl BackingView for TerminalView {
    type PaneHeaderOverflowMenuAction = TerminalAction;
    type CustomAction = TerminalAction;
    type AssociatedData = ModelHandle<Box<dyn TerminalManager>>;

    fn set_pane_stack(
        &mut self,
        pane_stack: WeakModelHandle<PaneStack<Self>>,
        _ctx: &mut ViewContext<Self>,
    ) {
        self.pane_stack = Some(pane_stack);
    }

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        action: &Self::PaneHeaderOverflowMenuAction,
        ctx: &mut ViewContext<Self>,
    ) {
        self.handle_action(action, ctx);
    }

    fn handle_custom_action(&mut self, action: &Self::CustomAction, ctx: &mut ViewContext<Self>) {
        self.handle_action(action, ctx);
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(Event::CloseRequested);
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        self.redetermine_global_focus(ctx);
    }

    fn on_pane_header_overflow_menu_toggled(&mut self, is_open: bool, ctx: &mut ViewContext<Self>) {
        self.pane_header_overflow_menu_toggled(is_open, ctx);
    }

    fn pane_header_overflow_menu_items(
        &self,
        ctx: &AppContext,
    ) -> Vec<MenuItem<Self::PaneHeaderOverflowMenuAction>> {
        let model = self.model.lock();
        let appearance = Appearance::as_ref(ctx);
        let terminal_colors = appearance.theme().terminal_colors().normal;
        let rename_tab_fallback_title = self
            .pane_configuration
            .as_ref(ctx)
            .title()
            .trim()
            .to_owned();
        let mut items = vec![];
        if let Some(focus_handle) = self.focus_handle() {
            items.push(
                MenuItemFields::new("Rename pane")
                    .with_on_select_action(TerminalAction::RenamePane(focus_handle.pane_id()))
                    .into_item(),
            );
        }
        items.extend([
            MenuItemFields::new("Rename tab")
                .with_on_select_action(TerminalAction::RenameActiveTabWithFallbackTitle(
                    rename_tab_fallback_title,
                ))
                .into_item(),
            MenuItem::ItemsRow {
                items: std::iter::once(
                    MenuItemFields::new_with_icon(
                        TAB_NO_COLOR_ICON_PATH,
                        appearance.theme().foreground(),
                        "None".to_owned(),
                    )
                    .no_highlight_on_hover()
                    .with_on_select_action(TerminalAction::SetActiveTabColor(
                        SelectedTabColor::Cleared,
                    )),
                )
                .chain(TAB_COLOR_OPTIONS.iter().map(|color_option| {
                    let color = color_option.to_ansi_color(&terminal_colors);
                    MenuItemFields::new_with_icon(
                        TAB_COLOR_ICON_PATH,
                        color.into(),
                        color_option.to_string(),
                    )
                    .no_highlight_on_hover()
                    .with_on_select_action(TerminalAction::SetActiveTabColor(
                        SelectedTabColor::Color(*color_option),
                    ))
                }))
                .collect(),
            },
        ]);
        let source = SharedSessionActionSource::PaneHeader;

        // Shared-session related items.
        let mut shared_session_items = vec![];
        let shared_session_status = model.shared_session_status();
        let is_ambient_agent = self.is_ambient_agent_session(ctx);
        if shared_session_status.is_sharer_or_viewer() {
            if !is_ambient_agent {
                shared_session_items.push(
                    MenuItemFields::new("Copy link")
                        .with_on_select_action(TerminalAction::CopySharedSessionLink { source })
                        .into_item(),
                );
            }

            if shared_session_status.is_sharer() {
                shared_session_items.push(
                    MenuItemFields::new("Stop sharing session")
                        .with_on_select_action(TerminalAction::StopSharingCurrentSession { source })
                        .into_item(),
                );
            }
            if !ContextFlag::HideOpenOnDesktopButton.is_enabled()
                && *UserAppInstallDetectionSettings::as_ref(ctx)
                    .user_app_installation_detected
                    .value()
                    == UserAppInstallStatus::Detected
            {
                shared_session_items.push(
                    MenuItemFields::new("Open on Desktop")
                        .with_on_select_action(TerminalAction::OpenSharedSessionOnDesktop {
                            source,
                        })
                        .into_item(),
                );
            }
        } else if FeatureFlag::CreatingSharedSessions.is_enabled()
            && ContextFlag::CreateSharedSession.is_enabled()
        {
            shared_session_items.push(
                MenuItemFields::new("Share session")
                    .with_on_select_action(TerminalAction::OpenShareSessionModal { source })
                    .into_item(),
            );
        }
        if !shared_session_items.is_empty() {
            items.push(MenuItem::Separator);
            items.extend(shared_session_items);
        }

        // Split-pane related items.
        if self.split_pane_state(ctx).is_in_split_pane() {
            if !items.is_empty() {
                items.push(MenuItem::Separator);
            }

            let is_maximized = self.split_pane_state(ctx).is_maximized();
            items.push(
                MenuItemFields::toggle_pane_action(is_maximized)
                    .with_on_select_action(TerminalAction::ToggleMaximizePane)
                    .into_item(),
            );
        }

        items
    }

    fn should_render_header(&self, app: &AppContext) -> bool {
        let is_shared = self
            .model
            .lock()
            .shared_session_status()
            .is_sharer_or_viewer();
        let is_fullscreen_agent_view = FeatureFlag::AgentView.is_enabled()
            && self.agent_view_controller.as_ref(app).is_fullscreen();
        is_shared
            || is_fullscreen_agent_view
            || FeatureFlag::ContextWindowUsageV2.is_enabled()
                && self.split_pane_state(app).is_in_split_pane()
    }

    fn render_header_content(
        &self,
        header_ctx: &view::HeaderRenderContext<'_>,
        app: &AppContext,
    ) -> view::HeaderContent {
        view::HeaderContent::Custom {
            element: self.render_terminal_pane_header(header_ctx, app),
            // We wrap only the title row in the drag handler ourselves;
            // the secondary row stays interactive.
            has_custom_draggable_behavior: true,
        }
    }

    /// Sets the focus handle for this terminal view, enabling it to track its split pane state.
    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle.clone());
        // Subscribe to focus state changes to update pane state when focus/split state changes
        ctx.subscribe_to_model(
            focus_handle.focus_state_handle(),
            Self::handle_focus_state_event,
        );
        self.input.update(ctx, |input, ctx| {
            input.set_focus_handle(focus_handle, ctx);
        });
        self.on_pane_state_change(ctx);
    }
}

impl TerminalView {
    /// Render the cancel button for cancelling the ambient agent task while it's loading.
    fn render_ambient_agent_cancel_button(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let ui_builder = appearance.ui_builder().clone();

        icon_button_with_color(
            appearance,
            icons::Icon::StopFilled,
            false, /* active */
            self.ambient_agent_cancel_mouse_state.clone(),
            blended_colors::text_sub(theme, theme.background()).into(),
        )
        .with_tooltip(move || ui_builder.tool_tip("Cancel".to_string()).build().finish())
        .build()
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action::<PaneHeaderAction<TerminalAction, TerminalAction>>(
                PaneHeaderAction::CustomAction(TerminalAction::CancelAmbientAgentTask),
            );
        })
        .finish()
    }

    /// Render the info button for toggling the conversation details panel.
    /// Only available on non-WASM platforms (WASM uses a per-window button instead).
    #[cfg(not(target_arch = "wasm32"))]
    fn render_conversation_details_toggle_button(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let is_open = self.is_conversation_details_panel_open;
        let ui_builder = appearance.ui_builder().clone();

        // Use main text color when panel is open (hover-like appearance), sub color when closed
        let icon_color = if is_open {
            blended_colors::text_main(theme, theme.background()).into()
        } else {
            blended_colors::text_sub(theme, theme.background()).into()
        };

        let button = icon_button_with_color(
            appearance,
            icons::Icon::Info,
            is_open, // show active background when panel is open
            self.conversation_details_panel_toggle_mouse_state.clone(),
            icon_color,
        );

        // Add explicit background when panel is open
        let button = if is_open {
            button.with_style(UiComponentStyles::default().set_background(theme.surface_2().into()))
        } else {
            button
        };

        button
            .with_tooltip(move || {
                let tooltip_text = if is_open {
                    "Hide details"
                } else {
                    "Show details"
                };
                ui_builder
                    .tool_tip(tooltip_text.to_string())
                    .build()
                    .finish()
            })
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action::<PaneHeaderAction<TerminalAction, TerminalAction>>(
                    PaneHeaderAction::CustomAction(TerminalAction::ToggleConversationDetailsPanel),
                );
            })
            .finish()
    }

    /// Render the indicator for terminal mode (no conversation selected).
    /// Shows error indicator if terminal is in error state, otherwise shell indicator on Windows.
    fn render_terminal_mode_indicator(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        let appearance = Appearance::as_ref(app);
        let font_size = appearance.ui_font_size();

        // Error indicator takes priority
        if matches!(self.current_state.state, TerminalViewState::Errored) {
            return Some(
                ConstrainedBox::new(
                    icons::Icon::AlertTriangle
                        .to_warpui_icon(appearance.theme().ui_error_color().into())
                        .finish(),
                )
                .with_height(font_size)
                .with_width(font_size)
                .finish(),
            );
        }

        // Shell indicator (Windows only)
        if let Some(shell_indicator_type) = self.shell_indicator_type {
            let shell_indicator_icon = shell_indicator_type
                .to_icon()
                .to_warpui_icon(
                    blended_colors::text_sub(appearance.theme(), appearance.theme().background())
                        .into(),
                )
                .finish();
            return Some(
                ConstrainedBox::new(shell_indicator_icon)
                    .with_height(font_size)
                    .with_width(font_size)
                    .finish(),
            );
        }

        None
    }

    /// Render shared session header content (participant avatars and role controls).
    fn render_shared_session_header_content(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        let Some(shared_session) = &self.shared_session else {
            return None;
        };

        let presence_manager = shared_session.presence_manager();
        let role = presence_manager.as_ref(app).role();

        // Get viewer avatars to render
        let viewers = shared_session.pane_header_viewer_avatars(app);

        // Get role change menu info based on session kind
        let (role_change_menu, is_role_change_menu_open, mouse_state_handle) =
            match shared_session.kind() {
                SharedSessionKind::Viewer(viewer) => (
                    Some(viewer.role_change_menu.clone()),
                    viewer.is_role_change_menu_open,
                    viewer.role_change_menu_button.clone(),
                ),
                SharedSessionKind::Sharer(sharer) => {
                    (None, false, sharer.revoke_all_mouse_state_handle().clone())
                }
            };

        // Hide role change button in cloud mode conversations
        let hide_role_change_button = self.model.lock().is_shared_ambient_agent_session();

        // Render participant avatars and role elements
        Some(render_participants_and_role_elements(
            viewers,
            role,
            mouse_state_handle,
            role_change_menu,
            is_role_change_menu_open,
            hide_role_change_button,
            app,
        ))
    }

    pub fn is_ambient_agent_session(&self, ctx: &AppContext) -> bool {
        FeatureFlag::CloudMode.is_enabled()
            && self
                .ambient_agent_view_model
                .as_ref()
                .is_some_and(|model| model.as_ref(ctx).is_ambient_agent())
    }

    fn selected_conversation_for_user_facing_chrome<'a>(
        &'a self,
        ctx: &'a AppContext,
    ) -> Option<&'a AIConversation> {
        self.ai_context_model
            .as_ref(ctx)
            .selected_conversation(ctx)
            .filter(|conversation| {
                !conversation.is_entirely_passive()
                    && (conversation.title().is_some_and(|title| !title.is_empty())
                        || FeatureFlag::AgentView.is_enabled())
            })
    }

    fn selected_conversation_display_title_for_chrome(
        &self,
        conversation: &AIConversation,
        is_ambient_agent: bool,
    ) -> String {
        if FeatureFlag::AgentView.is_enabled() {
            conversation
                .title()
                .filter(|title| !title.is_empty())
                .unwrap_or_else(|| default_agent_conversation_title(is_ambient_agent))
        } else {
            conversation
                .title()
                .expect("checked above that title exists")
        }
    }

    /// Returns `true` while a cloud-mode ambient agent run is still spinning up. This covers
    /// both the `WaitingForSession` phase (env being provisioned, "Connecting to Host") and
    /// the post-session pre-first-exchange phase (session ready, harness not started, no
    /// exchange yet). In either case the run is committed and we want the UI to read as busy.
    fn is_in_cloud_agent_setup_phase(&self, ctx: &AppContext) -> bool {
        if self
            .ambient_agent_view_model
            .as_ref()
            .is_some_and(|model| model.as_ref(ctx).is_waiting_for_session())
        {
            return true;
        }

        let model = self.model.lock();
        is_cloud_agent_pre_first_exchange(
            self.ambient_agent_view_model.as_ref(),
            &self.agent_view_controller,
            &model,
            ctx,
        )
    }

    /// Selected conversation status for chrome, or [`ConversationStatus::InProgress`] while the
    /// active block is long-running (terminal-derived; not mirrored in history events) or while
    /// a cloud-mode ambient agent is still in its environment-setup phase.
    pub fn selected_conversation_status(&self, ctx: &AppContext) -> Option<ConversationStatus> {
        let long_running = self.is_long_running();
        let cloud_setup = self.is_in_cloud_agent_setup_phase(ctx);

        let Some(conversation) = self.selected_conversation_for_user_facing_chrome(ctx) else {
            // Ambient agent tabs can show Oz chrome without a filtered "chrome" conversation;
            // still surface busy while a long-running shell command is active or the cloud
            // environment is spinning up.
            if (long_running || cloud_setup) && self.is_ambient_agent_session(ctx) {
                return Some(ConversationStatus::InProgress);
            }
            return None;
        };

        if long_running || cloud_setup {
            return Some(ConversationStatus::InProgress);
        }

        if self.selected_conversation_is_empty(ctx) {
            return None;
        }

        Some(conversation.status().clone())
    }

    pub fn selected_conversation_is_empty(&self, ctx: &AppContext) -> bool {
        self.selected_conversation_for_user_facing_chrome(ctx)
            .is_some_and(|conversation| conversation.is_empty())
    }

    /// Returns the conversation status for display purposes, suppressing the status when the
    /// conversation is empty (no exchanges yet) AND nothing else makes the run "busy". This
    /// avoids showing a misleading "In progress" indicator on a brand-new conversation; real
    /// InProgress states (long-running shell commands, cloud-environment setup) come through
    /// because [`Self::selected_conversation_status`] surfaces them as `InProgress`.
    pub fn selected_conversation_status_for_display(
        &self,
        ctx: &AppContext,
    ) -> Option<ConversationStatus> {
        let status = self.selected_conversation_status(ctx)?;
        if matches!(status, ConversationStatus::InProgress)
            || !self.selected_conversation_is_empty(ctx)
        {
            Some(status)
        } else {
            None
        }
    }

    pub fn selected_conversation_display_title(&self, ctx: &AppContext) -> Option<String> {
        let is_ambient_agent = self.is_ambient_agent_session(ctx);
        self.selected_conversation_for_user_facing_chrome(ctx)
            .map(|conversation| {
                self.selected_conversation_display_title_for_chrome(conversation, is_ambient_agent)
            })
    }

    /// Server metadata for the selected conversation, if any.
    pub fn selected_conversation_server_metadata<'a>(
        &'a self,
        ctx: &'a AppContext,
    ) -> Option<&'a ServerAIConversationMetadata> {
        self.selected_conversation_for_user_facing_chrome(ctx)
            .and_then(AIConversation::server_metadata)
    }

    pub fn selected_conversation_latest_user_prompt_for_tab_name(
        &self,
        ctx: &AppContext,
    ) -> Option<String> {
        self.selected_conversation_for_user_facing_chrome(ctx)
            .and_then(AIConversation::latest_user_query)
    }

    fn selected_cli_agent_title_for_chrome(&self, ctx: &AppContext) -> Option<String> {
        let session = CLIAgentSessionsModel::as_ref(ctx)
            .session(self.view_id)
            .filter(|session| session.listener.is_some())?;

        if *TabSettings::as_ref(ctx).use_latest_user_prompt_as_conversation_title_in_tab_names {
            session
                .session_context
                .latest_user_prompt()
                .or_else(|| session.session_context.title_like_text())
        } else {
            session.session_context.title_like_text()
        }
    }
}

fn default_agent_conversation_title(is_ambient_agent: bool) -> String {
    if is_ambient_agent {
        "New cloud agent".to_owned()
    } else {
        "New agent conversation".to_owned()
    }
}
