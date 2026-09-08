use pathfinder_color::ColorU;
use warp_core::ui::Icon;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::Fill;
use warp_core::ui::theme::color::internal_colors;
use warpui_core::Element;
use warpui_core::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex,
    FormattedTextElement, MainAxisAlignment, MainAxisSize, ParentElement, Radius, Shrinkable,
};
use warpui_core::fonts::Weight;
use warpui_core::text_layout::TextAlignment;

/// Content for a "you'll lose these features" opt-out confirmation dialog.
pub struct FeatureOptOutDialog {
    pub title: &'static str,
    pub body: &'static str,
    pub features: &'static [&'static str],
    pub close_button: Box<dyn Element>,
    pub cancel_button: Box<dyn Element>,
    pub confirm_button: Box<dyn Element>,
}

pub fn render_feature_optout_dialog(
    appearance: &Appearance,
    dialog: FeatureOptOutDialog,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let dialog_surface = theme.surface_1();
    let dialog_surface_solid = dialog_surface.into_solid();
    let border_color = internal_colors::neutral_4(theme);

    let title = FormattedTextElement::from_str(dialog.title, appearance.ui_font_family(), 16.)
        .with_color(internal_colors::text_main(theme, dialog_surface_solid))
        .with_weight(Weight::Bold)
        .with_line_height_ratio(1.25)
        .finish();

    let title_row = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_child(Shrinkable::new(1., title).finish())
        .with_child(dialog.close_button)
        .finish();

    let body_text = FormattedTextElement::from_str(dialog.body, appearance.ui_font_family(), 14.)
        .with_color(internal_colors::text_main(theme, dialog_surface_solid))
        .with_weight(Weight::Normal)
        .with_line_height_ratio(1.2)
        .finish();

    let mut body_section = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_child(body_text);

    // The list is optional: callers that only want a warning (e.g. the no-AI
    // onboarding modal) pass an empty slice, in which case we skip it entirely.
    if !dialog.features.is_empty() {
        let feature_row_color: ColorU = theme.foreground().into();
        let feature_x_fill = Fill::Solid(theme.ansi_fg_red());
        let mut feature_list =
            Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        for &item in dialog.features {
            let icon_el = ConstrainedBox::new(Icon::X.to_warpui_icon(feature_x_fill).finish())
                .with_width(16.)
                .with_height(16.)
                .finish();
            let text_el = FormattedTextElement::from_str(item, appearance.ui_font_family(), 14.)
                .with_color(feature_row_color)
                .with_weight(Weight::Normal)
                .with_alignment(TextAlignment::Left)
                .with_line_height_ratio(1.0)
                .finish();
            let row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(icon_el)
                .with_child(Container::new(text_el).with_margin_left(4.).finish())
                .finish();
            feature_list = feature_list.with_child(
                Container::new(row)
                    .with_padding_top(4.)
                    .with_padding_bottom(4.)
                    .finish(),
            );
        }
        body_section = body_section.with_child(
            Container::new(feature_list.finish())
                .with_margin_top(12.)
                .finish(),
        );
    }

    let body_section = body_section.finish();

    let footer = Container::new(
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(dialog.cancel_button)
            .with_child(
                Container::new(dialog.confirm_button)
                    .with_margin_left(8.)
                    .finish(),
            )
            .finish(),
    )
    .with_border(Border::top(1.).with_border_color(border_color))
    .with_horizontal_padding(24.)
    .with_vertical_padding(12.)
    .finish();

    let dialog_body = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(
            Container::new(title_row)
                .with_horizontal_padding(24.)
                .with_padding_top(24.)
                .with_padding_bottom(12.)
                .finish(),
        )
        .with_child(
            Container::new(body_section)
                .with_horizontal_padding(24.)
                .with_padding_bottom(16.)
                .finish(),
        )
        .with_child(footer)
        .finish();

    ConstrainedBox::new(
        Container::new(dialog_body)
            .with_background(dialog_surface)
            .with_border(Border::all(1.).with_border_color(border_color))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .finish(),
    )
    .with_width(460.)
    .finish()
}
