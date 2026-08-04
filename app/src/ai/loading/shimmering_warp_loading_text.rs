//! Shimmering Warp loading text - renders Warp logo with shimmering text for loading states.

use warp_core::ui::appearance::Appearance;
use warpui::elements::Element;
use warpui::elements::shimmering_text::{
    ShimmerConfig, ShimmeringTextElement, ShimmeringTextStateHandle,
};
use warpui::{AppContext, SingletonEntity};

/// Warp icon glyph character
const WARP_GLYPH: &str = "\u{E500}";

/// Creates a shimmering text element with the Warp glyph.
pub fn shimmering_warp_loading_text(
    text: impl Into<String>,
    font_size: f32,
    shimmer_handle: ShimmeringTextStateHandle,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    // Use same colors as common.rs for consistency
    let base_color = theme.disabled_text_color(theme.surface_1()).into_solid();
    let shimmer_color = theme.main_text_color(theme.surface_1()).into_solid();

    // Hardcoded shimmer config for consistent animation
    let config = ShimmerConfig::default();

    // Create a single shimmering element with glyph and text
    ShimmeringTextElement::new(
        format!("{} {}", WARP_GLYPH, text.into()),
        appearance.ui_font_family(),
        font_size,
        base_color,
        shimmer_color,
        config,
        shimmer_handle,
    )
    .finish()
}
