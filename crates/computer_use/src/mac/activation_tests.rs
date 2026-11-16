use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use objc2::rc::Retained;
use objc2_core_foundation::CGPoint;
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventSource, CGEventSourceStateID, CGEventType, CGMouseButton,
};

use super::*;

fn untapped_session(owner: Option<&str>) -> ActiveSession {
    ActiveSession {
        suppress: Arc::new(AtomicBool::new(true)),
        stop: Arc::new(AtomicBool::new(false)),
        thread: None,
        has_taps: false,
        previous: None,
        owner: owner.map(str::to_owned),
    }
}

/// Builds a `TapContext` for the given activation window, tap kind, and suppression state.
fn tap_context(window_number: i64, is_previous: bool, suppress: bool) -> TapContext {
    TapContext {
        suppress: Arc::new(AtomicBool::new(suppress)),
        is_previous,
        window_number,
    }
}

/// Creates a synthetic `CGEvent` and stamps the private window-addressing pair (field 51/58)
/// plus the standard mouse window-under-pointer fields (91/92) exactly as
/// `post_appkit_activation` / `post_window_mouse_event` do. Pass `None` for any field to leave
/// it unset (its default zero value), which lets the tests express the missing/invalid matrix.
///
/// The event is built as a mouse event because the window-under-pointer fields (91/92) only
/// persist on mouse events — on a generic `CGEvent::new` event the setters are silently ignored
/// and reads return 0. The callback under test never reads the event's own type (it receives
/// the tapped `CGEventType` separately), so the mouse type does not affect the matrix.
fn synthetic_focus_event(
    field_51_window: Option<i64>,
    field_58_valid: Option<i64>,
    field_91_window: Option<i64>,
    field_92_window: Option<i64>,
) -> Retained<CGEvent> {
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState);
    let event = CGEvent::new_mouse_event(
        source.as_deref(),
        CGEventType::MouseMoved,
        CGPoint { x: 0.0, y: 0.0 },
        CGMouseButton::Left,
    )
    .expect("creating a synthetic CGEvent for the callback matrix must succeed");
    if let Some(window) = field_51_window {
        CGEvent::set_integer_value_field(Some(&event), FIELD_WINDOW_NUMBER, window);
    }
    if let Some(valid) = field_58_valid {
        CGEvent::set_integer_value_field(Some(&event), FIELD_WINDOW_NUMBER_VALID, valid);
    }
    if let Some(window) = field_91_window {
        CGEvent::set_integer_value_field(
            Some(&event),
            CGEventField::MouseEventWindowUnderMousePointer,
            window,
        );
    }
    if let Some(window) = field_92_window {
        CGEvent::set_integer_value_field(
            Some(&event),
            CGEventField::MouseEventWindowUnderMousePointerThatCanHandleThisEvent,
            window,
        );
    }
    event.into()
}

/// Regression guard for the background-computer-use focus-stuck bug: ending a session must remove
/// that owner's `(pid, window_number)` entries from the registry (so the next `ensure_activated`
/// re-primes rather than hitting the "already activated" no-op), while leaving other owners'
/// concurrent sessions intact.
#[test]
fn end_sessions_for_owner_clears_only_that_owners_entries() {
    // Target our own process so any `ApplicationDeactivated` post is harmless; use distinct
    // window numbers per fake entry so keys don't collide.
    let pid = std::process::id() as libc::pid_t;
    let key_a = (pid, i64::MAX);
    let key_b = (pid, i64::MAX - 1);
    {
        let mut registry = registry().lock().unwrap();
        registry.insert(key_a, untapped_session(Some("conversation-a")));
        registry.insert(key_b, untapped_session(Some("conversation-b")));
    }

    end_sessions_for_owner("conversation-a");

    {
        let registry = registry().lock().unwrap();
        // Owner A's entry is gone so a restart re-primes; owner B's concurrent session survives.
        assert!(!registry.contains_key(&key_a));
        assert!(registry.contains_key(&key_b));
    }

    // Idempotent: ending an owner with no active session is a harmless no-op.
    end_sessions_for_owner("conversation-a");
    assert!(registry().lock().unwrap().contains_key(&key_b));

    // Clean up the surviving entry so the process-global registry doesn't leak across tests.
    end_sessions_for_owner("conversation-b");
    assert!(!registry().lock().unwrap().contains_key(&key_b));
}

