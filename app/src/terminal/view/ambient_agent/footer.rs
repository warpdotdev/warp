use pathfinder_color::ColorU;
use warp_core::ui::appearance::Appearance;
use warpui::Element;
use warpui::elements::{
    Border, ConstrainedBox, Container, CrossAxisAlignment, Flex, MainAxisAlignment, MainAxisSize,
    ParentElement, Text,
};
use warpui::fonts::{Properties, Weight};

use crate::ui_components::blended_colors;

const CONTENT_SPACING: f32 = 4.;
const HORIZONTAL_PADDING: f32 = 12.;
const VERTICAL_PADDING: f32 = 8.;
const BORDER_WIDTH: f32 = 1.0;
const MAX_CONTENT_WIDTH: f32 = 400.;

/// Helper to build a centered two-line footer with common styling.
fn build_centered_footer(
    header_text: String,
    body_text: String,
    header_color: ColorU,
    body_color: ColorU,
    background: ColorU,
    border_color: ColorU,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let content = Flex::column()
        .with_main_axis_size(MainAxisSize::Min)
        .with_main_axis_alignment(MainAxisAlignment::Center)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(CONTENT_SPACING)
        .with_child(
            Text::new(
                header_text,
                appearance.ui_font_family(),
                appearance.ui_font_size() + 2.,
            )
            .with_style(Properties::default().weight(Weight::Bold))
            .with_color(header_color)
            .finish(),
        )
        .with_child(
            Text::new(
                body_text,
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(body_color)
            .finish(),
        )
        .finish();

    let content = ConstrainedBox::new(content)
        .with_max_width(MAX_CONTENT_WIDTH)
        .finish();

    // Use a row to horizontally center the content.
    let content = Flex::row()
        .with_child(content)
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::Center)
        .finish();

    Container::new(content)
        .with_background(background)
        .with_border(Border::top(BORDER_WIDTH).with_border_fill(border_color))
        .with_horizontal_padding(HORIZONTAL_PADDING)
        .with_vertical_padding(VERTICAL_PADDING)
        .finish()
}

/// Render a loading footer that replaces the terminal input while waiting to connect to an
// ambient agent session.
pub fn render_loading_footer(appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();

    let header_color = blended_colors::text_main(theme, theme.background());
    let body_color = blended_colors::text_disabled(theme, theme.background());
    let background = theme.surface_2().into();
    let border_color = blended_colors::neutral_4(theme);

    build_centered_footer(
        "Cloud agent starting up…".to_string(),
        "You'll be able to interact with Oz soon".to_string(),
        header_color,
        body_color,
        background,
        border_color,
        appearance,
    )
}

/// Render an error footer that shows when the ambient agent failed to spawn.
pub fn render_error_footer(error_message: &str, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();

    let header_color = theme.ui_error_color();
    let body_color = blended_colors::text_main(theme, theme.background());
    let background: ColorU = {
        let red: warp_core::ui::theme::Fill = theme.ui_error_color().into();
        red.with_opacity(50).into()
    };
    let border_color = theme.ui_error_color();

    build_centered_footer(
        "Agent failed".to_string(),
        error_message.to_string(),
        header_color,
        body_color,
        background,
        border_color,
        appearance,
    )
}
