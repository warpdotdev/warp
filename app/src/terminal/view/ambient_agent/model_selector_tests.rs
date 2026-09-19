use std::collections::HashMap;

use ai::LLMProvider;
use warp_core::ui::Icon;

use super::*;
use crate::ai::llms::{LLMContextWindow, LLMUsageMetadata};

fn test_llm(id: &str, provider: LLMProvider) -> LLMInfo {
    LLMInfo {
        display_name: id.to_string(),
        base_model_name: id.to_string(),
        id: id.into(),
        reasoning_level: None,
        usage_metadata: LLMUsageMetadata {
            request_multiplier: 1,
            credit_multiplier: None,
        },
        description: None,
        disable_reason: None,
        vision_supported: false,
        spec: None,
        provider,
        host_configs: HashMap::new(),
        discount_percentage: None,
        context_window: LLMContextWindow::default(),
    }
}

#[test]
fn oz_menu_item_leading_icon_shows_kimi_logo_for_unknown_provider_kimi_id() {
    // Kimi models are Fireworks-hosted and reported with LLMProvider::Unknown;
    // this menu must not special-case icon selection and instead pick up the
    // brand fallback from the shared `model_leading_icon` helper.
    let llm = test_llm("kimi-k26-fireworks", LLMProvider::Unknown);

    assert_eq!(
        oz_menu_item_leading_icon(&llm, false, false),
        Icon::KimiLogo
    );
}

#[test]
fn oz_menu_item_leading_icon_still_prefers_host_icons_for_kimi_like_id() {
    // Routing icons describe how the request is routed, not which model
    // handles it, so they must still win even for a Kimi-like id.
    let llm = test_llm("kimi-k26-fireworks", LLMProvider::Unknown);

    assert_eq!(oz_menu_item_leading_icon(&llm, true, false), Icon::Aws);
    assert_eq!(
        oz_menu_item_leading_icon(&llm, false, true),
        Icon::GeminiEnterpriseAgentPlatform
    );
}

#[test]
fn oz_menu_item_leading_icon_uses_provider_logo_when_available() {
    let llm = test_llm("claude-test", LLMProvider::Anthropic);

    assert_eq!(
        oz_menu_item_leading_icon(&llm, false, false),
        Icon::ClaudeLogo
    );
}
