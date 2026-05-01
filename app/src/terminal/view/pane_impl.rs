//! This module contains the implementation of `BackingView` for `TerminalView`, as well as
//! business logic for integrating the terminal view with the pane infra (`crate::pane_group`).
use super::shared_session::adapter::Kind as SharedSessionKind;
use super::{Event, PaneConfiguration, TerminalAction, TerminalViewState, Viewer};
use crate::ai::agent::conversation::{AIConversation, ConversationStatus};
use crate::ai::blocklist::agent_view::agent_view_bg_fill;
use crate::ai::blocklist::agent_view::orchestration_conversation_links::parent_conversation_navigation_card;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::conversation_status_ui::{render_status_element, STATUS_ELEMENT_PADDING};
use crate::appearance::Appearance;
use crate::drive::sharing::ShareableObject;
use crate::features::FeatureFlag;
use crate::menu::{MenuItem, MenuItemFields};
use crate::pane_group::focus_state::{PaneFocusHandle, PaneGroupFocusEvent, PaneGroupFocusState};
use crate::pane_group::pane::view::header::components::{
    header_edge_min_width, render_pane_header_buttons, render_pane_header_title_text,
    render_three_column_header, CenteredHeaderEdgeWidth,
};
use crate::pane_group::pane::PaneStack;
use crate::pane_group::{pane::view, pane::view::PaneHeaderAction, BackingView, SplitPaneState};
use crate::settings::app_installation_detection::{
    UserAppInstallDetectionSettings, UserAppInstallStatus,
};
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::model::terminal_model::ConversationTranscriptViewerStatus;
use crate::terminal::shared_session::participant_avatar_view::render_participants_and_role_elements;
use crate::terminal::shared_session::render_util::shared_session_indicator_color;
use crate::terminal::shared_session::SharedSessionActionSource;
use crate::terminal::TerminalManager;
use crate::terminal::TerminalView;
use crate::ui_components::blended_colors;
use crate::ui_components::buttons::icon_button_with_color;
use crate::ui_components::icons;
use crate::workspace::tab_settings::TabSettings;
use settings::Setting as _;
use warp_core::context_flag::ContextFlag;
use warp_core::ui::Icon as WarpIcon;
use warpui::elements::{
    ChildAnchor, ConstrainedBox, CrossAxisAlignment, Flex, MainAxisAlignment, MainAxisSize,
    OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Shrinkable, Stack,
};
use warpui::prelude::{vec2f, ChildView, Container, Hoverable};
use warpui::text_layout::ClipConfig;
use warpui::ui_components::components::UiComponent;
#[cfg(not(target_arch = "wasm32"))]
use warpui::ui_components::components::UiComponentStyles;
use warpui::WeakModelHandle;
use warpui::{AppContext, Element, ModelHandle, SingletonEntity, TypedActionView, ViewContext};

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
            if FeatureFlag::Orchestration.is_enabled() {
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
        let appearance = Appearance::as_ref(app);
        let pane_config = self.pane_configuration.as_ref(app);
        let title = pane_config.title().to_owned();
        let clip_config = if self.is_using_conversation_for_pane_header_title {
            ClipConfig::ellipsis()
        } else {
            ClipConfig::start()
        };

        let should_render_ambient_agent_indicator = {
            let model = self.model.lock();
            model.is_shared_ambient_agent_session()
                || matches!(
                    model.conversation_transcript_viewer_status(),
                    Some(ConversationTranscriptViewerStatus::ViewingAmbientConversation(_))
                )
        };
        let pane_indicator = if should_render_ambient_agent_indicator {
            Some(self.render_ambient_agent_indicator(app))
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
            self.ai_context_model
                .as_ref(app)
                .selected_conversation(app)
                .map(|conversation| {
                    self.render_agent_indicator(
                        conversation.id(),
                        conversation.status().clone(),
                        self.is_long_running(),
                        app,
                    )
                })
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

        if FeatureFlag::CloudMode.is_enabled() {
            let is_waiting_for_session = self
                .ambient_agent_view_model
                .as_ref()
                .is_some_and(|model| model.as_ref(app).is_waiting_for_session());
            let button_element = if is_waiting_for_session {
                Some(self.render_ambient_agent_cancel_button(app))
            } else if self.can_show_cloud_mode_details_ui(app) {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    Some(self.render_cloud_mode_details_toggle_button(app))
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
        if !(FeatureFlag::Orchestration.is_enabled()
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
    ) -> Box<dyn Element> {
        if !FeatureFlag::Orchestration.is_enabled() {
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
        let header = self.maybe_add_parent_navigation_card(header, parent_conversation_header_card);

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
        let mut items = vec![];
        let source = SharedSessionActionSource::PaneHeader;

        // Shared-session related items.
        let shared_session_status = model.shared_session_status();
        let is_ambient_agent = self.is_ambient_agent_session(ctx);
        if shared_session_status.is_sharer_or_viewer() {
            if !is_ambient_agent {
                items.push(
                    MenuItemFields::new("Copy link")
                        .with_on_select_action(TerminalAction::CopySharedSessionLink { source })
                        .into_item(),
                );
            }

            if shared_session_status.is_sharer() {
                items.push(
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
                items.push(
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
            items.push(
                MenuItemFields::new("Share session")
                    .with_on_select_action(TerminalAction::OpenShareSessionModal { source })
                    .into_item(),
            );
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
            has_custom_draggable_behavior: false,
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

    /// Render the info button for toggling the cloud mode details panel.
    /// Only available on non-WASM platforms (WASM uses a per-window button instead).
    #[cfg(not(target_arch = "wasm32"))]
    fn render_cloud_mode_details_toggle_button(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let is_open = self.is_cloud_mode_details_panel_open;
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
            self.cloud_mode_details_panel_toggle_mouse_state.clone(),
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
                    PaneHeaderAction::CustomAction(TerminalAction::ToggleCloudModeDetailsPanel),
                );
            })
            .finish()
    }

    /// Render the agent indicator icon for when a conversation is selected.
    fn render_agent_indicator(
        &self,
        conversation_id: crate::ai::agent::conversation::AIConversationId,
        status: ConversationStatus,
        is_long_running: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let Some(conversation) =
            BlocklistAIHistoryModel::as_ref(app).conversation(&conversation_id)
        else {
            return warpui::elements::Empty::new().finish();
        };

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        // Check if we're configuring or waiting on an ambient agent
        let is_ambient_agent = self.is_ambient_agent_session(app);

        // When a long-running command is active, show InProgress
        // instead of the conversation's actual status.
        let status = if is_long_running {
            ConversationStatus::InProgress
        } else {
            status
        };

        if FeatureFlag::AgentView.is_enabled()
            && conversation.exchange_count() == 0
            && !is_long_running
        {
            ConstrainedBox::new(
                if is_ambient_agent {
                    WarpIcon::OzCloud
                } else {
                    WarpIcon::Oz
                }
                .to_warpui_icon(blended_colors::text_sub(theme, theme.background()).into())
                .finish(),
            )
            .with_height(appearance.ui_font_size())
            .with_width(appearance.ui_font_size())
            .finish()
        } else if FeatureFlag::NewTabStyling.is_enabled() {
            let icon_size = appearance.ui_font_size() + 2.0 - STATUS_ELEMENT_PADDING * 2.;
            render_status_element(&status, icon_size, appearance)
        } else {
            ConstrainedBox::new(status.render_icon(appearance).finish())
                .with_height(appearance.ui_font_size())
                .with_width(appearance.ui_font_size())
                .finish()
        }
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

    fn render_ambient_agent_indicator(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let icon_color = appearance
            .theme()
            .main_text_color(appearance.theme().background());
        let font_size = appearance.ui_font_size();
        let ui_builder = appearance.ui_builder().clone();

        Hoverable::new(
            self.mouse_states
                .ambient_agent_indicator_mouse_handle
                .clone(),
            move |state| {
                let mut stack = Stack::new().with_child(
                    ConstrainedBox::new(icons::Icon::OzCloud.to_warpui_icon(icon_color).finish())
                        .with_height(font_size * 1.5)
                        .with_width(font_size * 1.5)
                        .finish(),
                );
                if state.is_hovered() {
                    let tooltip = ui_builder
                        .tool_tip("Cloud agent run".to_string())
                        .build()
                        .finish();
                    stack.add_positioned_overlay_child(
                        tooltip,
                        OffsetPositioning::offset_from_parent(
                            vec2f(0., 3.),
                            ParentOffsetBounds::WindowByPosition,
                            ParentAnchor::BottomMiddle,
                            ChildAnchor::TopMiddle,
                        ),
                    );
                }

                stack.finish()
            },
        )
        .finish()
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

    /// Selected conversation status for chrome, or [`ConversationStatus::InProgress`] while the
    /// active block is long-running (terminal-derived; not mirrored in history events).
    pub fn selected_conversation_status(&self, ctx: &AppContext) -> Option<ConversationStatus> {
        let long_running = self.is_long_running();

        let Some(conversation) = self.selected_conversation_for_user_facing_chrome(ctx) else {
            // Ambient agent tabs can show Oz chrome without a filtered "chrome" conversation;
            // still surface busy while a long-running shell command is active.
            if long_running && self.is_ambient_agent_session(ctx) {
                return Some(ConversationStatus::InProgress);
            }
            return None;
        };

        if long_running {
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
    /// conversation is empty (no exchanges yet). This avoids showing a misleading "In progress"
    /// indicator when a new conversation hasn't started streaming, except when a shell command
    /// is actively long-running — that InProgress is real and should always surface.
    pub fn selected_conversation_status_for_display(
        &self,
        ctx: &AppContext,
    ) -> Option<ConversationStatus> {
        if self.selected_conversation_is_empty(ctx) && !self.is_long_running() {
            None
        } else {
            self.selected_conversation_status(ctx)
        }
    }

    pub fn selected_conversation_display_title(&self, ctx: &AppContext) -> Option<String> {
        let is_ambient_agent = self.is_ambient_agent_session(ctx);
        self.selected_conversation_for_user_facing_chrome(ctx)
            .map(|conversation| {
                self.selected_conversation_display_title_for_chrome(conversation, is_ambient_agent)
            })
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