/// The activation window under test in the callback matrix. Any non-zero value stands in for a
/// real window number; a different value (`ACTIVATION_WINDOW + 1`) represents another window on
/// the same previously-frontmost process (e.g. another Warp window the user clicks).
const ACTIVATION_WINDOW: i64 = 4242;
const OTHER_WINDOW: i64 = ACTIVATION_WINDOW + 1;

/// Regression for APP-4902: the per-PID tap callback must drop only focus events addressed to
/// the session's exact activation window, and pass through focus events for every other window
/// on the same previously-frontmost process. Against the baseline (which dropped every focus
/// event when `is_previous && suppress`), the "different window" assertions fail.
#[test]
fn tap_callback_filters_focus_by_activation_window() {
    let previous = tap_context(ACTIVATION_WINDOW, true, true);
    let target = tap_context(ACTIVATION_WINDOW, false, true);

    for &raw in FOCUS_EVENT_TYPES.iter() {
        let event_type = CGEventType(raw);

        // The activation transition itself: validly addressed to the activation window, with
        // no competing standard-window fields. Dropped on the previous tap, passed on the target.
        let activation = synthetic_focus_event(Some(ACTIVATION_WINDOW), Some(1), None, None);
        assert!(
            is_activation_focus_transition(&previous, event_type, &activation),
            "previous tap must drop focus type {raw} addressed to the activation window"
        );
        assert!(
            !is_activation_focus_transition(&target, event_type, &activation),
            "target tap must never suppress, even for its own activation window (type {raw})"
        );

        // A focus event for a *different* window on the same previous process — the APP-4902
        // symptom. Must pass through on both taps so the user's click reaches that window.
        let other_window = synthetic_focus_event(Some(OTHER_WINDOW), Some(1), None, None);
        assert!(
            !is_activation_focus_transition(&previous, event_type, &other_window),
            "previous tap must pass focus type {raw} for a different window (APP-4902 fix)"
        );
        assert!(!is_activation_focus_transition(
            &target,
            event_type,
            &other_window
        ));
    }
}

/// Window-addressing matrix (spec validation criterion #3): the callback drops only when the
/// private field-51/field-58 pair validly identifies the activation window. A different field
/// 51, a cleared/invalid field 58, or a standard 91/92 field naming another window all pass.
#[test]
fn tap_callback_addressing_matrix_passes_non_matching_identity() {
    let previous = tap_context(ACTIVATION_WINDOW, true, true);
    let event_type = CGEventType(13);

    // Baseline: valid pair addressed to the activation window is the drop case.
    let activation = synthetic_focus_event(Some(ACTIVATION_WINDOW), Some(1), None, None);
    assert!(is_activation_focus_transition(
        &previous,
        event_type,
        &activation
    ));

    // Field 51 naming any other window passes through.
    let different_51 = synthetic_focus_event(Some(OTHER_WINDOW), Some(1), None, None);
    assert!(!is_activation_focus_transition(
        &previous,
        event_type,
        &different_51
    ));

    // Field 51 missing entirely (zero) passes through.
    let missing_51 = synthetic_focus_event(None, Some(1), None, None);
    assert!(!is_activation_focus_transition(
        &previous,
        event_type,
        &missing_51
    ));

    // Field 58 cleared (0) passes through even when field 51 matches: the private routing field
    // is not flagged valid, so the identity is untrustworthy.
    let cleared_58 = synthetic_focus_event(Some(ACTIVATION_WINDOW), Some(0), None, None);
    assert!(!is_activation_focus_transition(
        &previous,
        event_type,
        &cleared_58
    ));

    // Field 58 missing entirely passes through.
    let missing_58 = synthetic_focus_event(Some(ACTIVATION_WINDOW), None, None, None);
    assert!(!is_activation_focus_transition(
        &previous,
        event_type,
        &missing_58
    ));

    // Standard field 91 naming a different window passes through even when 51/58 match: the
    // event is provably not this session's activation transition.
    let std_91_other =
        synthetic_focus_event(Some(ACTIVATION_WINDOW), Some(1), Some(OTHER_WINDOW), None);
    assert!(!is_activation_focus_transition(
        &previous,
        event_type,
        &std_91_other
    ));

    // Standard field 92 naming a different window passes through.
    let std_92_other =
        synthetic_focus_event(Some(ACTIVATION_WINDOW), Some(1), None, Some(OTHER_WINDOW));
    assert!(!is_activation_focus_transition(
        &previous,
        event_type,
        &std_92_other
    ));

    // Standard 91/92 naming the *same* window corroborate the identity and still drop.
    let std_same = synthetic_focus_event(
        Some(ACTIVATION_WINDOW),
        Some(1),
        Some(ACTIVATION_WINDOW),
        Some(ACTIVATION_WINDOW),
    );
    assert!(is_activation_focus_transition(
        &previous, event_type, &std_same
    ));
}

