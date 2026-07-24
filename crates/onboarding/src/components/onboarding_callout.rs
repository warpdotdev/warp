use std::borrow::Cow;

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use ui_components::button::Button as ButtonComponent;
use ui_components::{Component, MouseEventHandler, Options as _, button};
use warp_core::ui::appearance::Appearance;
use warp_core::ui::color::coloru_with_opacity;
use warp_core::ui::color::contrast::relative_luminance;
use warp_core::ui::theme::Fill;
use warp_core::ui::theme::phenomenon::PhenomenonStyle;
use warpui_core::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DropShadow, Flex,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius, Rect,
};
use warpui_core::fonts::Weight;
use warpui_core::keymap::Keystroke;
use warpui_core::prelude::*;
use warpui_core::ui_components::checkbox::Checkbox as WarpCheckbox;
use warpui_core::ui_components::components::{UiComponent as _, UiComponentStyles};

const CALLOUT_WIDTH: f32 = 480.;
const CALLOUT_BORDER_WIDTH: f32 = 1.;
const CALLOUT_CORNER_RADIUS: f32 = 8.;
const CALLOUT_PADDING: f32 = 16.;

struct PhenomenonPrimaryButtonTheme;
struct PhenomenonSecondaryButtonTheme;

impl button::Theme for PhenomenonPrimaryButtonTheme {
    fn background(&self, button_state: button::State, _appearance: &Appearance) -> Option<Fill> {
        let hovered = matches!(
            button_state,
            button::State::Hovered | button::State::Pressed
        );
        Some(PhenomenonStyle::primary_button_background(hovered))
    }

    fn text_color(&self, _background: Option<Fill>, _appearance: &Appearance) -> ColorU {
        PhenomenonStyle::primary_button_text()
    }

    fn keyboard_shortcut_border(&self, text_color: ColorU, _: &Appearance) -> Option<ColorU> {
        Some(coloru_with_opacity(text_color, 60))
    }
}

impl button::Theme for PhenomenonSecondaryButtonTheme {
    fn background(&self, button_state: button::State, _appearance: &Appearance) -> Option<Fill> {
        match button_state {
            button::State::Default => None,
            button::State::Hovered | button::State::Pressed => {
                Some(PhenomenonStyle::segmented_control_background())
            }
        }
    }

    fn text_color(&self, _background: Option<Fill>, _appearance: &Appearance) -> ColorU {
        PhenomenonStyle::foreground()
    }

    fn border(&self, _appearance: &Appearance) -> Option<ColorU> {
        Some(PhenomenonStyle::subtle_border())
    }

    fn keyboard_shortcut_border(&self, text_color: ColorU, _: &Appearance) -> Option<ColorU> {
        Some(coloru_with_opacity(text_color, 60))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StepStatus {
    pub current_step: u8,
    pub total_steps: u8,
}

impl StepStatus {
    pub fn new(current_step: u8, total_steps: u8) -> Self {
        Self {
            current_step,
            total_steps,
        }
    }
}

pub struct Button {
    pub text: Cow<'static, str>,
    pub keystroke: Option<Keystroke>,
    pub handler: MouseEventHandler,
}

impl Button {
    pub fn next(handler: MouseEventHandler) -> Self {
        Self {
            text: Cow::Borrowed("Next"),
            keystroke: Some(Keystroke {
                key: "enter".into(),
                ..Default::default()
            }),
            handler,
        }
    }
}

/// A checkbox with a label and click handler.
pub struct Checkbox {
    pub label: Cow<'static, str>,
    pub checked: bool,
    pub handler: MouseEventHandler,
}

#[derive(Default)]
pub struct OnboardingCallout {
    right_button: ButtonComponent,
    left_button: ButtonComponent,
    checkbox_mouse_state: MouseStateHandle,
}

pub struct Params {
    /// The title of the callout.
    pub title: Cow<'static, str>,
    /// The body text of the callout.
    pub text: Cow<'static, str>,
    /// Current step and total steps.
    pub step: StepStatus,
    pub right_button: Button,
    /// Optional configuration.
    pub options: Options,
}

impl ui_components::Params for Params {
    type Options<'a> = Options;
}

pub struct Options {
    /// Optional left button, typically "Skip".
    pub left_button: Option<Button>,
    /// Optional checkbox for toggling settings.
    pub checkbox: Option<Checkbox>,
}

impl ui_components::Options for Options {
    fn default(_appearance: &Appearance) -> Self {
        Self {
            left_button: None,
            checkbox: None,
        }
    }
}

impl Component for OnboardingCallout {
    type Params<'a> = Params;

    fn render<'a>(&self, appearance: &Appearance, params: Self::Params<'a>) -> Box<dyn Element> {
        let Params {
            title,
            text,
            step,
            right_button,
            options,
        } = params;

        let header = self.render_header(appearance, &title);
        let body = self.render_body(appearance, &text);
        // Take checkbox out before passing options to render_actions
        let mut options = options;
        let checkbox = options
            .checkbox
            .take()
            .map(|cb| self.render_checkbox(appearance, cb));
        let actions = self.render_actions(step, right_button, options, appearance);

        let mut content = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(12.);

        content.add_child(header);
        content.add_child(body);
        if let Some(checkbox_element) = checkbox {
            content.add_child(checkbox_element);
        }
        content.add_child(actions);

        let content = content.finish();

        let background = PhenomenonStyle::tinted_surface();
        let border_color = Fill::Solid(PhenomenonStyle::surface_border());

        // Use lighter shadow on dark themes, darker shadow on light themes
        let background_luminance = relative_luminance(appearance.theme().background().into_solid());
        let is_light_theme = background_luminance > 0.2;
        let shadow_opacity = if is_light_theme { 20 } else { 35 };

        let shadow = DropShadow {
            color: coloru_with_opacity(ColorU::black(), shadow_opacity),
            offset: vec2f(0., 10.),
            blur_radius: 20.,
            spread_radius: 0.,
        };

        ConstrainedBox::new(
            Container::new(content)
                .with_uniform_padding(CALLOUT_PADDING)
                .with_background(background)
                .with_border(Border::all(CALLOUT_BORDER_WIDTH).with_border_fill(border_color))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                    CALLOUT_CORNER_RADIUS,
                )))
                .with_drop_shadow(shadow)
                .finish(),
        )
        .with_width(CALLOUT_WIDTH)
        .finish()
    }
}

