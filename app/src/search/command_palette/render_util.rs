use pathfinder_color::ColorU;
use warp_core::ui::theme::Fill;
use warpui::Element;
use warpui::elements::{Align, ConstrainedBox, Container, Empty};

use crate::appearance::Appearance;
use crate::search::result_renderer::ItemHighlightState;
use crate::themes::theme::Blend;
use crate::ui_components::icons::Icon;
use crate::util::color::{ContrastingColor, MinimumAllowedContrast};

/// Helper function to render an icon for any search item within the command palette with consistent
/// styling.
pub fn render_search_item_icon(
    appearance: &Appearance,
    icon: Icon,
    icon_color: ColorU,
    highlight_state: ItemHighlightState,
) -> Box<dyn Element> {
    let base_background = appearance.theme().surface_2();
    let background_color = match highlight_state.container_background_fill(appearance) {
        None => base_background,
        Some(highlight) => base_background.blend(&highlight),
    };
    let icon_color = icon_color.on_background(
        background_color.into_solid(),
        MinimumAllowedContrast::NonText,
    );
    let icon_element = icon.to_warpui_icon(Fill::Solid(icon_color)).finish();
    render_search_item_icon_inner(appearance, icon_element)
}

/// Helper function to render a placeholder element when a search item does not have an icon.
pub fn render_search_item_icon_placeholder(appearance: &Appearance) -> Box<dyn Element> {
    render_search_item_icon_inner(appearance, Empty::new().finish())
}

fn render_search_item_icon_inner(
    appearance: &Appearance,
    inner_element: Box<dyn Element>,
) -> Box<dyn Element> {
    Container::new(
        ConstrainedBox::new(Align::new(inner_element).finish())
            .with_width(appearance.monospace_font_size())
            .with_height(appearance.monospace_font_size())
            .finish(),
    )
    .with_margin_right(12.)
    .finish()
}

pub mod colors {
    pub const WARP_AI: u32 = 0xF3B911FF;
}
