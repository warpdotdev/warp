use warp_core::ui::color::contrast::MinimumAllowedContrast;
use warp_core::ui::color::ContrastingColor;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Border, Clipped, ConstrainedBox, Container, DispatchEventResult, DropTarget, Element,
    EventHandler, Flex, Hoverable, ParentElement, SavePosition, Stack,
};
use warpui::presenter::ChildView;
use warpui::{AppContext, SingletonEntity as _, ViewContext};

use super::common::{
    add_input_suggestions_overlays, wrap_input_with_terminal_padding_and_focus_handler,
};
use super::{
    Input, InputAction, InputDropTargetData, CLI_AGENT_RICH_INPUT_EDITOR_BOTTOM_PADDING,
    CLI_AGENT_RICH_INPUT_EDITOR_MAX_HEIGHT, CLI_AGENT_RICH_INPUT_EDITOR_TOP_PADDING,
    TERMINAL_VIEW_PADDING_LEFT,
};
use crate::appearance::Appearance;
use crate::context_chips::spacing;
use crate::editor::{EnterAction, EnterSettings, TextColors};
use crate::features::FeatureFlag;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::view::TerminalAction;

impl Input {
    /// Renders the CLI rich input (editor + CLI agent footer).
    pub(super) fn render_cli_agent_input(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let menu_positioning = self.menu_positioning(app);

        let mut stack = Stack::new().with_constrain_absolute_children();

        let input_box = Container::new(
            ConstrainedBox::new(Clipped::new(ChildView::new(&self.editor).finish()).finish())
                .with_max_height(CLI_AGENT_RICH_INPUT_EDITOR_MAX_HEIGHT)
                .finish(),
        )
        .with_padding_top(CLI_AGENT_RICH_INPUT_EDITOR_TOP_PADDING)
        .with_padding_right(*TERMINAL_VIEW_PADDING_LEFT)
        .with_padding_bottom(CLI_AGENT_RICH_INPUT_EDITOR_BOTTOM_PADDING)
        .finish();

        let input_editor_save_position_id = self.editor_save_position_id();
        let editor_element = SavePosition::new(
            EventHandler::new(input_box)
                .on_right_mouse_down(move |ctx, _, position| {
                    let input_rect = ctx
                        .element_position_by_id(input_editor_save_position_id.clone())
                        .expect("input editor position id should be saved");
                    let offset_position = position - input_rect.origin();
                    ctx.dispatch_typed_action(TerminalAction::OpenInputContextMenu {
                        position: offset_position,
                    });
                    DispatchEventResult::StopPropagation
                })
                .finish(),
            &self.editor_save_position_id(),
        )
        .finish();

        let mut column = Flex::column();

        // Render attachment chips (e.g. pasted screenshots) above the editor,
        // matching the pattern used by the agent view input in agent.rs.
        if FeatureFlag::ImageAsContext.is_enabled() {
            if let Some(images) = self.render_attachment_chips(appearance) {
                column.add_child(
                    Container::new(images)
                        .with_margin_top(spacing::UDI_CHIP_MARGIN)
                        .finish(),
                );
            }
        }

        column.add_child(editor_element);
        column.add_child(
            SavePosition::new(
                Container::new(ChildView::new(&self.agent_input_footer).finish())
                    .with_padding_right(*TERMINAL_VIEW_PADDING_LEFT)
                    .finish(),
                &self.prompt_save_position_id(),
            )
            .finish(),
        );

        stack.add_child(wrap_input_with_terminal_padding_and_focus_handler(
            self.is_active_session(app),
            column.finish(),
            false,
        ));

        if self.is_pane_focused(app) {
            add_input_suggestions_overlays(self, &mut stack, appearance, menu_positioning, app);
        }

        let mut input_container = Container::new(stack.finish()).with_border(
            Border::top(1.0).with_border_fill(internal_colors::fg_overlay_2(appearance.theme())),
        );

        // When an alt screen CLI agent (e.g. OpenCode) is running, match
        // the rich input background to the alt screen so it blends in.
        {
            let terminal_model = self.model.lock();
            if terminal_model.is_alt_screen_active() {
                if let Some(bg_color) = terminal_model.alt_screen().inferred_bg_color() {
                    input_container = input_container.with_background(bg_color);
                }
            }
        }

        let drop_target = DropTarget::new(
            input_container.finish(),
            InputDropTargetData::new(self.weak_view_handle.clone()),
        )
        .finish();

        let input = SavePosition::new(
            Hoverable::new(self.hoverable_handle.clone(), |_| drop_target)
                .on_hover(|is_hovered, ctx, _app, _position| {
                    ctx.dispatch_typed_action(InputAction::SetUDIHovered(is_hovered));
                })
                .on_middle_click(|ctx, _app, _position| {
                    ctx.dispatch_typed_action(TerminalAction::MiddleClickOnInput)
                })
                .finish(),
            &self.status_free_input_save_position_id(),
        )
        .finish();

        // Render inline menus (slash commands, prompts, skills) above the input,
        // matching the pattern used by the agent view input in agent.rs.
        // These must be outside the Hoverable so that mouse events on the menu
        // don't trigger SetUDIHovered, which would cause layout jitter.
        let mut outer_column = Flex::column();
        if self.suggestions_mode_model.as_ref(app).is_slash_commands() {
            outer_column.add_child(ChildView::new(&self.inline_slash_commands_view).finish());
        } else if self.suggestions_mode_model.as_ref(app).is_prompts_menu() {
            outer_column.add_child(ChildView::new(&self.inline_prompts_menu_view).finish());
        } else if self.suggestions_mode_model.as_ref(app).is_skill_menu() {
            outer_column.add_child(ChildView::new(&self.inline_skill_selector_view).finish());
        }
        outer_column.add_child(input);

        SavePosition::new(outer_column.finish(), &self.save_position_id()).finish()
    }

