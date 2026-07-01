//! Tests for the context-window usage circle icon mapping and the
//! long-context warning state.
//!
//! Regression guard for the color semantics of the context-window circle:
//! the solid (white) marks represent the context *remaining*, not the amount
//! used. An empty conversation (0% used → 100% remaining) shows a full white
//! circle and it counts down to an all-grey circle as the window fills up
//! (100% used → 0% remaining).

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
fn set_total_input_tokens_updates_visibility() {
    let mut state = LongContextWarningState::new(LLMProvider::OpenAI, Some(THRESHOLD), false, 0);
    assert!(!state.is_visible());

    // A qualifying request shows the warning.
    state.set_total_input_tokens(THRESHOLD + 1);
    assert!(state.is_visible());

    // A later short request clears it.
    state.set_total_input_tokens(12_000);
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
    // With the warning active, the icon shows the context-full state regardless of usage.
    assert_eq!(
        icon_for_context_window_usage(0.0, true),
        Icon::ContextRemaining0
    );
    assert_eq!(
        icon_for_context_window_usage(0.5, true),
        Icon::ContextRemaining0
    );
}

#[test]
fn empty_conversation_shows_full_white_circle() {
    // 0% used == 100% remaining -> all-white circle.
    assert_eq!(
        icon_for_context_window_usage(0.0, false),
        Icon::ContextRemaining100
    );
}

#[test]
fn full_context_window_shows_all_grey_circle() {
    // 100% used == 0% remaining -> all-grey circle.
    assert_eq!(
        icon_for_context_window_usage(1.0, false),
        Icon::ContextRemaining0
    );
}

#[test]
fn icon_brightness_tracks_remaining_not_used() {
    // Lightly-used conversation: lots of context remaining -> mostly white.
    assert_eq!(
        icon_for_context_window_usage(0.1, false),
        Icon::ContextRemaining90
    );
    // Half used -> half white.
    assert_eq!(
        icon_for_context_window_usage(0.5, false),
        Icon::ContextRemaining50
    );
    // Heavily used (the original report's 88%): little remaining -> mostly grey.
    assert_eq!(
        icon_for_context_window_usage(0.88, false),
        Icon::ContextRemaining10
    );
}

#[test]
fn mapping_is_monotonic_more_usage_never_brightens_the_circle() {
    // As usage increases, the number of bright (remaining) marks must be
    // non-increasing — the circle only ever empties as context fills.
    let icon_rank = |usage: f32| match icon_for_context_window_usage(usage, false) {
        Icon::ContextRemaining0 => 0,
        Icon::ContextRemaining10 => 10,
        Icon::ContextRemaining20 => 20,
        Icon::ContextRemaining30 => 30,
        Icon::ContextRemaining40 => 40,
        Icon::ContextRemaining50 => 50,
        Icon::ContextRemaining60 => 60,
        Icon::ContextRemaining70 => 70,
        Icon::ContextRemaining80 => 80,
        Icon::ContextRemaining90 => 90,
        Icon::ContextRemaining100 => 100,
        other => panic!("unexpected icon: {other:?}"),
    };

    let mut usage = 0.0;
    let mut previous = icon_rank(usage);
    while usage <= 1.0 {
        let current = icon_rank(usage);
        assert!(
            current <= previous,
            "icon brightness increased as usage rose to {usage}: {previous} -> {current}"
        );
        previous = current;
        usage += 0.05;
    }
}
