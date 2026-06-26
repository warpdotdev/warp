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
        Icon::ContextRemaining100
    );
}

#[test]
fn full_context_window_shows_all_grey_circle() {
    // 100% used == 0% remaining -> all-grey circle.
    assert_eq!(icon_for_context_window_usage(1.0), Icon::ContextRemaining0);
}

#[test]
fn icon_brightness_tracks_remaining_not_used() {
    // Lightly-used conversation: lots of context remaining -> mostly white.
    assert_eq!(icon_for_context_window_usage(0.1), Icon::ContextRemaining90);
    // Half used -> half white.
    assert_eq!(icon_for_context_window_usage(0.5), Icon::ContextRemaining50);
    // Heavily used (the original report's 88%): little remaining -> mostly grey.
    assert_eq!(
        icon_for_context_window_usage(0.88),
        Icon::ContextRemaining10
    );
}

#[test]
fn mapping_is_monotonic_more_usage_never_brightens_the_circle() {
    // As usage increases, the number of bright (remaining) marks must be
    // non-increasing — the circle only ever empties as context fills.
    let icon_rank = |usage: f32| match icon_for_context_window_usage(usage) {
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
