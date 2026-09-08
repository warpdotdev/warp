use pathfinder_geometry::vector::Vector2F;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::Fill;
use warp_core::ui::theme::color::internal_colors;
use warpui_core::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex,
    FormattedTextElement, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    ParentElement, Radius, Shrinkable, Text, Wrap,
};
use warpui_core::fonts::Weight;
use warpui_core::platform::Cursor;
use warpui_core::prelude::Align;
use warpui_core::presenter::EventContext;
use warpui_core::text_layout::TextAlignment;
use warpui_core::{AppContext, Element};

pub(super) type ClickCallback = Box<dyn FnMut(&mut EventContext, &AppContext, Vector2F) + 'static>;
pub(super) type HoverCallback =
    Box<dyn FnMut(bool, &mut EventContext, &AppContext, Vector2F) + 'static>;

pub(super) struct ChipSpec {
    pub label: &'static str,
    pub is_enabled: bool,
    pub mouse_state: MouseStateHandle,
    pub on_click: ClickCallback,
    pub on_hover: Option<HoverCallback>,
}

pub(super) struct ToggleCardSpec {
    pub title: &'static str,
    pub is_expanded: bool,
    pub is_left_selected: bool,
    pub left_label: &'static str,
    pub right_label: &'static str,
    pub card_mouse_state: MouseStateHandle,
    pub on_expand: ClickCallback,
    pub left_mouse: MouseStateHandle,
    pub right_mouse: MouseStateHandle,
    pub on_left: ClickCallback,
    pub on_right: ClickCallback,
    pub chips: Vec<ChipSpec>,
}

pub(super) fn render_toggle_card(
    appearance: &Appearance,
    spec: ToggleCardSpec,
) -> Box<dyn Element> {
    if spec.is_expanded {
        render_expanded(appearance, spec)
    } else {
        render_collapsed(appearance, spec)
    }
}

