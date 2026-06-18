use warp_core::ui::Icon;

use super::{icon_for_context_window_usage, LongContextWarningState};
use crate::ai::llms::LLMProvider;

const THRESHOLD: u32 = 272_000;

#[test]
fn visible_only_strictly_above_threshold() {
    let below =
        LongContextWarningState::new(LLMProvider::OpenAI, Some(THRESHOLD), false, THRESHOLD - 1);
    assert!(!below.is_visible());

    let at_threshold =
        LongContextWarningState::new(LLMProvider::OpenAI, Some(THRESHOLD), false, THRESHOLD);
    assert!(
        !at_threshold.is_visible(),
        "exactly at the threshold must not warn; the server prices long context strictly above it"
    );

    let above =
        LongContextWarningState::new(LLMProvider::OpenAI, Some(THRESHOLD), false, THRESHOLD + 1);
    assert!(above.is_visible());
}

#[test]
fn sync_from_server_updates_visibility() {
    let mut state = LongContextWarningState::new(LLMProvider::OpenAI, Some(THRESHOLD), false, 0);
    assert!(!state.is_visible());

    // A qualifying request shows the warning.
    state.sync_from_server(THRESHOLD + 1);
    assert!(state.is_visible());

    // A later short request clears it.
    state.sync_from_server(12_000);
    assert!(!state.is_visible());
}

#[test]
fn hidden_without_threshold() {
    // Models without a long-context pricing tier (including Auto models, whose
    // underlying model varies) never warn, regardless of context size.
    let state = LongContextWarningState::new(LLMProvider::OpenAI, None, false, 1_000_000);
    assert!(!state.is_visible());
}

#[test]
fn hidden_for_non_openai_provider_even_above_threshold() {
    // Gemini exposes a 200K threshold, but the warning communicates OpenAI's
    // long-context pricing tiers and must not surface for other providers.
    let google = LongContextWarningState::new(LLMProvider::Google, Some(200_000), false, 250_000);
    assert!(!google.is_visible());

    let anthropic =
        LongContextWarningState::new(LLMProvider::Anthropic, Some(200_000), false, 250_000);
    assert!(!anthropic.is_visible());
}

#[test]
fn hidden_for_custom_endpoint_even_above_threshold() {
    // Custom endpoints are priced outside Warp's OpenAI long-context tiers, so
    // the warning must never surface for them — even if the synthetic model
    // somehow reports the OpenAI provider and a threshold.
    let state = LongContextWarningState::new(LLMProvider::OpenAI, Some(THRESHOLD), true, 1_000_000);
    assert!(!state.is_visible());
}

#[test]
fn model_switch_recomputes_against_new_threshold() {
    // 250K tokens is below GPT-5.4's 272K threshold...
    let mut state =
        LongContextWarningState::new(LLMProvider::OpenAI, Some(272_000), false, 250_000);
    assert!(!state.is_visible());

    // ...but above a hypothetical lower-threshold OpenAI model.
    state.update_effective_model(LLMProvider::OpenAI, Some(200_000), false);
    assert!(state.is_visible());

    // Switching to a higher-threshold model hides it again from the same tokens.
    state.update_effective_model(LLMProvider::OpenAI, Some(400_000), false);
    assert!(!state.is_visible());

    // Switching to a non-OpenAI model hides it even when its threshold is exceeded.
    state.update_effective_model(LLMProvider::Google, Some(200_000), false);
    assert!(!state.is_visible());

    // Switching to a custom endpoint hides it even when above its threshold.
    state.update_effective_model(LLMProvider::OpenAI, Some(200_000), true);
    assert!(!state.is_visible());
}

#[test]
fn zero_tokens_never_warn() {
    // Legacy conversations and old servers report 0; the warning stays hidden.
    let state = LongContextWarningState::new(LLMProvider::OpenAI, Some(THRESHOLD), false, 0);
    assert!(!state.is_visible());
}

#[test]
fn long_context_warning_forces_full_icon() {
    // With the warning active, the icon is fully lit regardless of usage.
    assert_eq!(
        icon_for_context_window_usage(0.0, true),
        Icon::ConversationContext100
    );
    assert_eq!(
        icon_for_context_window_usage(0.5, true),
        Icon::ConversationContext100
    );
}

#[test]
fn icon_tracks_usage_without_warning() {
    assert_eq!(
        icon_for_context_window_usage(0.0, false),
        Icon::ConversationContext0
    );
    assert_eq!(
        icon_for_context_window_usage(0.5, false),
        Icon::ConversationContext50
    );
    assert_eq!(
        icon_for_context_window_usage(0.95, false),
        Icon::ConversationContext100
    );
}
