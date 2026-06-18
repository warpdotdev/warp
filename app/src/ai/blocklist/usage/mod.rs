use warp_core::ui::theme::{Fill, WarpTheme};
use warp_core::ui::Icon;
use warpui::Element;

use crate::ai::llms::LLMProvider;

pub mod conversation_usage_view;
pub mod rollup;

#[derive(Debug)]
pub(crate) struct LongContextWarningState {
    effective_model_provider: LLMProvider,
    /// The active model's long-context pricing threshold, in input tokens.
    /// `None` when the model has no long-context pricing tier.
    effective_model_threshold: Option<u32>,
    /// Whether the active model is a user-configured custom endpoint. The
    /// warning communicates Warp-priced OpenAI long-context tiers, which don't
    /// apply to custom endpoints, so it is never surfaced for them.
    is_custom_endpoint: bool,
    /// Input tokens of the latest primary-agent LLM call in the latest
    /// successfully persisted request, as reported by the server.
    total_input_tokens: u32,
}

impl LongContextWarningState {
    pub fn new(
        effective_model_provider: LLMProvider,
        effective_model_threshold: Option<u32>,
        is_custom_endpoint: bool,
        total_input_tokens: u32,
    ) -> Self {
        Self {
            effective_model_provider,
            effective_model_threshold,
            is_custom_endpoint,
            total_input_tokens,
        }
    }

    pub fn sync_from_server(&mut self, total_input_tokens: u32) {
        self.total_input_tokens = total_input_tokens;
    }

    pub fn update_effective_model(
        &mut self,
        effective_model_provider: LLMProvider,
        effective_model_threshold: Option<u32>,
        is_custom_endpoint: bool,
    ) {
        self.effective_model_provider = effective_model_provider;
        self.effective_model_threshold = effective_model_threshold;
        self.is_custom_endpoint = is_custom_endpoint;
    }

    /// Visible when the latest reported input tokens exceed the active model's
    /// long-context pricing threshold (strictly greater, matching the server's
    /// pricing predicate). The warning communicates OpenAI's long-context
    /// pricing tiers, so it is only surfaced for OpenAI models — even though
    /// other providers (e.g. Gemini) also expose a threshold — and never for
    /// custom endpoints, whose pricing Warp does not control.
    pub fn is_visible(&self) -> bool {
        !self.is_custom_endpoint
            && self.effective_model_provider == LLMProvider::OpenAI
            && self
                .effective_model_threshold
                .is_some_and(|threshold| self.total_input_tokens > threshold)
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