fn collapsed_subtitle(
    is_enabled: bool,
    left_label: &str,
    right_label: &str,
    chips: &[ChipSpec],
) -> String {
    if !is_enabled {
        return right_label.to_string();
    }
    if chips.is_empty() {
        return left_label.to_string();
    }
    let enabled_labels: Vec<&str> = chips
        .iter()
        .filter(|c| c.is_enabled)
        .map(|c| c.label)
        .collect();
    if enabled_labels.is_empty() {
        return left_label.to_string();
    }
    let joined = enabled_labels.join(", ");
    let mut chars = joined.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

fn render_collapsed(appearance: &Appearance, spec: ToggleCardSpec) -> Box<dyn Element> {
    let theme = appearance.theme();
    let ui_font_family = appearance.ui_font_family();
    let text_color = internal_colors::text_sub(theme, theme.background().into_solid());
    let border_color = Fill::Solid(internal_colors::neutral_4(theme));
    let subtitle = collapsed_subtitle(
        spec.is_left_selected,
        spec.left_label,
        spec.right_label,
        &spec.chips,
    );
    let mut on_expand = spec.on_expand;

    Hoverable::new(spec.card_mouse_state, move |_| {
        let title_el = FormattedTextElement::from_str(spec.title, ui_font_family, 16.)
            .with_color(text_color)
            .with_weight(Weight::Normal)
            .with_alignment(TextAlignment::Left)
            .with_line_height_ratio(1.0)
            .finish();

        let sub_el = Text::new(subtitle.clone(), ui_font_family, 12.)
            .with_color(text_color)
            .with_line_height_ratio(1.0)
            .finish();

        let content = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(title_el)
            .with_child(Container::new(sub_el).with_margin_top(12.).finish())
            .finish();

        Container::new(content)
            .with_uniform_padding(24.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_border(Border::all(1.).with_border_fill(border_color))
            .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, app, pos| {
        on_expand(ctx, app, pos);
    })
    .finish()
}

fn render_expanded(appearance: &Appearance, spec: ToggleCardSpec) -> Box<dyn Element> {
    let theme = appearance.theme();
    let ui_font_family = appearance.ui_font_family();
    let text_color = internal_colors::text_main(theme, theme.background().into_solid());
    let border_color = theme.accent();
    let background = internal_colors::accent_overlay_1(theme);

    let title_el = FormattedTextElement::from_str(spec.title, ui_font_family, 16.)
        .with_color(text_color)
        .with_weight(Weight::Normal)
        .with_alignment(TextAlignment::Left)
        .with_line_height_ratio(1.0)
        .finish();

    let seg_control = render_inline_segmented_control(
        appearance,
        spec.is_left_selected,
        spec.left_label,
        spec.right_label,
        spec.left_mouse,
        spec.right_mouse,
        spec.on_left,
        spec.on_right,
    );

    let mut content = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(title_el)
        .with_child(Container::new(seg_control).with_margin_top(12.).finish());

    if !spec.chips.is_empty() {
        let chips_el = render_chips(appearance, spec.chips);
        content = content.with_child(Container::new(chips_el).with_margin_top(12.).finish());
    }

    Container::new(content.finish())
        .with_uniform_padding(24.)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .with_border(Border::all(1.).with_border_fill(border_color))
        .with_background(background)
        .finish()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_inline_segmented_control(
    appearance: &Appearance,
    is_left_selected: bool,
    left_label: &'static str,
    right_label: &'static str,
    enabled_mouse: MouseStateHandle,
    disabled_mouse: MouseStateHandle,
    on_left: ClickCallback,
    on_right: ClickCallback,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let ui_font_family = appearance.ui_font_family();
    let selected_bg = internal_colors::accent_overlay_3(theme);
    let text_main = internal_colors::text_main(theme, theme.background().into_solid());
    let text_sub = internal_colors::text_sub(theme, theme.background().into_solid());
    let control_bg = internal_colors::fg_overlay_1(theme);

    let build_option = move |label: &'static str,
                             is_selected: bool,
                             mouse: MouseStateHandle,
                             mut callback: ClickCallback| {
        let option = Hoverable::new(mouse, move |_| {
            let label_el = FormattedTextElement::from_str(label, ui_font_family, 14.)
                .with_color(if is_selected { text_main } else { text_sub })
                .with_weight(Weight::Normal)
                .with_alignment(TextAlignment::Center)
                .with_line_height_ratio(1.0)
                .finish();

            let aligned = Align::new(label_el).finish();

            let mut container = Container::new(aligned)
                .with_padding_left(8.)
                .with_padding_right(8.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));

            if is_selected {
                container = container.with_background(selected_bg);
            }

            container.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, app, pos| {
            callback(ctx, app, pos);
        })
        .finish();
        Shrinkable::new(1., option).finish()
    };

    let left = build_option(left_label, is_left_selected, enabled_mouse, on_left);
    let right = build_option(right_label, !is_left_selected, disabled_mouse, on_right);

    Container::new(
        ConstrainedBox::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(left)
                .with_child(right)
                .finish(),
        )
        .with_height(24.)
        .finish(),
    )
    .with_uniform_padding(4.)
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
    .with_background(control_bg)
    .finish()
}

fn render_chips(appearance: &Appearance, chips: Vec<ChipSpec>) -> Box<dyn Element> {
    let mut wrap = Wrap::row().with_spacing(12.).with_run_spacing(12.);
    wrap.extend(chips.into_iter().map(|chip| render_chip(appearance, chip)));
    wrap.finish()
}

fn render_chip(appearance: &Appearance, mut chip: ChipSpec) -> Box<dyn Element> {
    let theme = appearance.theme();
    let ui_font_family = appearance.ui_font_family();

    let (bg, border) = if chip.is_enabled {
        (
            Some(internal_colors::accent_overlay_2(theme)),
            Some(theme.accent()),
        )
    } else {
        (Some(internal_colors::fg_overlay_1(theme)), None)
    };

    let text_color = if chip.is_enabled {
        internal_colors::text_main(theme, theme.background().into_solid())
    } else {
        internal_colors::text_sub(theme, theme.background().into_solid())
    };

    let label = chip.label;

    let mut hoverable = Hoverable::new(chip.mouse_state, move |_| {
        let label_el = FormattedTextElement::from_str(label, ui_font_family, 14.)
            .with_color(text_color)
            .with_weight(Weight::Normal)
            .with_alignment(TextAlignment::Center)
            .with_line_height_ratio(1.0)
            .finish();

        let mut container = Container::new(Align::new(label_el).finish())
            .with_padding_left(12.)
            .with_padding_right(12.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));

        if let Some(bg) = bg {
            container = container.with_background(bg);
        }
        let border_fill = border.unwrap_or(Fill::Solid(pathfinder_color::ColorU::new(0, 0, 0, 0)));
        container = container.with_border(Border::all(1.).with_border_fill(border_fill));

        ConstrainedBox::new(container.finish())
            .with_height(32.)
            .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, app, pos| {
        (chip.on_click)(ctx, app, pos);
    });

    if let Some(mut hover_cb) = chip.on_hover {
        hoverable = hoverable.on_hover(move |is_hovered, ctx, app, pos| {
            hover_cb(is_hovered, ctx, app, pos);
        });
    }

    hoverable.finish()
}
