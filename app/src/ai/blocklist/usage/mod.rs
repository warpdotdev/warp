use warp_core::ui::theme::{Fill, WarpTheme};
use warp_core::ui::Icon;
use warpui::Element;

use crate::ai::llms::{LLMId, LLMProvider};

pub mod conversation_usage_view;
pub mod rollup;

#[derive(Debug)]
pub(crate) struct LongContextWarningState {
    effective_model_id: LLMId,
    effective_model_provider: LLMProvider,
    visible: bool,
}

impl LongContextWarningState {
    pub fn new(
        effective_model_id: LLMId,
        effective_model_provider: LLMProvider,
        long_context_used: bool,
    ) -> Self {
        Self {
            effective_model_id,
            effective_model_provider,
            visible: long_context_used,
        }
    }

    pub fn sync_from_server(&mut self, long_context_used: bool) {
        self.visible = long_context_used;
    }

    pub fn update_effective_model(
        &mut self,
        effective_model_id: LLMId,
        effective_model_provider: LLMProvider,
    ) {
        if self.effective_model_id != effective_model_id {
            self.effective_model_id = effective_model_id;
            self.effective_model_provider = effective_model_provider;
            self.visible = false;
        }
    }

    /// The long-context warning communicates OpenAI's long-context pricing tiers, so it is only
    /// surfaced for OpenAI models — even if the server reports long-context usage for a model
    /// from another provider.
    pub fn is_visible(&self) -> bool {
        self.visible && self.effective_model_provider == LLMProvider::OpenAI
    }
}

pub fn icon_for_context_window_usage(
    context_window_usage: f32,
    show_long_context_warning: bool,
) -> Icon {
    if show_long_context_warning {
        return Icon::ConversationContext100;
    }

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
    let icon = icon_for_context_window_usage(context_window_usage, false);

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
