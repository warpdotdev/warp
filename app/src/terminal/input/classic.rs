use crate::{
    ai::blocklist::InputType,
    appearance::Appearance,
    context_chips::spacing,
    features::FeatureFlag,
    settings::{AppEditorSettings, InputModeSettings},
    terminal::{
        block_list_settings::BlockListSettings,
        block_list_viewport::InputMode,
        input::{
            common::{
                add_command_xray_overlay, add_input_suggestions_overlays, add_vim_status_to_stack,
                add_voltron_overlay, add_workflow_info_overlay,
                should_show_terminal_input_message_bar,
                wrap_input_with_terminal_padding_and_focus_handler,
            },
            get_input_box_top_border_width, InputDropTargetData,
        },
        settings::{SpacingMode, TerminalSettings},
        view::TerminalAction,
        warpify::render::{render_subshell_flag, render_subshell_flag_pole},
    },
};
use pathfinder_geometry::vector::vec2f;
use settings::Setting;
use warpui::{
    elements::{
        Border, ChildAnchor, ChildView, Clipped, Container, DropTarget, Element, Empty, Flex,
        Hoverable, OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds,
        SavePosition, Stack,
    },
    AppContext, SingletonEntity,
};

use super::{should_render_prompt_using_editor_decorator_elements, Input, SubshellRenderState};

