use super::{
    common::{
        add_command_xray_overlay, add_input_suggestions_overlays, add_voltron_overlay,
        add_workflow_info_overlay, maybe_add_buy_credits_banner,
        wrap_input_with_terminal_padding_and_focus_handler,
    },
    Input, InputAction, InputDropTargetData,
};
use crate::{
    ai::{
        blocklist::{
            agent_view::{
                agent_view_bg_fill,
                shortcuts::{render_agent_shortcuts_view, AgentShortcutsViewContext},
                AgentViewState,
            },
            InputType,
        },
        harness_availability::HarnessAvailabilityModel,
    },
    appearance::Appearance,
    context_chips::spacing::{self},
    editor::position_id_for_cursor,
    features::FeatureFlag,
    settings::InputModeSettings,
    terminal::{settings::TerminalSettings, view::TerminalAction},
    BlocklistAIHistoryModel,
};
use warp_cli::agent::Harness;
use warp_core::settings::Setting;
use warp_core::ui::theme::color::internal_colors;
use warpui::{
    elements::{
        Align, AnchorPair, Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
        DispatchEventResult, DropTarget, Element, Empty, EventHandler, Expanded, Flex, Hoverable,
        MainAxisSize, OffsetPositioning, OffsetType, ParentElement, PositionedElementOffsetBounds,
        PositioningAxis, Radius, SavePosition, Stack, XAxisAnchor, YAxisAnchor,
    },
    presenter::ChildView,
    AppContext, SingletonEntity as _,
};

pub(super) const CLOUD_MODE_V2_MAX_WIDTH: f32 = 720.;

const CLOUD_MODE_V2_INPUT_RADIUS: f32 = 8.;

const CLOUD_MODE_V2_TOP_ROW_GAP: f32 = 10.;

const CLOUD_MODE_V2_INPUT_HORIZONTAL_PADDING: f32 = 16.;

const CLOUD_MODE_V2_INPUT_TOP_PADDING: f32 = 16.;

const CLOUD_MODE_V2_INPUT_EDITOR_BOTTOM_PADDING: f32 = 8.;

const CLOUD_MODE_V2_INPUT_BOTTOM_PADDING: f32 = 16.;

const CLOUD_MODE_V2_TOP_ROW_INNER_GAP: f32 = 4.;

const CLOUD_MODE_V2_INPUT_MIN_EDITOR_HEIGHT: f32 = 80.;

/// Horizontal gutter applied symmetrically on both sides of the V2 cloud-mode
/// composing UI so the floating input has matching breathing room on the left
/// and right at narrow widths.
const CLOUD_MODE_V2_HORIZONTAL_GUTTER: f32 = 16.;

// Top padding above the attachment chips row inside the V2 input container.
const CLOUD_MODE_V2_CHIPS_ROW_TOP_PADDING: f32 = 4.;

impl Input {
    pub fn is_cloud_mode_input_v2_composing(&self, app: &AppContext) -> bool {
        FeatureFlag::CloudModeInputV2.is_enabled()
            && FeatureFlag::CloudMode.is_enabled()
            && self.ambient_agent_view_model().is_some_and(|model| {
                let view_model = model.as_ref(app);
                view_model.is_configuring_ambient_agent()
                    // The handoff pane intentionally stays on the existing input UI even
                    // when V2 is on — V2 is for fresh cloud-mode runs only, and handoff has
                    // its own pre-spawn flow (submit interception).
                    && !view_model.is_local_to_cloud_handoff()
            })
    }