/// Non-focus and target-tap pass-through (spec validation criterion #4): mouse, keyboard, and
/// other non-focus event types pass unchanged even when addressed to the activation window; all
/// focus types pass when `is_previous=false`; everything passes when suppression is disabled.
#[test]
fn tap_callback_passes_non_focus_target_tap_and_suppression_off() {
    let previous = tap_context(ACTIVATION_WINDOW, true, true);
    let target = tap_context(ACTIVATION_WINDOW, false, true);
    let previous_unsuppressed = tap_context(ACTIVATION_WINDOW, true, false);

    let activation = synthetic_focus_event(Some(ACTIVATION_WINDOW), Some(1), None, None);

    // Non-focus event types are returned unchanged even when fully addressed to the activation
    // window. Use a representative spread of real CGEventType values.
    for raw in [
        CGEventType::LeftMouseDown.0,
        CGEventType::MouseMoved.0,
        CGEventType::KeyDown.0,
        CGEventType::KeyUp.0,
        CGEventType::FlagsChanged.0,
        0,
        999,
    ] {
        let non_focus = CGEventType(raw);
        assert!(
            !is_activation_focus_transition(&previous, non_focus, &activation),
            "non-focus type {raw} must never be suppressed"
        );
    }

    // Every focus type passes on the target tap (is_previous=false).
    for &raw in FOCUS_EVENT_TYPES.iter() {
        assert!(!is_activation_focus_transition(
            &target,
            CGEventType(raw),
            &activation
        ));
    }

    // Every focus type passes when suppression is disabled (the teardown/legacy state).
    for &raw in FOCUS_EVENT_TYPES.iter() {
        assert!(!is_activation_focus_transition(
            &previous_unsuppressed,
            CGEventType(raw),
            &activation
        ));
    }
}

/// Concurrent-session isolation (spec validation criterion #5): two tap contexts with distinct
/// activation window numbers each drop only their own addressed activation transition and pass
/// the other's. The existing owner-isolation test above covers registry teardown coexistence.
#[test]
fn tap_callback_concurrent_sessions_isolate_by_activation_window() {
    let context_a = tap_context(ACTIVATION_WINDOW, true, true);
    let context_b = tap_context(OTHER_WINDOW, true, true);
    let event_type = CGEventType(19);

    let event_a = synthetic_focus_event(Some(ACTIVATION_WINDOW), Some(1), None, None);
    let event_b = synthetic_focus_event(Some(OTHER_WINDOW), Some(1), None, None);

    // Each context drops only the event addressed to its own activation window.
    assert!(is_activation_focus_transition(
        &context_a, event_type, &event_a
    ));
    assert!(!is_activation_focus_transition(
        &context_a, event_type, &event_b
    ));
    assert!(is_activation_focus_transition(
        &context_b, event_type, &event_b
    ));
    assert!(!is_activation_focus_transition(
        &context_b, event_type, &event_a
    ));
}