impl Input {
    /// Renders the classic input. This is used when the user has 'Honor PS1' enabled in settings,
    /// OR if `FeatureFlag::AgentView` is disabled and the user has 'Classic' input type selected
    /// in settings.
    pub(super) fn render_classic_input(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let menu_positioning = self.menu_positioning(app);

        let model = self.model.lock();
        let should_render_prompt_using_editor_decorator_elements =
            should_render_prompt_using_editor_decorator_elements(
                false,
                &self.ai_input_model,
                &model,
                app,
            );

        // We should likely rework this stack to not need to use `with_constrain_absolute_children`,
        // by reworking the positioning of the children to not depend on this.
        let mut stack = Stack::new().with_constrain_absolute_children();

        let mut prompt_row = Stack::new();

        let (lprompt_top_area_option, rprompt_area_option);
        let mut prompt_top_padding_row = Stack::new();
        if should_render_prompt_using_editor_decorator_elements {
            // These are rendered as sections/notches in the EditorElement.
            lprompt_top_area_option = None;
            rprompt_area_option = None;

            let terminal_spacing = TerminalSettings::as_ref(app)
                .terminal_input_spacing(appearance.line_height_ratio(), app);
            let default_prompt_top_padding = terminal_spacing.block_padding.padding_top
                * self.size_info(app).cell_height_px().as_f32()
                - get_input_box_top_border_width();
            let prompt_top_padding_element = Container::new(Empty::new().finish())
                .with_padding_top(default_prompt_top_padding)
                .finish();
            prompt_top_padding_row.add_child(prompt_top_padding_element);
        } else {
            let prompt_elements = self.prompt_render_helper.render_prompt_areas(
                &model,
                appearance,
                self.effective_monospace_font_size(app),
                app,
            );
            lprompt_top_area_option = prompt_elements.lprompt;
            rprompt_area_option = prompt_elements.rprompt;
        }

        if !should_render_prompt_using_editor_decorator_elements {
            if let Some(lprompt_top_area) = lprompt_top_area_option {
                prompt_row.add_child(lprompt_top_area);
            }
            if let Some(rprompt_area) = rprompt_area_option {
                let block = &model.block_list().active_block();
                prompt_row.add_positioned_child(
                    rprompt_area,
                    OffsetPositioning::offset_from_parent(
                        block.rprompt_render_offset(&self.size_info(app)),
                        ParentOffsetBounds::Unbounded,
                        ParentAnchor::TopLeft,
                        ChildAnchor::TopLeft,
                    ),
                );
            }
        }

        let vim_state = self.editor.as_ref(app).vim_state(app);
        let app_editor_settings = AppEditorSettings::as_ref(app);
        let show_vim_status = vim_state.is_some() && *app_editor_settings.vim_status_bar.value();
        let input_mode = *InputModeSettings::as_ref(app).input_mode.value();

        let is_compact_mode = matches!(
            TerminalSettings::as_ref(app).spacing_mode.value(),
            SpacingMode::Compact
        );
        let mut column = Flex::column();

        if matches!(input_mode, InputMode::PinnedToBottom | InputMode::Waterfall) {
            if let Some(banner) =
                self.render_input_banner(appearance, app, input_mode, is_compact_mode)
            {
                column.add_child(banner);
            }
        }

        column.add_children([prompt_top_padding_row.finish(), prompt_row.finish()]);

        let ai_input_model = self.ai_input_model.as_ref(app);

        if FeatureFlag::ImageAsContext.is_enabled()
            && matches!(ai_input_model.input_type(), InputType::AI)
            && !FeatureFlag::AgentView.is_enabled()
        {
            if let Some(images) = self.render_attachment_chips(appearance) {
                column.add_child(
                    Container::new(images)
                        .with_padding_bottom(spacing::CLASSIC_PROMPT_ATTACH_IMAGES_BOTTOM_PADDING)
                        .finish(),
                );
            }
        }

        column.add_child(self.render_input_box(show_vim_status, appearance, app));

        if should_show_terminal_input_message_bar(&model, app) {
            column.add_child(
                Clipped::new(ChildView::new(&self.terminal_input_message_bar).finish()).finish(),
            );
        } else if !(matches!(input_mode, InputMode::PinnedToTop)
            && self
                .suggestions_mode_model
                .as_ref(app)
                .is_inline_menu_open())
        {
            column.add_child(
                Container::new(Flex::row().finish())
                    .with_margin_bottom(4.)
                    .finish(),
            );
        }

        if matches!(input_mode, InputMode::PinnedToTop) {
            if let Some(banner) =
                self.render_input_banner(appearance, app, input_mode, is_compact_mode)
            {
                column.add_child(banner);
            }
        }

        let subshell_flag = self.get_subshell_flag_render_state(&model, is_compact_mode, app);

        let should_extend_flag = subshell_flag.is_some();

        if should_extend_flag {
            let max_height = self.size_info(app).pane_height_px().as_f32();
            stack.add_positioned_child(
                render_subshell_flag_pole(max_height, theme.subshell_background()),
                OffsetPositioning::offset_from_parent(
                    vec2f(0.0, 0.0),
                    ParentOffsetBounds::ParentBySize,
                    ParentAnchor::TopLeft,
                    ChildAnchor::TopLeft,
                ),
            );
        }

        if let Some(SubshellRenderState::Flag(command)) = subshell_flag {
            let flag = render_subshell_flag(
                command,
                appearance.monospace_font_family(),
                appearance.monospace_font_size(),
                theme,
            );
            stack.add_positioned_child(
                flag,
                OffsetPositioning::offset_from_parent(
                    vec2f(0.0, 0.0),
                    ParentOffsetBounds::Unbounded,
                    ParentAnchor::TopLeft,
                    ChildAnchor::TopLeft,
                ),
            );
        }

        if !FeatureFlag::AgentView.is_enabled() {
            if let Some(vim_state) = vim_state.as_ref() {
                if show_vim_status {
                    add_vim_status_to_stack(
                        &mut stack, vim_state, appearance,
                        false, // legacy doesn't use adjusted padding for vim status
                    );
                }
            }
        }

        stack.add_child(wrap_input_with_terminal_padding_and_focus_handler(
            self.is_active_session(app),
            column.finish(),
            false, // legacy uses full padding
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

        let input_mode = *InputModeSettings::as_ref(app).input_mode.value();

        // When AgentView is enabled, match terminal-mode input behavior and only render the
        // divider adjacent to the status/message line when block dividers are enabled.
        let show_block_dividers = *BlockListSettings::as_ref(app).show_block_dividers.value();
        let should_render_divider = !FeatureFlag::AgentView.is_enabled() || show_block_dividers;

        let border = match input_mode {
            InputMode::PinnedToBottom => Border::top(if should_render_divider {
                get_input_box_top_border_width()
            } else {
                0.
            })
            .with_border_fill(theme.outline()),
            InputMode::PinnedToTop => Border::bottom(if should_render_divider {
                get_input_box_top_border_width()
            } else {
                0.
            })
            .with_border_fill(theme.outline()),
            InputMode::Waterfall => Border::new(get_input_box_top_border_width())
                .with_sides(true, false, true, false)
                .with_border_fill(theme.outline()),
        };

        let drop_target = DropTarget::new(
            Container::new(stack.finish()).with_border(border).finish(),
            InputDropTargetData::new(self.weak_view_handle.clone()),
        )
        .finish();

        let input = SavePosition::new(
            Hoverable::new(self.hoverable_handle.clone(), |_| drop_target)
                .on_middle_click(|ctx, _app, _position| {
                    ctx.dispatch_typed_action(TerminalAction::MiddleClickOnInput)
                })
                .finish(),
            &self.status_free_input_save_position_id(),
        )
        .finish();

        let mut column = Flex::column();
        let is_slash_commands = self.suggestions_mode_model.as_ref(app).is_slash_commands();
        let is_conversation_menu = self
            .suggestions_mode_model
            .as_ref(app)
            .is_conversation_menu();
        let is_model_selector = self
            .suggestions_mode_model
            .as_ref(app)
            .is_inline_model_selector();
        let is_prompts_menu = self.suggestions_mode_model.as_ref(app).is_prompts_menu();
        let is_skill_menu = self.suggestions_mode_model.as_ref(app).is_skill_menu();
        let is_inline_history_menu = FeatureFlag::InlineHistoryMenu.is_enabled()
            && self
                .suggestions_mode_model
                .as_ref(app)
                .is_inline_history_menu();
        let is_repos_menu = FeatureFlag::InlineRepoMenu.is_enabled()
            && self.suggestions_mode_model.as_ref(app).is_repos_menu();

        match input_mode {
            InputMode::PinnedToBottom => {
                column.add_children(
                    [
                        if is_model_selector {
                            Some(ChildView::new(&self.inline_model_selector_view).finish())
                        } else if is_slash_commands {
                            Some(ChildView::new(&self.inline_slash_commands_view).finish())
                        } else if is_prompts_menu {
                            Some(ChildView::new(&self.inline_prompts_menu_view).finish())
                        } else if is_conversation_menu {
                            Some(ChildView::new(&self.inline_conversation_menu_view).finish())
                        } else if FeatureFlag::ListSkills.is_enabled() && is_skill_menu {
                            Some(ChildView::new(&self.inline_skill_selector_view).finish())
                        } else if is_inline_history_menu {
                            Some(ChildView::new(&self.inline_history_menu_view).finish())
                        } else if is_repos_menu {
                            Some(ChildView::new(&self.inline_repos_menu_view).finish())
                        } else {
                            None
                        },
                        Some(ChildView::new(&self.agent_status_view).finish()),
                        Some(input),
                    ]
                    .into_iter()
                    .flatten(),
                );
            }
            InputMode::PinnedToTop => {
                column.add_children(
                    [
                        Some(input),
                        Some(ChildView::new(&self.agent_status_view).finish()),
                        if is_model_selector {
                            Some(ChildView::new(&self.inline_model_selector_view).finish())
                        } else if is_slash_commands {
                            Some(ChildView::new(&self.inline_slash_commands_view).finish())
                        } else if is_prompts_menu {
                            Some(ChildView::new(&self.inline_prompts_menu_view).finish())
                        } else if is_conversation_menu {
                            Some(ChildView::new(&self.inline_conversation_menu_view).finish())
                        } else if FeatureFlag::ListSkills.is_enabled() && is_skill_menu {
                            Some(ChildView::new(&self.inline_skill_selector_view).finish())
                        } else if is_inline_history_menu {
                            Some(ChildView::new(&self.inline_history_menu_view).finish())
                        } else if is_repos_menu {
                            Some(ChildView::new(&self.inline_repos_menu_view).finish())
                        } else {
                            None
                        },
                    ]
                    .into_iter()
                    .flatten(),
                );
            }
            InputMode::Waterfall => {
                let should_render_below = self
                    .inline_terminal_menu_positioner
                    .as_ref(app)
                    .should_render_inline_menu_below_input();

                if is_slash_commands && !should_render_below {
                    column.add_child(ChildView::new(&self.inline_slash_commands_view).finish());
                } else if is_prompts_menu && !should_render_below {
                    column.add_child(ChildView::new(&self.inline_prompts_menu_view).finish());
                } else if is_conversation_menu && !should_render_below {
                    column.add_child(ChildView::new(&self.inline_conversation_menu_view).finish());
                } else if FeatureFlag::ListSkills.is_enabled()
                    && is_skill_menu
                    && !should_render_below
                {
                    column.add_child(ChildView::new(&self.inline_skill_selector_view).finish());
                } else if is_inline_history_menu && !should_render_below {
                    column.add_child(ChildView::new(&self.inline_history_menu_view).finish());
                } else if is_repos_menu && !should_render_below {
                    column.add_child(ChildView::new(&self.inline_repos_menu_view).finish());
                }

                column.add_children([ChildView::new(&self.agent_status_view).finish(), input]);

                if is_model_selector && should_render_below {
                    column.add_child(ChildView::new(&self.inline_model_selector_view).finish());
                } else if is_slash_commands && should_render_below {
                    column.add_child(ChildView::new(&self.inline_slash_commands_view).finish());
                } else if is_prompts_menu && should_render_below {
                    column.add_child(ChildView::new(&self.inline_prompts_menu_view).finish());
                } else if is_conversation_menu && should_render_below {
                    column.add_child(ChildView::new(&self.inline_conversation_menu_view).finish());
                } else if FeatureFlag::ListSkills.is_enabled()
                    && is_skill_menu
                    && should_render_below
                {
                    column.add_child(ChildView::new(&self.inline_skill_selector_view).finish());
                } else if is_inline_history_menu && should_render_below {
                    column.add_child(ChildView::new(&self.inline_history_menu_view).finish());
                } else if is_repos_menu && should_render_below {
                    column.add_child(ChildView::new(&self.inline_repos_menu_view).finish());
                }
            }
        }

        SavePosition::new(column.finish(), &self.save_position_id()).finish()
    }
}
