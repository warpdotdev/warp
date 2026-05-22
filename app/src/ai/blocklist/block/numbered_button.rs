use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Border, Container, CornerRadius, CrossAxisAlignment, Expanded, Flex, FormattedTextElement,
    MainAxisSize, MouseStateHandle, ParentElement, Radius, Shrinkable, Text,
    DEFAULT_UI_LINE_HEIGHT_RATIO,
};
use warpui::ui_components::button::{Button, ButtonVariant};
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, SingletonEntity, ViewHandle};

use super::compact_agent_input::CompactAgentInput;
use crate::context_chips::spacing;

fn render_number_badge(
    number: usize,
    is_checked: bool,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let font_size = appearance.monospace_font_size();
    let (badge_background, badge_border_color) = if is_checked {
        (theme.accent(), theme.accent().into_solid())
    } else {
        (theme.surface_1(), internal_colors::neutral_4(theme))
    };
    Container::new(
        Text::new(
            format!("{number}"),
            appearance.monospace_font_family(),
            font_size.max(4.) - 1.,
        )
        .with_color(theme.foreground().into())
        .finish(),
    )
    .with_horizontal_padding(5.)
    .with_vertical_padding(1.)
    .with_border(Border::all(1.).with_border_color(badge_border_color))
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)))
    .with_background(badge_background)
    .finish()
}

pub(super) fn render_recommended_badge(appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    Container::new(
        Text::new(
            "Recommended".to_string(),
            appearance.ui_font_family(),
            appearance.monospace_font_size() - 2.,
        )
        .with_color(internal_colors::neutral_6(theme))
        .finish(),
    )
    .with_background(internal_colors::fg_overlay_2(theme))
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
    .with_vertical_padding(spacing::UDI_CHIP_VERTICAL_PADDING)
    .with_horizontal_padding(spacing::UDI_CHIP_HORIZONTAL_PADDING)
    .finish()
}

fn base_numbered_button(mouse_state: &MouseStateHandle, app: &AppContext) -> Button {
    let appearance = Appearance::as_ref(app);
    let font_size = appearance.monospace_font_size();
    appearance
        .ui_builder()
        .button(ButtonVariant::Secondary, mouse_state.clone())
        .with_style(UiComponentStyles {
            font_size: Some(font_size),
            ..UiComponentStyles::default()
        })
        .with_hovered_styles(UiComponentStyles {
            font_size: Some(font_size),
            ..UiComponentStyles::default()
        })
}

pub(super) fn build_numbered_button(
    number: usize,
    content: Box<dyn Element>,
    is_checked: bool,
    is_highlighted: bool,
    mouse_state: &MouseStateHandle,
    app: &AppContext,
) -> Button {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let mut button = base_numbered_button(mouse_state, app);

    if is_highlighted {
        button = button.with_style(UiComponentStyles {
            border_color: Some(theme.accent().into()),
            border_width: Some(1.0),
            background: Some(internal_colors::fg_overlay_2(theme).into()),
            ..UiComponentStyles::default()
        });
    } else if is_checked {
        button = button.with_style(UiComponentStyles {
            border_color: Some(theme.accent().into()),
            border_width: Some(1.0),
            background: Some(internal_colors::accent_overlay_1(theme).into()),
            ..UiComponentStyles::default()
        });
    }

    let badge = render_number_badge(number, is_checked, appearance);

    let row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_child(badge)
        .with_child(
            Shrinkable::new(1., Container::new(content).with_margin_left(8.).finish()).finish(),
        )
        .finish();

    button.with_custom_label(row)
}

pub(super) fn build_text_button_content(
    text_label: &str,
    recommended: bool,
    use_markdown: bool,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let font_size = appearance.monospace_font_size();
    let text_color = theme.foreground().into();

    let label_element = if let (true, Ok(formatted_text)) =
        (use_markdown, markdown_parser::parse_markdown(text_label))
    {
        FormattedTextElement::new(
            formatted_text,
            font_size,
            appearance.ui_font_family(),
            appearance.monospace_font_family(),
            text_color,
            Default::default(),
        )
        .with_line_height_ratio(DEFAULT_UI_LINE_HEIGHT_RATIO)
        .disable_mouse_interaction()
        .finish()
    } else {
        Text::new(
            text_label.to_string(),
            appearance.ui_font_family(),
            font_size,
        )
        .soft_wrap(true)
        .with_color(text_color)
        .finish()
    };

    if !recommended {
        return label_element;
    }

    Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_child(Expanded::new(1., label_element).finish())
        .with_child(
            Container::new(render_recommended_badge(appearance))
                .with_margin_left(12.)
                .finish(),
        )
        .finish()
}

pub(super) fn build_inline_input_content(
    input_view: &ViewHandle<CompactAgentInput>,
) -> Box<dyn Element> {
    warpui::presenter::ChildView::new(input_view).finish()
}
