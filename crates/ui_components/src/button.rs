mod params;
pub mod themes;

pub use params::*;
pub use themes::Theme;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::color::ContrastingColor as _;
use warp_core::ui::color::contrast::MinimumAllowedContrast;
use warpui_core::elements::{MouseState, MouseStateHandle};
use warpui_core::prelude::*;

use crate::{keyboard_shortcut, tooltip};

#[derive(Default)]
pub struct Button {
    mouse_state: MouseStateHandle,
    tooltip: tooltip::Tooltip,
}

impl crate::Component for Button {
    type Params<'a> = Params<'a>;

    fn render<'a>(&self, appearance: &Appearance, params: Self::Params<'a>) -> Box<dyn Element> {
        let theme: &dyn Theme = if params.options.disabled {
            &themes::Disabled
        } else {
            params.theme
        };

        let mut hoverable = Hoverable::new(self.mouse_state.clone(), |mouse_state| {
            let size = params.options.size;
            let is_icon_button = matches!(params.content, Content::Icon(_));

            let background = theme.background(mouse_state.into(), appearance);
            let mut text_color = theme.text_color(background, appearance);

            // Ensures that the action button text is always rendered with sufficient contrast.
            // For hovered states that use a semi-transparent background, we apply the contrast adjustment using the base background.
            if let Some(base_bg) = theme.background(State::Default, appearance) {
                text_color =
                    text_color.on_background(base_bg.into_solid(), MinimumAllowedContrast::Text);
            }

            let mut row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_spacing(size.inner_spacing());

            // Add icon, if any.
            match &params.content {
                Content::Icon(icon) | Content::IconAndLabel(icon, _) => {
                    let icon_size = size.icon_size();
                    row.add_child(
                        ConstrainedBox::new(icon.to_warpui_icon(text_color.into()).finish())
                            .with_width(icon_size)
                            .with_height(icon_size)
                            .finish(),
                    );
                }
                Content::Label(_) => {}
            }

            // Add label, if any.
            match params.content {
                Content::Label(label) | Content::IconAndLabel(_, label) => {
                    let font_size = size.font_size();
                    let font_properties = size.font_properties();

                    row.add_child(
                        Text::new_inline(label, appearance.ui_font_family(), font_size)
                            .with_color(text_color)
                            .with_style(font_properties)
                            .with_selectable(false)
                            .finish(),
                    );
                }
                Content::Icon(_) => {}
            }

            // Add keystroke, if any.
            if let Some(keystroke) = params.options.keystroke {
                let sizing = size.keyboard_shortcut_sizing();
                row.add_child(
                    Container::new(
                        keyboard_shortcut::KeyboardShortcut.render(
                            appearance,
                            keyboard_shortcut::Params {
                                keystroke,
                                options: keyboard_shortcut::Options {
                                    font_color: Some(text_color),
                                    background: theme
                                        .keyboard_shortcut_background(appearance)
                                        .map(Into::into),
                                    border_fill: theme
                                        .keyboard_shortcut_border(text_color, appearance)
                                        .map(Into::into),
                                    sizing,
                                },
                            },
                        ),
                    )
                    .with_margin_left(2.)
                    .finish(),
                );
            }

            let mut button = Container::new(row.finish())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
            if let Some(background) = background {
                button = button.with_background(background);
            }
            if let Some(border) = theme.border(appearance) {
                button = button.with_border(Border::all(1.).with_border_color(border));
            }
            if is_icon_button {
                // Make the button evenly padded when there is no label.
                let vertical_padding = (size.height() - size.icon_size()) / 2.;
                button = button.with_horizontal_padding(vertical_padding);
            } else {
                button = button.with_horizontal_padding(size.horizontal_padding());
            }

            // Constrain the button sizing.
            //
            // It is important that this is done after styling changes are applied above,
            // otherwise the presence of a border or padding will affect the final size.
            let height = size.height();
            let mut button = ConstrainedBox::new(button.finish()).with_height(height);
            // If the content is an icon, make it square.
            if is_icon_button {
                button = button.with_width(height);
            }

            let mut stack = stack::Stack::new().with_child(button.finish());
            if mouse_state.is_hovered()
                && let Some(tooltip) = params.options.tooltip
            {
                stack.add_positioned_overlay_child(
                    self.tooltip.render(appearance, tooltip.params),
                    stack::OffsetPositioning::offset_from_parent(
                        vec2f(0., -4.),
                        stack::ParentOffsetBounds::WindowByPosition,
                        tooltip.alignment.parent_anchor(),
                        tooltip.alignment.child_anchor(),
                    ),
                );
            }
            stack.finish()
        });

        if !params.options.disabled
            && let Some(on_click) = params.options.on_click
        {
            hoverable = hoverable
                .with_cursor(Cursor::PointingHand)
                .on_click(on_click);
        }

        hoverable.finish()
    }
}

/// The current state of the button.
pub enum State {
    Default,
    Hovered,
    Pressed,
}

impl From<&MouseState> for State {
    fn from(mouse_state: &MouseState) -> Self {
        if mouse_state.is_clicked() {
            return Self::Pressed;
        }
        if mouse_state.is_hovered() {
            return Self::Hovered;
        }
        Self::Default
    }
}
