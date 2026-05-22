use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    ConstrainedBox, Container, CornerRadius, Empty, Flex, MainAxisSize, ParentElement, Radius,
};
use warpui::Element;

/// Render `n` dots with 4px radius and 8px spacing. `k` is the 0-based active dot index.
pub(crate) fn progress_dots(n: usize, k: usize, appearance: &Appearance) -> Box<dyn Element> {
    const DOT_RADIUS: f32 = 4.;
    const DOT_DIAMETER: f32 = DOT_RADIUS * 2.;
    const DOT_SPACING: f32 = 8.;

    let theme = appearance.theme();
    let neutral = internal_colors::neutral_4(theme);
    let accent = internal_colors::accent(theme).into_solid();

    let dots = (0..n)
        .map(|i| {
            let color = if i == k { accent } else { neutral };
            Container::new(
                ConstrainedBox::new(
                    Container::new(Empty::new().finish())
                        .with_background_color(color)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(DOT_RADIUS)))
                        .finish(),
                )
                .with_width(DOT_DIAMETER)
                .with_height(DOT_DIAMETER)
                .finish(),
            )
            .with_margin_left(if i == 0 { 0. } else { DOT_SPACING })
            .finish()
        })
        .collect::<Vec<_>>();

    Flex::row()
        .with_main_axis_size(MainAxisSize::Min)
        .with_children(dots)
        .finish()
}
