use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::color::blend::Blend;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Align, Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Dismiss,
    DropShadow, Element, Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    ParentElement, Radius, Shrinkable, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::keymap::{FixedBinding, Keystroke};
use warpui::platform::Cursor;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use crate::appearance::Appearance;
use crate::ui_components::icons::Icon;

// Figma node 6583:23542
const MODAL_WIDTH: f32 = 441.;
const DIALOG_CORNER_RADIUS: f32 = 8.;

const HEADER_PADDING_TOP: f32 = 16.;
const HEADER_PADDING_BOTTOM: f32 = 16.;
const HEADER_PADDING_HORIZONTAL: f32 = 24.;

const BODY_PADDING_VERTICAL: f32 = 16.;
const BODY_PADDING_HORIZONTAL: f32 = 20.;

const OPTION_PADDING_VERTICAL: f32 = 8.;
const OPTION_PADDING_HORIZONTAL: f32 = 12.;
const OPTION_CORNER_RADIUS: f32 = 4.;
const OPTION_GAP: f32 = 12.;
const OPTIONS_VERTICAL_GAP: f32 = 8.;

const AVATAR_SIZE: f32 = 48.;
const AVATAR_ICON_SIZE: f32 = 24.;

const TITLE_FONT_SIZE: f32 = 16.;
const OPTION_TITLE_FONT_SIZE: f32 = 14.;
const OPTION_DESC_FONT_SIZE: f32 = 12.;

/// The mode for environment setup selected by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentSetupMode {
    /// Use remote GitHub repositories (form-based flow).
    RemoteGitHub,
    /// Use local repositories (agent-based flow).
    LocalRepositories,
}

#[derive(Debug, Clone)]
pub enum EnvironmentSetupModeSelectorAction {
    SelectRemoteGitHub,
    SelectLocalRepositories,
    Dismiss,
    HoveredIn(usize),
    ArrowUp,
    ArrowDown,
    Enter,
}

#[derive(Debug)]
pub enum EnvironmentSetupModeSelectorEvent {
    Selected(EnvironmentSetupMode),
    Dismissed,
}

pub struct EnvironmentSetupModeSelector {
    close_button_mouse_state: MouseStateHandle,
    remote_github_mouse_state: MouseStateHandle,
    local_repos_mouse_state: MouseStateHandle,
    selected_option_index: usize,
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings(vec![
        FixedBinding::new(
            "escape",
            EnvironmentSetupModeSelectorAction::Dismiss,
            id!("EnvironmentSetupModeSelector"),
        ),
        FixedBinding::new(
            "enter",
            EnvironmentSetupModeSelectorAction::Enter,
            id!("EnvironmentSetupModeSelector"),
        ),
        FixedBinding::new(
            "numpadenter",
            EnvironmentSetupModeSelectorAction::Enter,
            id!("EnvironmentSetupModeSelector"),
        ),
        FixedBinding::new(
            "up",
            EnvironmentSetupModeSelectorAction::ArrowUp,
            id!("EnvironmentSetupModeSelector"),
        ),
        FixedBinding::new(
            "down",
            EnvironmentSetupModeSelectorAction::ArrowDown,
            id!("EnvironmentSetupModeSelector"),
        ),
        FixedBinding::new(
            "tab",
            EnvironmentSetupModeSelectorAction::ArrowDown,
            id!("EnvironmentSetupModeSelector"),
        ),
        FixedBinding::new(
            "shift-tab",
            EnvironmentSetupModeSelectorAction::ArrowUp,
            id!("EnvironmentSetupModeSelector"),
        ),
    ]);
}

impl EnvironmentSetupModeSelector {
    pub fn new(_ctx: &mut ViewContext<Self>) -> Self {
        Self {
            close_button_mouse_state: MouseStateHandle::default(),
            remote_github_mouse_state: MouseStateHandle::default(),
            local_repos_mouse_state: MouseStateHandle::default(),
            selected_option_index: 0,
        }
    }

    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let title = Text::new(
            "Choose how you'd like to set up your environment".to_string(),
            appearance.ui_font_family(),
            TITLE_FONT_SIZE,
        )
        .with_style(Properties::default().weight(Weight::Bold))
        .with_color(theme.active_ui_text_color().into())
        .soft_wrap(true)
        .finish();

