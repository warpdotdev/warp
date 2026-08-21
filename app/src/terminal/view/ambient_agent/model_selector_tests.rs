use std::collections::HashMap;

use ai::LLMProvider;

use super::*;
use crate::ai::llms::{LLMContextWindow, LLMUsageMetadata};

fn llm(id: &str, provider: LLMProvider) -> LLMInfo {
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
fn kimi_models_show_the_kimi_logo_in_the_oz_menu() {
    let model = llm("kimi-k3-fireworks", LLMProvider::Unknown);
    assert_eq!(oz_menu_item_icon(&model), Icon::KimiLogo);
}

#[test]
fn auto_models_keep_the_agent_glyph_in_the_oz_menu() {
    // `is_auto` is derived from the id/display name here (unlike
    // `model_leading_icon`'s other call sites, this menu has no host/router
    // flags to pass beyond custom-router and auto), so this pins the
    // pre-existing behavior for auto rows across the `model_leading_icon` swap.
    let model = llm("auto", LLMProvider::Unknown);
    assert_eq!(oz_menu_item_icon(&model), Icon::Agent);
}

#[test]
fn custom_router_models_show_the_dataflow_icon_in_the_oz_menu() {
    let model = llm("custom-router:local:my-router", LLMProvider::Unknown);
    assert_eq!(oz_menu_item_icon(&model), Icon::Dataflow);
}

#[test]
fn provider_models_keep_their_own_logo_in_the_oz_menu() {
    let model = llm("claude-opus", LLMProvider::Anthropic);
    assert_eq!(oz_menu_item_icon(&model), Icon::ClaudeLogo);
}