impl OnboardingCallout {
    fn render_header(&self, appearance: &Appearance, title: &str) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .paragraph(title.to_string())
            .with_style(UiComponentStyles {
                font_color: Some(PhenomenonStyle::foreground()),
                font_size: Some(16.),
                font_weight: Some(Weight::Bold),
                ..Default::default()
            })
            .build()
            .finish()
    }

    fn render_body(&self, appearance: &Appearance, text: &str) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .paragraph(text.to_string())
            .with_style(UiComponentStyles {
                font_color: Some(PhenomenonStyle::body_text()),
                font_size: Some(13.),
                ..Default::default()
            })
            .build()
            .finish()
    }

    fn render_checkbox(&self, appearance: &Appearance, checkbox: Checkbox) -> Box<dyn Element> {
        let checkbox_size = Some(12.);
        let corner_radius = CornerRadius::with_all(Radius::Pixels(2.));
        let foreground_color = PhenomenonStyle::foreground();
        let subtle_border = Fill::Solid(PhenomenonStyle::subtle_border());

        let checkbox_element = WarpCheckbox::new(
            self.checkbox_mouse_state.clone(),
            UiComponentStyles {
                font_size: checkbox_size,
                border_color: Some(Fill::Solid(foreground_color).into()),
                font_color: Some(foreground_color),
                border_width: Some(1.),
                border_radius: Some(corner_radius),
                ..Default::default()
            },
            None,
            Some(UiComponentStyles {
                font_size: checkbox_size,
                background: Some(Fill::Solid(foreground_color).into()),
                border_color: Some(Fill::Solid(foreground_color).into()),
                font_color: Some(PhenomenonStyle::background()),
                border_radius: Some(corner_radius),
                ..Default::default()
            }),
            Some(UiComponentStyles {
                font_size: checkbox_size,
                border_color: Some(subtle_border.into()),
                font_color: Some(PhenomenonStyle::subtle_border()),
                border_width: Some(1.),
                border_radius: Some(corner_radius),
                ..Default::default()
            }),
        )
        .check(checkbox.checked)
        .build()
        .on_click(checkbox.handler)
        .finish();

        let label = appearance
            .ui_builder()
            .paragraph(checkbox.label.to_string())
            .with_style(UiComponentStyles {
                font_color: Some(PhenomenonStyle::label_text()),
                font_size: Some(12.),
                ..Default::default()
            })
            .build()
            .finish();

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.)
            .with_child(checkbox_element)
            .with_child(label)
            .finish()
    }

    fn render_status_dots(&self, step: StepStatus) -> Box<dyn Element> {
        const DOT_SIZE: f32 = 8.;
        const DOT_SPACING: f32 = 4.;

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(DOT_SPACING);

        for i in 0..step.total_steps {
            let is_current = i == step.current_step;
            let dot_color = if is_current {
                PhenomenonStyle::surface_border()
            } else {
                PhenomenonStyle::subtle_border()
            };

            let dot = ConstrainedBox::new(
                Rect::new()
                    .with_background_color(dot_color)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(DOT_SIZE / 2.)))
                    .finish(),
            )
            .with_width(DOT_SIZE)
            .with_height(DOT_SIZE)
            .finish();

            row.add_child(dot);
        }

        row.finish()
    }

    fn render_actions(
        &self,
        step: StepStatus,
        right_button: Button,
        mut options: Options,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let right_button_text = right_button.text.clone();

        let right_button = self.right_button.render(
            appearance,
            button::Params {
                content: button::Content::Label(right_button_text),
                theme: &PhenomenonPrimaryButtonTheme,
                options: button::Options {
                    on_click: Some(right_button.handler),
                    keystroke: right_button.keystroke.clone(),
                    ..button::Options::default(appearance)
                },
            },
        );

        let status_dots = self.render_status_dots(step);

        let mut buttons_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_main_axis_size(MainAxisSize::Min);

        if let Some(left_button) = options.left_button.take() {
            let left_button_text = left_button.text.clone();

            let left_button_element = self.left_button.render(
                appearance,
                button::Params {
                    content: button::Content::Label(left_button_text),
                    theme: &PhenomenonSecondaryButtonTheme,
                    options: button::Options {
                        on_click: Some(left_button.handler),
                        keystroke: left_button.keystroke.clone(),
                        ..button::Options::default(appearance)
                    },
                },
            );

            buttons_row.add_child(
                Container::new(left_button_element)
                    .with_margin_right(8.)
                    .finish(),
            );
        }

        buttons_row.add_child(right_button);

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(status_dots)
            .with_child(buttons_row.finish())
            .finish()
    }
}
