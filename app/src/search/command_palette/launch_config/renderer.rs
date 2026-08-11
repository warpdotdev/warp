use warpui::Element;
use warpui::elements::{
    Align, Border, ConstrainedBox, Container, CornerRadius, Flex, Highlight, ParentElement, Radius,
    Shrinkable, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::text::Span;

use crate::appearance::Appearance;
use crate::launch_configs::launch_config::LaunchConfig;
use crate::search::result_renderer::ItemHighlightState;
use crate::themes::theme::Fill;

impl LaunchConfig {
    /// Renders a [`LaunchConfig`] using a [`StylesProvider`]. Any character indices of the launch
    /// config title contained within `highlighted_indices` are highlighted in bold.
    pub(super) fn render(
        &self,
        appearance: &Appearance,
        item_highlight_state: ItemHighlightState,
        highlight_indices: Vec<usize>,
    ) -> Box<dyn Element> {
        let bg_color = background_fill(item_highlight_state, appearance);

        let text_color = appearance.theme().main_text_color(bg_color).into_solid();

        let highlight = Highlight::new()
            .with_properties(Properties::default().weight(Weight::Bold))
            .with_foreground_color(text_color);

        let label = self
            .render_launch_config_name(appearance, item_highlight_state)
            .with_single_highlight(highlight, highlight_indices)
            .finish();

        let mut configuration = Flex::row();
        configuration.add_child(Shrinkable::new(1., Align::new(label).left().finish()).finish());

        configuration.add_child(
            Container::new(self.render_config_description(appearance))
                .with_margin_right(14.)
                .finish(),
        );

        ConstrainedBox::new(configuration.finish())
            .with_height(40.)
            .finish()
    }

    fn default_pill_styles(appearance: &Appearance) -> UiComponentStyles {
        UiComponentStyles {
            font_family_id: Some(appearance.ui_font_family()),
            font_size: Some(appearance.monospace_font_size()),
            font_color: Some(
                appearance
                    .theme()
                    .hint_text_color(appearance.theme().background())
                    .into_solid(),
            ),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
            background: Some(appearance.theme().background().into()),
            height: Some(24.),
            padding: Some(Coords::default().left(6.).right(6.)),
            margin: Some(Coords::default().left(3.)),
            ..Default::default()
        }
    }

    fn render_string_with_pill_styling(
        str: impl Into<String>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let style = Self::default_pill_styles(appearance);
        let mut container =
            Container::new(Align::new(Span::new(str.into(), style).build().finish()).finish());
        let mut border = Border::all(style.border_width.unwrap_or_default());
        if let Some(border_color) = style.border_color {
            border = border.with_border_fill(border_color);
        }
        container = container.with_border(border);
        if let Some(padding) = style.padding {
            container = container
                .with_padding_top(padding.top)
                .with_padding_right(padding.right)
                .with_padding_bottom(padding.bottom)
                .with_padding_left(padding.left);
        }
        if let Some(radius) = style.border_radius {
            container = container.with_corner_radius(radius);
        }
        if let Some(background_color) = style.background {
            container = container.with_background(background_color);
        }
        let mut sized_container = ConstrainedBox::new(container.finish());
        if let Some(width) = style.width {
            sized_container = sized_container.with_width(width);
        }
        if let Some(height) = style.height {
            sized_container = sized_container.with_height(height);
        }
        let mut container = Container::new(Align::new(sized_container.finish()).finish());
        if let Some(margin) = style.margin {
            container = container
                .with_margin_top(margin.top)
                .with_margin_right(margin.right)
                .with_margin_bottom(margin.bottom)
                .with_margin_left(margin.left);
        }
        container.finish()
    }

    fn render_config_description(&self, appearance: &Appearance) -> Box<dyn Element> {
        let num_windows = self.windows.len();
        let num_tabs: usize = self.windows.iter().map(|window| window.tabs.len()).sum();
        let mut windows_str = num_windows.to_string();
        match num_windows {
            1 => windows_str.push_str(" window "),
            _ => windows_str.push_str(" windows"),
        }
        let mut tabs_str = num_tabs.to_string();
        match num_tabs {
            1 => tabs_str.push_str(" tab "),
            _ => tabs_str.push_str(" tabs"),
        }
        Flex::row()
            .with_children(vec![
                Self::render_string_with_pill_styling(windows_str, appearance),
                Self::render_string_with_pill_styling(tabs_str, appearance),
            ])
            .finish()
    }

    fn render_launch_config_name(
        &self,
        appearance: &Appearance,
        item_highlight_state: ItemHighlightState,
    ) -> Text {
        let text = Text::new_inline(
            self.name.clone(),
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        );

        let bg_color = background_fill(item_highlight_state, appearance);
        text.with_color(appearance.theme().sub_text_color(bg_color).into_solid())
    }
}

fn background_fill(item_highlight_state: ItemHighlightState, appearance: &Appearance) -> Fill {
    item_highlight_state
        .container_background_fill(appearance)
        .unwrap_or_else(|| appearance.theme().surface_2())
}