    /// Renders the input when there is an active `AgentView`.
    ///
    /// Only used when `FeatureFlag::AgentView` is enabled.
    pub(super) fn render_agent_input(&self, app: &AppContext) -> Box<dyn Element> {
        if self.is_cloud_mode_input_v2_composing(app) {
            return self.render_cloud_mode_v2_composing_input(app);
        }

        let appearance = Appearance::as_ref(app);
        let menu_positioning = self.menu_positioning(app);

        let model = self.model.lock();

        // We should likely rework this stack to not need to use `with_constrain_absolute_children`,
        // by reworking the positioning of the children to not depend on this.
        let mut stack = Stack::new().with_constrain_absolute_children();

        let input_mode = *InputModeSettings::as_ref(app).input_mode.value();

        let mut column = Flex::column();

        if let Some(banner) =
            self.render_input_banner(appearance, app, input_mode, /*is_compact_mode=*/ false)
        {
            column.add_child(
                Container::new(banner)
                    .with_margin_top(spacing::UDI_CHIP_MARGIN)
                    .finish(),
            );
        }

        let ai_input_model = self.ai_input_model.as_ref(app);

        if FeatureFlag::ImageAsContext.is_enabled()
            && matches!(ai_input_model.input_type(), InputType::AI)
        {
            if let Some(images) = self.render_attachment_chips(appearance) {
                column.add_child(
                    Container::new(images)
                        .with_margin_top(spacing::UDI_CHIP_MARGIN)
                        .finish(),
                );
            }
        }

        let show_harness_row = FeatureFlag::CloudMode.is_enabled()
            && HarnessAvailabilityModel::as_ref(app).should_show_harness_selector()
            && self
                .ambient_agent_view_model()
                .is_some_and(|ambient_agent_model| {
                    ambient_agent_model
                        .as_ref(app)
                        .is_configuring_ambient_agent()
                });
        if show_harness_row {
            if let Some(harness_selector) = self.harness_selector() {
                // Temporarily render the harness selector in the cloud mode UDI until we fully
                // implement the new designs.
                let harness_row = Flex::row()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_child(ChildView::new(harness_selector).finish())
                    .finish();
                column.add_child(
                    Container::new(harness_row)
                        .with_padding_top(spacing::UDI_CHIP_MARGIN)
                        .with_padding_bottom(4.)
                        .finish(),
                );
            }
        }

        let terminal_spacing = TerminalSettings::as_ref(app)
            .terminal_input_spacing(appearance.line_height_ratio(), app);
        column.add_child(
            Container::new(self.render_input_box(/*show_vim_status=*/ false, appearance, app))
                .with_margin_top(
                    terminal_spacing.prompt_to_editor_padding
                        * spacing::UDI_PROMPT_BOTTOM_PADDING_FACTOR,
                )
                .finish(),
        );
        column.add_child(
            SavePosition::new(
                ChildView::new(&self.agent_input_footer).finish(),
                &self.prompt_save_position_id(),
            )
            .finish(),
        );

        stack.add_child(wrap_input_with_terminal_padding_and_focus_handler(
            self.is_active_session(app),
            column.finish(),
            false,
        ));

        if let Some(selected_workflow_state) = self.workflows_state.selected_workflow_state.as_ref()
        {
            if selected_workflow_state.should_show_more_info_view {
                add_workflow_info_overlay(
                    &mut stack,
                    selected_workflow_state,
                    self.size_info(app).pane_height_px().as_f32(),
                    menu_positioning,
                );
            }
        }

        if self.is_voltron_open && self.is_pane_focused(app) {
            add_voltron_overlay(&mut stack, &self.voltron_view, menu_positioning);
        }

        if self.is_pane_focused(app) {
            add_input_suggestions_overlays(self, &mut stack, appearance, menu_positioning, app);
        }

        if let Some(token_description) = &self.command_x_ray_description {
            add_command_xray_overlay(
                self,
                &mut stack,
                token_description,
                appearance,
                menu_positioning,
                app,
            );
        }

        let drop_target = DropTarget::new(
            SavePosition::new(stack.finish(), &self.status_free_input_save_position_id()).finish(),
            InputDropTargetData::new(self.weak_view_handle.clone()),
        )
        .finish();

        let border_color = if self.handoff_compose_state.as_ref(app).is_active() {
            appearance.theme().ansi_fg_magenta()
        } else if !self.ai_input_model.as_ref(app).is_ai_input_enabled()
            && !self.suggestions_mode_model.as_ref(app).is_slash_commands()
            && !self.slash_command_model.as_ref(app).state().is_detected_command()
            // If NLD, don't color the border if the input is empty, because the current
            // classification is necessarily stale (intentionally inherited from the last
            // classification prior to clearing the input)
            && (!self.editor.as_ref(app).is_empty(app)
                || self.ai_input_model.as_ref(app).is_input_type_locked())
        {
            appearance.theme().ansi_fg_blue()
        } else {
            styles::default_border_color(appearance.theme())
        };

        let mut input = Container::new(
            Hoverable::new(self.hoverable_handle.clone(), |_| drop_target)
                .on_hover(|is_hovered, ctx, _app, _position| {
                    ctx.dispatch_typed_action(InputAction::SetUDIHovered(is_hovered));
                })
                .on_middle_click(|ctx, _app, _position| {
                    ctx.dispatch_typed_action(TerminalAction::MiddleClickOnInput)
                })
                .finish(),
        )
        .with_border(Border::top(1.).with_border_color(border_color))
        .with_padding_bottom(4.);

        if self.agent_view_controller.as_ref(app).is_inline() {
            input = input.with_background(agent_view_bg_fill(app));
        }

        let input = input.finish();

        let mut column = Flex::column();

        if self
            .suggestions_mode_model
            .as_ref(app)
            .is_inline_model_selector()
        {
            column.add_child(ChildView::new(&self.inline_model_selector_view).finish());
        } else if FeatureFlag::InlineProfileSelector.is_enabled()
            && self
                .suggestions_mode_model
                .as_ref(app)
                .is_profile_selector()
        {
            column.add_child(ChildView::new(&self.inline_profile_selector_view).finish());
        } else if self.suggestions_mode_model.as_ref(app).is_slash_commands()
            && !self.is_cloud_mode_input_v2_composing(app)
        {
            column.add_child(ChildView::new(&self.inline_slash_commands_view).finish());
        } else if self.suggestions_mode_model.as_ref(app).is_prompts_menu() {
            column.add_child(ChildView::new(&self.inline_prompts_menu_view).finish());
        } else if self
            .suggestions_mode_model
            .as_ref(app)
            .is_conversation_menu()
        {
            column.add_child(ChildView::new(&self.inline_conversation_menu_view).finish());
        } else if FeatureFlag::ListSkills.is_enabled()
            && self.suggestions_mode_model.as_ref(app).is_skill_menu()
        {
            column.add_child(ChildView::new(&self.inline_skill_selector_view).finish());
        } else if self.suggestions_mode_model.as_ref(app).is_user_query_menu() {
            column.add_child(ChildView::new(&self.user_query_menu_view).finish());
        } else if self.suggestions_mode_model.as_ref(app).is_rewind_menu() {
            column.add_child(ChildView::new(&self.rewind_menu_view).finish());
        } else if self
            .suggestions_mode_model
            .as_ref(app)
            .is_inline_history_menu()
        {
            column.add_child(ChildView::new(&self.inline_history_menu_view).finish());
        } else if self.suggestions_mode_model.as_ref(app).is_repos_menu() {
            column.add_child(ChildView::new(&self.inline_repos_menu_view).finish());
        } else if self.suggestions_mode_model.as_ref(app).is_plan_menu() {
            column.add_child(ChildView::new(&self.inline_plan_menu_view).finish());
        }

        if self
            .agent_shortcut_view_model
            .as_ref(app)
            .is_shortcut_view_open()
        {
            let agent_view_controller = self.agent_view_controller.as_ref(app);
            let (is_cloud_agent, has_submitted_first_prompt) =
                match agent_view_controller.agent_view_state() {
                    AgentViewState::Active {
                        conversation_id,
                        origin,
                        ..
                    } => {
                        let is_cloud_agent = origin.is_cloud_agent();
                        let has_submitted_first_prompt = if is_cloud_agent {
                            BlocklistAIHistoryModel::as_ref(app)
                                .conversation(conversation_id)
                                .is_some_and(|c| c.initial_user_query().is_some())
                        } else {
                            true
                        };
                        (is_cloud_agent, has_submitted_first_prompt)
                    }
                    // When inactive, show all shortcuts (treat as not-cloud and not in the zero-state).
                    AgentViewState::Inactive => (false, true),
                };

            column.add_child(render_agent_shortcuts_view(
                AgentShortcutsViewContext {
                    is_cloud_agent,
                    has_submitted_first_prompt,
                },
                app,
            ));
        }
        column.add_children([ChildView::new(&self.agent_status_view).finish(), input]);

        let mut outer_stack = Stack::new().with_constrain_absolute_children();
        outer_stack.add_child(column.finish());
        maybe_add_buy_credits_banner(
            &mut outer_stack,
            &self.buy_credits_banner,
            self.is_pane_focused(app),
            self.terminal_view_id,
            self.is_input_at_top(&model, app),
            app,
        );

        SavePosition::new(outer_stack.finish(), &self.save_position_id()).finish()
    }

