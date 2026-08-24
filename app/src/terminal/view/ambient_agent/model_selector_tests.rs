use std::collections::HashMap;

use super::*;
use crate::ai::llms::{LLMContextWindow, LLMProvider, LLMUsageMetadata};

fn test_llm(id: &str) -> LLMInfo {
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
        provider: LLMProvider::Unknown,
        host_configs: HashMap::new(),
        discount_percentage: None,
        context_window: LLMContextWindow::default(),
    }
}

#[test]
fn kimi_models_show_the_kimi_logo_in_the_oz_menu() {
    let llm = test_llm("kimi-k3-fireworks");
    assert_eq!(oz_menu_item_leading_icon(&llm), Icon::KimiLogo);
}

#[test]
fn non_kimi_models_show_the_provider_fallback_icon_in_the_oz_menu() {
    let llm = test_llm("gpt-test");
    assert_eq!(oz_menu_item_leading_icon(&llm), Icon::Agent);
}

#[test]
fn custom_routers_keep_the_dataflow_icon_in_the_oz_menu() {
    // The custom-router check must keep winning over the kimi/provider fallback.
    let llm = test_llm("custom-router:local:my-router");
    assert_eq!(oz_menu_item_leading_icon(&llm), Icon::Dataflow);
}
