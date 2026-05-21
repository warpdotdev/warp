use crate::appearance::Appearance;
use crate::search::QueryFilter;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex, Hoverable, Icon,
    MouseStateHandle, ParentElement, Radius, Text,
};
use warpui::platform::Cursor;
use warpui::{Element, EventContext};

/// Trait to render a filter chip.
pub trait FilterChipRenderer {
    /// Returns how much larger the icon should be than the font size.
    fn icon_size_offset(&self) -> f32;

    /// Returns the margin from the top of the icon of the filter chip.
    fn icon_margin_top(&self) -> f32;

    /// Renders the filter chip. When the filter chip is clicked, `on_click_fn` is called.  
    fn render_filter_chip(
        &self,
        mouse_state_handle: MouseStateHandle,
        appearance: &Appearance,
        on_click_fn: fn(&mut EventContext, Self),
    ) -> Box<dyn Element>;
}

impl FilterChipRenderer for QueryFilter {
    fn icon_size_offset(&self) -> f32 {
        match self {
            QueryFilter::NaturalLanguage => 2.,
            _ => 0.,
        }
    }

    fn icon_margin_top(&self) -> f32 {
        match self {
            QueryFilter::Sessions => 2.,
            QueryFilter::Tabs => 2.,
            QueryFilter::NaturalLanguage => 2.,
            _ => 0.,
        }
    }

    fn render_filter_chip(
        &self,
        mouse_state_handle: MouseStateHandle,
        appearance: &Appearance,
        on_click_fn: fn(&mut EventContext, Self),
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let self_copy: QueryFilter = *self;
        Hoverable::new(mouse_state_handle, |mouse_state| {
            let font_size = appearance.monospace_font_size() + 2.;
            let icon_size = font_size + self.icon_size_offset();

            let mut flex = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
            if let Some(icon_name) = self.icon_svg_path() {
                flex.add_child(
                    Container::new(
                        ConstrainedBox::new(
                            Icon::new(
                                icon_name,
                                appearance
                                    .theme()
                                    .sub_text_color(appearance.theme().surface_2())
                                    .into_solid(),
                            )
                            .finish(),
                        )
                        .with_width(icon_size)
                        .with_height(icon_size)
                        .finish(),
                    )
                    .with_margin_top(self.icon_margin_top())
                    .with_margin_right(8.)
                    .finish(),
                );
            }

            flex.add_child(
                Text::new_inline(self.display_name(), appearance.ui_font_family(), font_size)
                    .with_color(
                        appearance
                            .theme()
                            .main_text_color(appearance.theme().surface_2())
                            .into_solid(),
                    )
                    .finish(),
            );

            Container::new(flex.finish())
                .with_padding_top(8.)
                .with_padding_bottom(8.)
                .with_padding_left(10.)
                .with_padding_right(10.)
                .with_background(if mouse_state.is_hovered() {
                    theme.accent_overlay()
                } else {
                    internal_colors::neutral_3(theme).into()
                })
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |event_ctx, _, _| on_click_fn(event_ctx, self_copy))
        .finish()
    }
}