    fn render_cloud_mode_v2_composing_input(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let menu_positioning = self.menu_positioning(app);
        let model = self.model.lock();

        let mut stack = Stack::new();

        // Apply the V2 gutter symmetrically (left + right) so the floating
        // input keeps equal breathing room on both sides as the pane shrinks.
        // The shared `wrap_input_with_terminal_padding_and_focus_handler`
        // helper only pads the left, so V2 inlines its own padding + focus
        // handler instead of routing through it.
        let centered_content = Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(
                    Expanded::new(1., self.render_cloud_mode_v2_content(appearance, app)).finish(),
                )
                .finish(),
        )
        .with_padding_left(CLOUD_MODE_V2_HORIZONTAL_GUTTER)
        .with_padding_right(CLOUD_MODE_V2_HORIZONTAL_GUTTER)
        .finish();

        let centered_content = if self.is_active_session(app) {
            EventHandler::new(centered_content)
                .on_left_mouse_down(|ctx, _, _| {
                    ctx.dispatch_typed_action(TerminalAction::ClearSelectionsWhenShellMode);
                    ctx.dispatch_typed_action(InputAction::FocusInputBox);
                    ctx.dispatch_typed_action(InputAction::DismissCloudModeV2SlashCommandsMenu);
                    DispatchEventResult::StopPropagation
                })
                .finish()
        } else {
            centered_content
        };

