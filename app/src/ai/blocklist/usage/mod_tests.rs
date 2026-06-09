use warp_core::ui::Icon;

use super::{icon_for_context_window_usage, LongContextWarningState};
use crate::ai::llms::{LLMId, LLMProvider};

#[test]
fn new_initializes_visibility_from_long_context_used() {
    let visible = LongContextWarningState::new(LLMId::from("gpt-5"), LLMProvider::OpenAI, true);
    assert!(visible.is_visible());

    let hidden = LongContextWarningState::new(LLMId::from("gpt-5"), LLMProvider::OpenAI, false);
    assert!(!hidden.is_visible());
}

#[test]
fn sync_from_server_overwrites_visibility() {
    let mut state = LongContextWarningState::new(LLMId::from("gpt-5"), LLMProvider::OpenAI, false);
    state.sync_from_server(true);
    assert!(state.is_visible());

    // A later short request clears the warning.
    state.sync_from_server(false);
    assert!(!state.is_visible());
}

#[test]
fn changing_effective_model_resets_warning() {
    let mut state = LongContextWarningState::new(LLMId::from("gpt-5"), LLMProvider::OpenAI, true);
    assert!(state.is_visible());

    // Selecting a different base model hides the prior model's warning. Both models are
    // OpenAI here so this isolates the model-change reset from the provider gate.
    state.update_effective_model(LLMId::from("gpt-5.1"), LLMProvider::OpenAI);
    assert!(!state.is_visible());
}

#[test]
fn reselecting_same_effective_model_does_not_reset_warning() {
    let mut state = LongContextWarningState::new(LLMId::from("gpt-5"), LLMProvider::OpenAI, true);
    assert!(state.is_visible());

    // Re-selecting the same effective model must not reset the warning.
    state.update_effective_model(LLMId::from("gpt-5"), LLMProvider::OpenAI);
    assert!(state.is_visible());
}

#[test]
fn server_value_remains_authoritative_after_model_change() {
    let mut state = LongContextWarningState::new(LLMId::from("gpt-5"), LLMProvider::OpenAI, true);
    state.update_effective_model(LLMId::from("gpt-5.1"), LLMProvider::OpenAI);
    assert!(!state.is_visible());

    // The next streamed/restored server value can show the warning again.
    state.sync_from_server(true);
    assert!(state.is_visible());
}

#[test]
fn warning_hidden_for_non_openai_provider_even_when_long_context_used() {
    // The server may report long-context usage for non-OpenAI models (e.g. Gemini), but the
    // OpenAI-specific pricing warning must not surface for them.
    let anthropic =
        LongContextWarningState::new(LLMId::from("claude-sonnet"), LLMProvider::Anthropic, true);
    assert!(!anthropic.is_visible());

    let google =
        LongContextWarningState::new(LLMId::from("gemini-3-pro"), LLMProvider::Google, true);
    assert!(!google.is_visible());
}

#[test]
fn sync_from_server_does_not_show_for_non_openai_provider() {
    let mut state =
        LongContextWarningState::new(LLMId::from("claude-sonnet"), LLMProvider::Anthropic, false);
    state.sync_from_server(true);
    assert!(!state.is_visible());
}

#[test]
fn switching_to_non_openai_model_hides_warning_even_with_server_true() {
    let mut state = LongContextWarningState::new(LLMId::from("gpt-5"), LLMProvider::OpenAI, true);
    assert!(state.is_visible());

    // Switching to a non-OpenAI model hides the warning, and a later server "true" must not
    // resurface it while a non-OpenAI model is the effective model.
    state.update_effective_model(LLMId::from("claude-sonnet"), LLMProvider::Anthropic);
    assert!(!state.is_visible());
    state.sync_from_server(true);
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
