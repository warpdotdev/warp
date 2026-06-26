//! Tests for the context-window usage circle icon mapping.
//!
//! Regression guard for the color semantics of the context-window circle:
//! the solid (white) marks represent the context *remaining*, not the amount
//! used. An empty conversation (0% used → 100% remaining) shows a full white
//! circle and it counts down to an all-grey circle as the window fills up
//! (100% used → 0% remaining).

use warp_core::ui::Icon;

use super::icon_for_context_window_usage;

#[test]
fn empty_conversation_shows_full_white_circle() {
    // 0% used == 100% remaining -> all-white circle.
    assert_eq!(
        icon_for_context_window_usage(0.0),
        Icon::ConversationContext100
    );
}

#[test]
fn full_context_window_shows_all_grey_circle() {
    // 100% used == 0% remaining -> all-grey circle.
    assert_eq!(
        icon_for_context_window_usage(1.0),
        Icon::ConversationContext0
    );
}

#[test]
fn icon_brightness_tracks_remaining_not_used() {
    // Lightly-used conversation: lots of context remaining -> mostly white.
    assert_eq!(
        icon_for_context_window_usage(0.1),
        Icon::ConversationContext90
    );
    // Half used -> half white.
    assert_eq!(
        icon_for_context_window_usage(0.5),
        Icon::ConversationContext50
    );
    // Heavily used (the original report's 88%): little remaining -> mostly grey.
    assert_eq!(
        icon_for_context_window_usage(0.88),
        Icon::ConversationContext10
    );
}

#[test]
fn mapping_is_monotonic_more_usage_never_brightens_the_circle() {
    // As usage increases, the number of bright (remaining) marks must be
    // non-increasing — the circle only ever empties as context fills.
    let icon_rank = |usage: f32| match icon_for_context_window_usage(usage) {
        Icon::ConversationContext0 => 0,
        Icon::ConversationContext10 => 10,
        Icon::ConversationContext20 => 20,
        Icon::ConversationContext30 => 30,
        Icon::ConversationContext40 => 40,
        Icon::ConversationContext50 => 50,
        Icon::ConversationContext60 => 60,
        Icon::ConversationContext70 => 70,
        Icon::ConversationContext80 => 80,
        Icon::ConversationContext90 => 90,
        Icon::ConversationContext100 => 100,
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
