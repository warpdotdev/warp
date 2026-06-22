use warp_core::ui::theme::{Fill, WarpTheme};
use warp_core::ui::Icon;
use warpui::Element;

pub mod conversation_usage_view;
pub mod rollup;

pub fn icon_for_context_window_usage(context_window_usage: f32) -> Icon {
    // Match the context window usage to the nearest 10% icon.
    if context_window_usage >= 0.95 {
        Icon::ConversationContext100
    } else if context_window_usage >= 0.85 {
        Icon::ConversationContext90
    } else if context_window_usage >= 0.75 {
        Icon::ConversationContext80
    } else if context_window_usage >= 0.65 {
        Icon::ConversationContext70
    } else if context_window_usage >= 0.55 {
        Icon::ConversationContext60
    } else if context_window_usage >= 0.45 {
        Icon::ConversationContext50
    } else if context_window_usage >= 0.35 {
        Icon::ConversationContext40
    } else if context_window_usage >= 0.25 {
        Icon::ConversationContext30
    } else if context_window_usage >= 0.15 {
        Icon::ConversationContext20
    } else if context_window_usage >= 0.05 {
        Icon::ConversationContext10
    } else {
        Icon::ConversationContext0
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
