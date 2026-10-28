use super::*;

#[test]
fn initial_bridge_focus_suppresses_the_resulting_canvas_blur() {
    // The desktop text-input bridge stealing focus for the first time (e.g. an editable text
    // caret just became active) blurs the canvas. The document itself never lost focus, so this
    // must not be treated as the window/app going inactive.
    assert!(is_internal_focus_handoff(false, true, true));
}

#[test]
fn click_or_move_within_an_editable_surface_refocusing_the_canvas_is_not_suppressed() {
    // A click/move within an already-editable surface momentarily refocuses the canvas before
    // the bridge steals focus back on the next sync. The canvas regaining focus is a genuine
    // `Focused(true)` and must be recorded normally (it is a no-op if the app was never actually
    // marked inactive, since the preceding blur was correctly suppressed).
    assert!(!is_internal_focus_handoff(true, true, false));
}

#[test]
fn click_to_a_non_text_area_refocusing_the_canvas_is_not_suppressed() {
    // Clicking a non-editable part of the canvas permanently reclaims focus from the bridge
    // (no active caret to hand focus back for). Same signal as the click-within-an-editable-
    // surface case: `Focused(true)` while the document itself is still focused.
    assert!(!is_internal_focus_handoff(true, true, false));
}

#[test]
fn a_real_tab_or_window_blur_is_not_suppressed() {
    // Alt-tabbing away or switching browser tabs clears `document.hasFocus()` too, so the
    // resulting `Focused(false)` must still be treated as a genuine loss of focus.
    assert!(!is_internal_focus_handoff(false, false, true));
}

#[test]
fn focus_moving_to_an_unrelated_dom_element_is_not_suppressed() {
    assert!(!is_internal_focus_handoff(false, true, false));
}
