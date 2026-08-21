//! Manually executed test that verifies the model picker's leading icon
//! renders the Kimi logo for Kimi models, alongside other provider logos for
//! comparison. Kimi models are served through Fireworks and arrive from the
//! server as `LLMProvider::Unknown`, so the client keys the icon off the
//! model id instead (see `is_kimi_model_id` in `app/src/ai/llms.rs`).
//!
//! Run with a real display to capture the screenshot:
//!
//! ```sh
//! WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
//!   cargo run -p integration --bin integration -- test_model_picker_shows_kimi_logo
//! ```

use std::collections::HashMap;

use warp::integration_testing::settings::open_default_profile_base_model_dropdown;
use warp::integration_testing::terminal::wait_until_bootstrapped_single_pane_for_tab;
use warpui_core::async_assert;
use warpui_core::integration::TestStep;

use super::{Builder, new_builder};

/// A `ModelsByFeature` JSON payload (matching the wire/cache format read by
/// `get_cached_models` under the `AvailableLLMs` user-preferences key) with
/// one model per existing provider logo plus a Kimi model, so the model
/// picker can be screenshotted with several logos side by side for
/// comparison.
const SEEDED_MODELS_BY_FEATURE_JSON: &str = r#"{
  "agent_mode": {
    "default_id": "auto",
    "choices": [
      { "display_name": "auto", "id": "auto", "usage_metadata": { "request_multiplier": 1, "credit_multiplier": null }, "description": null, "disable_reason": null, "vision_supported": true, "spec": null, "provider": "Unknown" },
      { "display_name": "Claude Sonnet", "id": "claude-4-5-sonnet", "usage_metadata": { "request_multiplier": 1, "credit_multiplier": null }, "description": null, "disable_reason": null, "vision_supported": true, "spec": null, "provider": "Anthropic" },
      { "display_name": "GPT-5", "id": "gpt-5", "usage_metadata": { "request_multiplier": 1, "credit_multiplier": null }, "description": null, "disable_reason": null, "vision_supported": true, "spec": null, "provider": "OpenAI" },
      { "display_name": "Grok 4", "id": "grok-4", "usage_metadata": { "request_multiplier": 1, "credit_multiplier": null }, "description": null, "disable_reason": null, "vision_supported": true, "spec": null, "provider": "Xai" },
      { "display_name": "Kimi K3", "id": "kimi-k3-fireworks", "usage_metadata": { "request_multiplier": 1, "credit_multiplier": null }, "description": null, "disable_reason": null, "vision_supported": true, "spec": null, "provider": "Unknown" }
    ]
  },
  "coding": {
    "default_id": "auto",
    "choices": [
      { "display_name": "auto", "id": "auto", "usage_metadata": { "request_multiplier": 1, "credit_multiplier": null }, "description": null, "disable_reason": null, "vision_supported": true, "spec": null, "provider": "Unknown" }
    ]
  }
}"#;

/// Manually executed test that seeds the cached model list with a Kimi model
/// (and several other provider models) with no network/login required, then
/// opens the execution profile editor's base-model dropdown and screenshots
/// it so the Kimi logo can be visually verified next to other provider logos.
pub fn test_model_picker_shows_kimi_logo() -> Builder {
    let user_defaults = HashMap::from([(
        "AvailableLLMs".to_owned(),
        SEEDED_MODELS_BY_FEATURE_JSON.to_owned(),
    )]);

    new_builder()
        .with_real_display()
        .with_user_defaults(user_defaults)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(
            TestStep::new("Open execution profile editor and expand base model dropdown")
                .add_named_assertion("Open base model dropdown", |app, window_id| {
                    open_default_profile_base_model_dropdown(app, window_id);
                    async_assert!(true, "Opened base model dropdown")
                }),
        )
        .with_step(
            TestStep::new("Screenshot model picker with Kimi logo")
                .with_take_screenshot("model_picker_with_kimi_logo.png"),
        )
}