        let close_button = appearance
            .ui_builder()
            .close_button(16., self.close_button_mouse_state.clone())
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(EnvironmentSetupModeSelectorAction::Dismiss);
            })
            .finish();

        let esc_keystroke = Keystroke::parse("escape").expect("escape keystroke parses");
        let esc_pill = appearance
            .ui_builder()
            .keyboard_shortcut(&esc_keystroke)
            .with_style(UiComponentStyles {
                font_size: Some(OPTION_DESC_FONT_SIZE),
                font_color: Some(theme.nonactive_ui_text_color().into_solid()),
                border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
                padding: Some(Coords {
                    top: 0.,
                    bottom: 0.,
                    left: 3.,
                    right: 3.,
                }),
                height: Some(16.),
                ..Default::default()
            })
            .build()
            .finish();

        let right_controls = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Container::new(esc_pill).with_margin_right(8.).finish())
            .with_child(close_button)
            .finish();

        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(12.)
            .with_child(Shrinkable::new(1., title).finish())
            .with_child(right_controls)
            .finish()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_option(
        &self,
        index: usize,
        icon: Icon,
        title: &'static str,
        description: &'static str,
        is_suggested: bool,
        mouse_state: MouseStateHandle,
        action: EnvironmentSetupModeSelectorAction,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        let font_family = appearance.ui_font_family();
        let active_text = theme.active_ui_text_color();
        let nonactive_text = theme.nonactive_ui_text_color();

        let base_background = theme.surface_2();
        let hover_background = base_background.blend(&internal_colors::accent_overlay_1(theme));

        let base_border = internal_colors::neutral_4(theme);
        let hover_border = theme.accent().into_solid();

        // Avatar styling - lighter background for the circular icon container
        let avatar_background = internal_colors::neutral_2(theme);
        let avatar_border = internal_colors::neutral_3(theme);

        // Badge styling - lighter colors
        let badge_background = internal_colors::neutral_2(theme);
        let badge_border = internal_colors::neutral_3(theme);
        let badge_text_color = internal_colors::neutral_5(theme);

        let icon_color = nonactive_text;

        let is_selected = self.selected_option_index == index;
        let action = action.clone();
        Hoverable::new(mouse_state, move |state| {
            let is_hovered = state.is_hovered() || state.is_clicked();
            let (background, border_color) = if is_hovered || is_selected {
                (hover_background, hover_border)
            } else {
                (base_background, base_border)
            };

            let avatar_icon = ConstrainedBox::new(icon.to_warpui_icon(icon_color).finish())
                .with_width(AVATAR_ICON_SIZE)
                .with_height(AVATAR_ICON_SIZE)
                .finish();

            let avatar_contents = ConstrainedBox::new(
                Flex::row()
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_child(avatar_icon)
                    .finish(),
            )
            .with_width(AVATAR_SIZE)
            .with_height(AVATAR_SIZE)
            .finish();

            let avatar = Container::new(avatar_contents)
                .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                .with_background(avatar_background)
                .with_border(Border::all(1.).with_border_color(avatar_border))
                .finish();

            let title_text = Text::new(title.to_string(), font_family, OPTION_TITLE_FONT_SIZE)
                .with_style(Properties::default().weight(Weight::Semibold))
                .with_color(active_text.into())
                .finish();

            let mut title_row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(4.)
                .with_child(title_text);

            if is_suggested {
                let suggested_text =
                    Text::new("Suggested".to_string(), font_family, OPTION_DESC_FONT_SIZE)
                        .with_style(Properties::default().weight(Weight::Medium))
                        .with_color(badge_text_color)
                        .finish();

                let suggested = Container::new(suggested_text)
                    .with_horizontal_padding(8.)
                    .with_vertical_padding(2.)
                    .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                    .with_background(badge_background)
                    .with_border(Border::all(1.).with_border_color(badge_border))
                    .finish();

                title_row.add_child(Container::new(suggested).with_margin_left(8.).finish());
            }

            let description_text =
                Text::new(description.to_string(), font_family, OPTION_DESC_FONT_SIZE)
                    .with_style(Properties::default().weight(Weight::Normal))
                    .with_color(nonactive_text.into())
                    .soft_wrap(true)
                    .finish();

            let text_content = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(title_row.finish())
                .with_child(
                    Container::new(description_text)
                        .with_margin_top(4.)
                        .finish(),
                )
                .finish();

            Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(OPTION_GAP)
                    .with_child(avatar)
                    .with_child(Shrinkable::new(1., text_content).finish())
                    .finish(),
            )
            .with_padding_left(OPTION_PADDING_HORIZONTAL)
            .with_padding_right(OPTION_PADDING_HORIZONTAL)
            .with_padding_top(OPTION_PADDING_VERTICAL)
            .with_padding_bottom(OPTION_PADDING_VERTICAL)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(OPTION_CORNER_RADIUS)))
            .with_border(Border::all(1.).with_border_color(border_color))
            .with_background(background)
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .additional_on_hover(move |is_hovered, ctx, _app, _pos| {
            if is_hovered {
                ctx.dispatch_typed_action(EnvironmentSetupModeSelectorAction::HoveredIn(index));
            }
        })
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(action.clone());
        })
        .finish()
    }

    fn render_modal(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let header = Container::new(self.render_header(appearance))
            .with_padding_top(HEADER_PADDING_TOP)
            .with_padding_bottom(HEADER_PADDING_BOTTOM)
            .with_padding_left(HEADER_PADDING_HORIZONTAL)
            .with_padding_right(HEADER_PADDING_HORIZONTAL)
            .finish();

        let remote_github_option = self.render_option(
            0,
            Icon::Github,
            "Quick setup",
            "Select the GitHub repositories you'd like to work with and we'll suggest a base image and config",
            true,
            self.remote_github_mouse_state.clone(),
            EnvironmentSetupModeSelectorAction::SelectRemoteGitHub,
            appearance,
        );

        let local_repos_option = self.render_option(
            1,
            Icon::Terminal,
            "Use the agent",
            "Choose a locally set up project and we'll help you set up an environment based on it",
            false,
            self.local_repos_mouse_state.clone(),
            EnvironmentSetupModeSelectorAction::SelectLocalRepositories,
            appearance,
        );

        let options = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(OPTIONS_VERTICAL_GAP)
            .with_child(remote_github_option)
            .with_child(local_repos_option)
            .finish();

        let body = Container::new(options)
            .with_padding_top(BODY_PADDING_VERTICAL)
            .with_padding_bottom(BODY_PADDING_VERTICAL)
            .with_padding_left(BODY_PADDING_HORIZONTAL)
            .with_padding_right(BODY_PADDING_HORIZONTAL)
            .finish();

        let dialog_contents = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header)
            .with_child(body)
            .finish();

        let dialog_background = theme
            .surface_1()
            .blend(&internal_colors::fg_overlay_1(theme));
        let dialog_border = internal_colors::neutral_4(theme);

        let dialog = Container::new(dialog_contents)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(DIALOG_CORNER_RADIUS)))
            .with_border(Border::all(1.).with_border_color(dialog_border))
            .with_background(dialog_background)
            .with_drop_shadow(DropShadow {
                color: ColorU::new(0, 0, 0, 77),
                offset: vec2f(0., 7.),
                blur_radius: 7.,
                spread_radius: 0.,
            })
            .finish();

        let constrained_dialog = ConstrainedBox::new(dialog).with_width(MODAL_WIDTH).finish();

        let dismiss_dialog = Dismiss::new(constrained_dialog)
            .prevent_interaction_with_other_elements()
            .on_dismiss(|ctx, _app| {
                ctx.dispatch_typed_action(EnvironmentSetupModeSelectorAction::Dismiss);
            })
            .finish();

        Container::new(Align::new(dismiss_dialog).finish())
            .with_background(appearance.theme().dark_overlay())
            .finish()
    }
}

