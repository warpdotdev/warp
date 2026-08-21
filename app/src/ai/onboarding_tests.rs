use std::collections::HashMap;

use warp_core::ui::icons::Icon;

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
fn kimi_models_show_the_kimi_logo_on_the_onboarding_slide() {
    let llm = test_llm("kimi-k3-fireworks");
    let info = OnboardingModelInfo::from(&llm);
    assert_eq!(info.icon, Icon::KimiLogo);
}

#[test]
fn non_kimi_models_show_the_agent_glyph_on_the_onboarding_slide() {
    let llm = test_llm("gpt-test");
    let info = OnboardingModelInfo::from(&llm);
    assert_eq!(info.icon, Icon::Agent);
}
