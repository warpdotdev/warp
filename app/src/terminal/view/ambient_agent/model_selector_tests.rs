use std::collections::HashMap;

use warp_core::ui::Icon;
use warpui::App;

use super::*;
use crate::ai::llms::{LLMContextWindow, LLMProvider, LLMUsageMetadata};
use crate::auth::AuthStateProvider;
use crate::workspaces::user_workspaces::UserWorkspaces;

fn test_llm(id: &str, display_name: &str, provider: LLMProvider) -> LLMInfo {
    LLMInfo {
        display_name: display_name.to_owned(),
        base_model_name: display_name.to_owned(),
        id: LLMId::from(id),
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

/// Runs `f` with an `AppContext` that has just enough singletons registered
/// for `should_show_bedrock_icon_for_model` / `should_show_gemini_enterprise_agent_platform_icon_for_model`
/// to resolve (they read `UserWorkspaces`, gated behind `AuthStateProvider`).
fn with_test_app_context(f: impl FnOnce(&AppContext) + 'static) {
    App::test((), |app| async move {
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(UserWorkspaces::default_mock);
        app.read(f);
    });
}

#[test]
fn kimi_models_get_the_kimi_logo_in_the_ambient_agent_menu() {
    with_test_app_context(|ctx| {
        let llm = test_llm("kimi-k3-fireworks", "Kimi K3", LLMProvider::Unknown);
        let item = oz_menu_item_for_llm(&llm, Fill::black(), ctx);
        let MenuItem::Item(fields) = item else {
            panic!("expected a plain menu item");
        };
        assert_eq!(fields.icon(), Some(Icon::KimiLogo));
    });
}

#[test]
fn non_kimi_models_keep_their_provider_icon_in_the_ambient_agent_menu() {
    with_test_app_context(|ctx| {
        let mut llm = test_llm("gpt-4o", "GPT-4o", LLMProvider::OpenAI);
        let item = oz_menu_item_for_llm(&llm, Fill::black(), ctx);
        let MenuItem::Item(fields) = item else {
            panic!("expected a plain menu item");
        };
        assert_eq!(fields.icon(), Some(Icon::OpenAILogo));

        // Auto models now go through the shared helper too, so they show the
        // generic agent glyph rather than a provider logo, consistent with
        // every other model picker surface.
        llm.id = LLMId::from("auto");
        llm.display_name = "auto".to_owned();
        let item = oz_menu_item_for_llm(&llm, Fill::black(), ctx);
        let MenuItem::Item(fields) = item else {
            panic!("expected a plain menu item");
        };
        assert_eq!(fields.icon(), Some(Icon::Agent));
    });
}