impl Entity for EnvironmentSetupModeSelector {
    type Event = EnvironmentSetupModeSelectorEvent;
}

impl TypedActionView for EnvironmentSetupModeSelector {
    type Action = EnvironmentSetupModeSelectorAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            EnvironmentSetupModeSelectorAction::SelectRemoteGitHub => {
                ctx.emit(EnvironmentSetupModeSelectorEvent::Selected(
                    EnvironmentSetupMode::RemoteGitHub,
                ));
                ctx.notify();
            }
            EnvironmentSetupModeSelectorAction::SelectLocalRepositories => {
                ctx.emit(EnvironmentSetupModeSelectorEvent::Selected(
                    EnvironmentSetupMode::LocalRepositories,
                ));
                ctx.notify();
            }
            EnvironmentSetupModeSelectorAction::Dismiss => {
                ctx.emit(EnvironmentSetupModeSelectorEvent::Dismissed);
                ctx.notify();
            }
            EnvironmentSetupModeSelectorAction::HoveredIn(index) => {
                self.selected_option_index = *index;
                ctx.notify();
            }
            EnvironmentSetupModeSelectorAction::ArrowUp => {
                self.selected_option_index = if self.selected_option_index == 0 {
                    1
                } else {
                    0
                };
                ctx.notify();
            }
            EnvironmentSetupModeSelectorAction::ArrowDown => {
                self.selected_option_index = if self.selected_option_index == 0 {
                    1
                } else {
                    0
                };
                ctx.notify();
            }
            EnvironmentSetupModeSelectorAction::Enter => {
                match self.selected_option_index {
                    0 => {
                        ctx.emit(EnvironmentSetupModeSelectorEvent::Selected(
                            EnvironmentSetupMode::RemoteGitHub,
                        ));
                    }
                    1 => {
                        ctx.emit(EnvironmentSetupModeSelectorEvent::Selected(
                            EnvironmentSetupMode::LocalRepositories,
                        ));
                    }
                    _ => {}
                }
                ctx.notify();
            }
        }
    }
}

impl View for EnvironmentSetupModeSelector {
    fn ui_name() -> &'static str {
        "EnvironmentSetupModeSelector"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.render_modal(app)
    }
}
