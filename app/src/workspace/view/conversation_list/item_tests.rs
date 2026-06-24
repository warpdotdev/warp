use super::*;

/// Regression test for APP-4253: conversation list rows must not respond to
/// hover (which drives the selection highlight and the row tooltip) while they
/// are covered by an overlay such as a modal. Both row renderers share
/// `ROW_MOUSE_IN_BEHAVIOR`, which opts out of `fire_when_covered`; the framework
/// honors that flag (see
/// `warpui_core::elements::gui::event_handler_tests::test_mouse_in_behavior_dont_fire_when_covered`).
/// Before the fix this was `true`, so hovering conversation items behind the
/// "Edit toolbar" modal still selected them and showed their tooltip.
//
// This intentionally asserts on a compile-time configuration constant, so the
// `assertions_on_constants` lint (which assumes such asserts are redundant) does
// not apply here — the assertion is the guard that keeps the constant correct.
#[test]
#[allow(clippy::assertions_on_constants)]
fn row_hover_does_not_fire_when_covered() {
    assert!(
        !ROW_MOUSE_IN_BEHAVIOR.fire_when_covered,
        "conversation list rows must not fire hover-driven selection/tooltip when covered by a modal/overlay"
    );
}
