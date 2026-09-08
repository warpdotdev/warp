use warp_core::ui::Icon;
use warp_core::ui::theme::{Fill, WarpTheme};
use warpui::Element;

pub mod conversation_usage_view;
pub mod rollup;

pub fn icon_for_context_window_usage(context_window_usage: f32) -> Icon {
    // The circle's solid (white) marks represent the context *remaining*, not
    // the amount used: an empty conversation shows an all-white circle (100%
    // remaining) and counts down to an all-grey circle as the context window
    // fills up (0% remaining). So match the *remaining* fraction
    // (`1 - usage`) to the nearest 10% icon, where `ContextRemainingN`
    // brightens N% of the ring.
    let context_window_remaining = 1.0 - context_window_usage;
    if context_window_remaining >= 0.95 {
        Icon::ContextRemaining100
    } else if context_window_remaining >= 0.85 {
        Icon::ContextRemaining90
    } else if context_window_remaining >= 0.75 {
        Icon::ContextRemaining80
    } else if context_window_remaining >= 0.65 {
        Icon::ContextRemaining70
    } else if context_window_remaining >= 0.55 {
        Icon::ContextRemaining60
    } else if context_window_remaining >= 0.45 {
        Icon::ContextRemaining50
    } else if context_window_remaining >= 0.35 {
        Icon::ContextRemaining40
    } else if context_window_remaining >= 0.25 {
        Icon::ContextRemaining30
    } else if context_window_remaining >= 0.15 {
        Icon::ContextRemaining20
    } else if context_window_remaining >= 0.05 {
        Icon::ContextRemaining10
    } else {
        Icon::ContextRemaining0
    }
}

pub fn render_context_window_usage_icon(
    context_window_usage: f32,
    theme: &WarpTheme,
    color_override: Option<Fill>,
) -> Box<dyn Element> {
    let icon = icon_for_context_window_usage(context_window_usage);

    let fill = if context_window_usage >= 0.8 {
        Fill::Solid(theme.ansi_fg_red())
    } else {
        color_override.unwrap_or_else(|| theme.main_text_color(theme.background()))
    };

    icon.to_warpui_icon(fill).finish()
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