        stack.add_child(centered_content);

        if let Some(history_menu) = self.render_cloud_mode_v2_history_menu(app) {
            let prompt_position = self.prompt_save_position_id();
            stack.add_positioned_overlay_child(
                ConstrainedBox::new(history_menu)
                    .with_max_width(CLOUD_MODE_V2_MAX_WIDTH)
                    .finish(),
                OffsetPositioning::from_axes(
                    PositioningAxis::relative_to_stack_child(
                        &prompt_position,
                        PositionedElementOffsetBounds::WindowByPosition,
                        OffsetType::Pixel(0.),
                        AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
                    ),
                    PositioningAxis::relative_to_stack_child(
                        &prompt_position,
                        PositionedElementOffsetBounds::Unbounded,
                        OffsetType::Pixel(-CLOUD_MODE_V2_TOP_ROW_GAP),
                        AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Bottom),
                    ),
                ),
            );
        }

        if self.suggestions_mode_model.as_ref(app).is_slash_commands() {
            if let Some(view) = self.cloud_mode_v2_slash_commands_view.as_ref() {
                let cursor_position = position_id_for_cursor(self.editor.id());
                stack.add_positioned_overlay_child(
                    ChildView::new(view).finish(),
                    OffsetPositioning::from_axes(
                        PositioningAxis::relative_to_stack_child(
                            &cursor_position,
                            PositionedElementOffsetBounds::WindowByPosition,
                            OffsetType::Pixel(0.),
                            AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
                        ),
                        PositioningAxis::relative_to_stack_child(
                            &cursor_position,
                            PositionedElementOffsetBounds::Unbounded,
                            OffsetType::Pixel(4.),
                            AnchorPair::new(YAxisAnchor::Bottom, YAxisAnchor::Top),
                        ),
                    ),
                );
            }
        }

        if let Some(selected_workflow_state) = self.workflows_state.selected_workflow_state.as_ref()
        {
            if selected_workflow_state.should_show_more_info_view {
                let prompt_position = self.prompt_save_position_id();
                let workflows_info_view = Container::new(
                    ChildView::new(&selected_workflow_state.more_info_view).finish(),
                )
                .finish();
                stack.add_positioned_overlay_child(
                    ConstrainedBox::new(workflows_info_view)
                        .with_max_width(CLOUD_MODE_V2_MAX_WIDTH)
                        .with_max_height(self.size_info(app).pane_height_px().as_f32() * 0.35)
                        .finish(),
                    OffsetPositioning::from_axes(
                        PositioningAxis::relative_to_stack_child(
                            &prompt_position,
                            PositionedElementOffsetBounds::WindowByPosition,
                            OffsetType::Pixel(0.),
                            AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
                        ),
                        PositioningAxis::relative_to_stack_child(
                            &prompt_position,
                            PositionedElementOffsetBounds::Unbounded,
                            OffsetType::Pixel(0.),
                            AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Bottom),
                        ),
                    ),
                );
            }
        }
        if self.is_voltron_open && self.is_pane_focused(app) {
            add_voltron_overlay(&mut stack, &self.voltron_view, menu_positioning);
        }
        if self.is_pane_focused(app) {
            add_input_suggestions_overlays(self, &mut stack, appearance, menu_positioning, app);
        }
        if let Some(token_description) = &self.command_x_ray_description {
            add_command_xray_overlay(
                self,
                &mut stack,
                token_description,
                appearance,
                menu_positioning,
                app,
            );
        }

        let drop_target = DropTarget::new(
            SavePosition::new(stack.finish(), &self.status_free_input_save_position_id()).finish(),
            InputDropTargetData::new(self.weak_view_handle.clone()),
        )
        .finish();

        let input = Hoverable::new(self.hoverable_handle.clone(), |_| drop_target)
            .on_hover(|is_hovered, ctx, _app, _position| {
                ctx.dispatch_typed_action(InputAction::SetUDIHovered(is_hovered));
            })
            .on_middle_click(|ctx, _app, _position| {
                ctx.dispatch_typed_action(TerminalAction::MiddleClickOnInput)
            })
            .finish();

        let mut outer_stack = Stack::new().with_constrain_absolute_children();
        outer_stack.add_child(input);
        maybe_add_buy_credits_banner(
            &mut outer_stack,
            &self.buy_credits_banner,
            self.is_pane_focused(app),
            self.terminal_view_id,
            self.is_input_at_top(&model, app),
            app,
        );

        SavePosition::new(outer_stack.finish(), &self.save_position_id()).finish()
    }

    pub(super) fn should_show_auth_secret_ftux(&self, app: &AppContext) -> bool {
        let Some(view_model) = self.ambient_agent_view_model() else {
            return false;
        };
        let vm = view_model.as_ref(app);
        let harness = vm.selected_harness();
        if harness == Harness::Oz {
            return false;
        }
        // Skip FTUX for harnesses that have no auth secret types defined.
        if crate::ai::auth_secret_types::auth_secret_types_for_harness(harness).is_empty() {
            return false;
        }
        if let Some(ftux_view) = self.auth_secret_ftux_view() {
            if ftux_view.as_ref(app).has_creation_state() {
                return true;
            }
        }
        if crate::ai::cloud_agent_settings::CloudAgentSettings::as_ref(app)
            .is_harness_auth_ftux_completed(harness)
        {
            return false;
        }
        vm.selected_harness_auth_secret_name().is_none()
    }

    fn render_cloud_mode_v2_content(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(CLOUD_MODE_V2_TOP_ROW_GAP);

        column.add_child(self.render_cloud_mode_v2_top_row(app));

        if self.should_show_auth_secret_ftux(app) {
            column.add_child(self.render_auth_secret_ftux_content());
        } else {
            column.add_child(self.render_cloud_mode_v2_input_container(appearance, app));
        }

        Align::new(
            ConstrainedBox::new(column.finish())
                .with_max_width(CLOUD_MODE_V2_MAX_WIDTH)
                .finish(),
        )
        .finish()
    }

    fn render_auth_secret_ftux_content(&self) -> Box<dyn Element> {
        match self.auth_secret_ftux_view() {
            Some(view) => ChildView::new(view).finish(),
            None => Empty::new().finish(),
        }
    }

    fn render_cloud_mode_v2_history_menu(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        if !self
            .suggestions_mode_model
            .as_ref(app)
            .is_inline_history_menu()
        {
            return None;
        }
        let view = self.cloud_mode_v2_history_menu_view.as_ref()?;
        Some(ChildView::new(view).finish())
    }

    fn render_cloud_mode_v2_top_row(&self, app: &AppContext) -> Box<dyn Element> {
        let mut row = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(CLOUD_MODE_V2_TOP_ROW_INNER_GAP);

        // Only show the host selector when a default host is configured.
        if let Some(host) = self.host_selector() {
            if host.as_ref(app).has_default_host() {
                row.add_child(ChildView::new(host).finish());
            }
        }
        if let Some(harness_selector) = self.harness_selector() {
            row.add_child(ChildView::new(harness_selector).finish());
        }

        if let Some(auth_secret_selector) = self.auth_secret_selector() {
            let harness = self
                .ambient_agent_view_model()
                .map(|m| m.as_ref(app).selected_harness())
                .unwrap_or(warp_cli::agent::Harness::Oz);
            if harness != warp_cli::agent::Harness::Oz && !self.should_show_auth_secret_ftux(app) {
                row.add_child(ChildView::new(auth_secret_selector).finish());
            }
        }

        row.finish()
    }

    fn render_cloud_mode_v2_input_container(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background = internal_colors::fg_overlay_1(theme);
        let border_color = internal_colors::neutral_2(theme);

        let editor_with_min_height =
            ConstrainedBox::new(self.render_input_box(/*show_vim_status=*/ false, appearance, app))
                .with_min_height(CLOUD_MODE_V2_INPUT_MIN_EDITOR_HEIGHT)
                .finish();

        let mut editor_column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min);

        let ai_input_model = self.ai_input_model.as_ref(app);
        let show_chips = FeatureFlag::ImageAsContext.is_enabled()
            && matches!(ai_input_model.input_type(), InputType::AI);
        if show_chips {
            if let Some(chips) = self.render_attachment_chips(appearance) {
                editor_column.add_child(
                    Container::new(chips)
                        .with_padding_top(CLOUD_MODE_V2_CHIPS_ROW_TOP_PADDING)
                        .with_padding_left(CLOUD_MODE_V2_INPUT_HORIZONTAL_PADDING)
                        .with_padding_right(CLOUD_MODE_V2_INPUT_HORIZONTAL_PADDING)
                        .finish(),
                );
            }
        }

        editor_column.add_child(
            Container::new(editor_with_min_height)
                .with_padding_top(CLOUD_MODE_V2_INPUT_TOP_PADDING)
                .with_padding_bottom(CLOUD_MODE_V2_INPUT_EDITOR_BOTTOM_PADDING)
                .with_padding_left(CLOUD_MODE_V2_INPUT_HORIZONTAL_PADDING)
                .with_padding_right(CLOUD_MODE_V2_INPUT_HORIZONTAL_PADDING)
                .finish(),
        );

        let editor = editor_column.finish();

        let footer = Container::new(ChildView::new(&self.agent_input_footer).finish())
            .with_padding_bottom(CLOUD_MODE_V2_INPUT_BOTTOM_PADDING)
            .with_padding_left(CLOUD_MODE_V2_INPUT_HORIZONTAL_PADDING)
            .with_padding_right(CLOUD_MODE_V2_INPUT_HORIZONTAL_PADDING)
            .finish();

        let stacked = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(editor)
            .with_child(footer)
            .finish();

        Container::new(SavePosition::new(stacked, &self.prompt_save_position_id()).finish())
            .with_background(background)
            .with_border(Border::all(1.).with_border_color(border_color))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                CLOUD_MODE_V2_INPUT_RADIUS,
            )))
            .finish()
    }

    pub(super) fn render_ambient_agent_status_footer(&self, app: &AppContext) -> Box<dyn Element> {
        let Some(ambient_agent_model) = self.ambient_agent_view_model() else {
            return Empty::new().finish();
        };
        let ambient_agent_model = ambient_agent_model.as_ref(app);
        let mut stack = Stack::new().with_constrain_absolute_children();

        // Don't render status bar when agent has failed or is waiting for session
        let show_status_bar = ambient_agent_model.error_message().is_none()
            && !ambient_agent_model.is_waiting_for_session();

        let model = self.model.lock();
        maybe_add_buy_credits_banner(
            &mut stack,
            &self.buy_credits_banner,
            self.focus_handle.as_ref().is_none_or(|h| h.is_focused(app)),
            self.terminal_view_id,
            self.is_input_at_top(&model, app),
            app,
        );

        let save_position =
            SavePosition::new(stack.finish(), &self.status_free_input_save_position_id()).finish();

        let input = Hoverable::new(self.hoverable_handle.clone(), |_| save_position)
            .on_hover(|is_hovered, ctx, _app, _position| {
                ctx.dispatch_typed_action(InputAction::SetUDIHovered(is_hovered));
            })
            .on_middle_click(|ctx, _app, _position| {
                ctx.dispatch_typed_action(TerminalAction::MiddleClickOnInput);
            })
            .finish();

        let mut column = Flex::column();
        if show_status_bar {
            column.add_child(ChildView::new(&self.agent_status_view).finish());
        }
        column.add_child(input);

        SavePosition::new(column.finish(), &self.save_position_id()).finish()
    }
}

pub mod styles {
    use pathfinder_color::ColorU;
    use warp_core::ui::theme::WarpTheme;

    use crate::ui_components::blended_colors;

    pub fn default_border_color(theme: &WarpTheme) -> ColorU {
        blended_colors::neutral_2(theme)
    }
}