    /// Keep the rich input editor's text colors legible when it's rendered on
    /// top of an alt-screen CLI agent's inferred background (e.g. OpenCode),
    /// which does not respect the Warp theme. When no alt-screen-backed CLI
    /// agent rich input is active, restores the theme default text colors.
    ///
    /// This mirrors the contrast-adjustment pattern used for the use-agent
    /// toolbar button text (see `AgentFooterButtonTheme::text_color`) and the
    /// CLI agent brand icon in `AgentInputFooter::render_cli_mode_footer`.
    pub(super) fn update_cli_agent_editor_text_colors(&mut self, ctx: &mut ViewContext<Self>) {
        let appearance = Appearance::as_ref(ctx);
        let default_colors = TextColors::from_appearance(appearance);

        // Only override while the CLI agent rich input is actually open - the
        // same editor is reused for the normal terminal input and for other
        // modes (AI, shared sessions), and those shouldn't see the override.
        let rich_input_open =
            CLIAgentSessionsModel::as_ref(ctx).is_input_open(self.terminal_view_id);

        let alt_screen_bg = if rich_input_open {
            let terminal_model = self.model.lock();
            terminal_model
                .is_alt_screen_active()
                .then(|| terminal_model.alt_screen().inferred_bg_color())
                .flatten()
        } else {
            None
        };

        let text_colors = match alt_screen_bg {
            Some(bg) => TextColors {
                default_color: default_colors
                    .default_color
                    .on_background(bg.into(), MinimumAllowedContrast::Text),
                disabled_color: default_colors
                    .disabled_color
                    .on_background(bg.into(), MinimumAllowedContrast::Text),
                hint_color: default_colors
                    .hint_color
                    .on_background(bg.into(), MinimumAllowedContrast::Text),
            },
            None => default_colors,
        };

        self.editor.update(ctx, |editor, ctx| {
            editor.set_text_colors(text_colors, ctx);
        });
    }

    /// Configures the editor's enter-key behaviour for the CLI agent rich input.
    ///
    /// When rich input is **open**, `enter` is always `Emit` so `input_enter`
    /// runs first and handles inline-menu acceptance before any newline or
    /// submit logic.  `ctrl_enter` is `Emit` only when the toggle is ON
    /// (submit on Ctrl+Enter); when the toggle is OFF it is
    /// `InsertNewLineIfMultiLine` to restore baseline newline insertion.
    ///
    /// When rich input is **closed**, `EnterSettings::default()` is restored.
    pub(super) fn update_cli_agent_enter_settings(&mut self, ctx: &mut ViewContext<Self>) {
        let rich_input_open =
            CLIAgentSessionsModel::as_ref(ctx).is_input_open(self.terminal_view_id);

        let settings = if rich_input_open {
            let submit_on_ctrl_enter =
                *crate::settings::AISettings::as_ref(ctx).submit_on_ctrl_enter;
            EnterSettings {
                // Always Emit so input_enter handles menus before submit/newline.
                enter: EnterAction::Emit,
                // Toggle ON  → Emit (submit path in input_ctrl_enter).
                // Toggle OFF → InsertNewLineIfMultiLine (baseline newline).
                ctrl_enter: if submit_on_ctrl_enter {
                    EnterAction::Emit
                } else {
                    EnterAction::InsertNewLineIfMultiLine
                },
                ..Default::default()
            }
        } else {
            EnterSettings::default()
        };

        self.editor.update(ctx, |editor, _ctx| {
            editor.set_enter_settings(settings);
        });
    }
}
