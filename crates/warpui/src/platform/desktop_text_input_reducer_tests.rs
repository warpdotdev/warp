use super::*;
use crate::event::Event;

fn key_payload(key: &str, code: &str) -> DesktopKeyboardPayload {
    DesktopKeyboardPayload {
        kind: DomKeyEventKind::Down,
        key: key.to_string(),
        code: code.to_string(),
        ctrl: false,
        alt: false,
        shift: false,
        meta: false,
        is_composing: false,
    }
}

#[test]
fn unmodified_printable_key_produces_keydown_and_fallback_chars() {
    let payload = key_payload("a", "KeyA");
    let Some(KeyConversion::Down { event, chars }) = convert_key(&payload) else {
        panic!("expected a Down conversion");
    };
    let Event::KeyDown {
        keystroke,
        chars: key_down_chars,
        ..
    } = event
    else {
        panic!("expected a KeyDown event");
    };
    assert_eq!(keystroke.key, "a");
    assert!(!keystroke.has_any_modifier());
    assert_eq!(key_down_chars, "a");
    assert_eq!(chars.as_deref(), Some("a"));
}

#[test]
fn shifted_letter_uppercases_the_keystroke_key() {
    let mut payload = key_payload("A", "KeyA");
    payload.shift = true;
    let Some(KeyConversion::Down { event, .. }) = convert_key(&payload) else {
        panic!("expected a Down conversion");
    };
    let Event::KeyDown { keystroke, .. } = event else {
        panic!("expected a KeyDown event");
    };
    assert_eq!(keystroke.key, "A");
    assert!(keystroke.shift);
}

#[test]
fn ctrl_c_carries_the_control_byte_on_keydown_but_not_in_fallback_chars() {
    let mut payload = key_payload("c", "KeyC");
    payload.ctrl = true;
    let Some(KeyConversion::Down { event, chars }) = convert_key(&payload) else {
        panic!("expected a Down conversion");
    };
    let Event::KeyDown {
        keystroke,
        chars: key_down_chars,
        ..
    } = event
    else {
        panic!("expected a KeyDown event");
    };
    assert_eq!(keystroke.key, "c");
    assert!(keystroke.ctrl);
    // Raw terminal input reads this field directly to write the interrupt byte to the pty.
    assert_eq!(key_down_chars, "\u{3}");
    // The unhandled-keydown fallback should never insert the literal "c" instead.
    assert_eq!(chars.as_deref(), Some("c"));
}

#[test]
fn named_key_produces_keydown_with_no_fallback_chars() {
    let payload = key_payload("Enter", "Enter");
    let Some(KeyConversion::Down { event, chars }) = convert_key(&payload) else {
        panic!("expected a Down conversion");
    };
    let Event::KeyDown { keystroke, .. } = event else {
        panic!("expected a KeyDown event");
    };
    assert_eq!(keystroke.key, "enter");
    assert_eq!(chars, None);
}

#[test]
fn extended_function_key_f13_is_forwarded() {
    // The canvas keyboard path supports F1-F35; the bridge must not silently drop the upper
    // half of that range once it owns focus.
    let payload = key_payload("F13", "F13");
    let Some(KeyConversion::Down { event, .. }) = convert_key(&payload) else {
        panic!("expected a Down conversion");
    };
    let Event::KeyDown { keystroke, .. } = event else {
        panic!("expected a KeyDown event");
    };
    assert_eq!(keystroke.key, "f13");
}

#[test]
fn composing_key_is_left_to_the_browser() {
    let mut payload = key_payload("a", "KeyA");
    payload.is_composing = true;
    assert!(convert_key(&payload).is_none());
}

// `KEYS_TO_IGNORE` only contains the cmd/ctrl-v entry this test exercises under the wasm target
// (see `platform::KEYS_TO_IGNORE`); on native targets the desktop bridge's `convert_key` is never
// actually invoked (only `wasm::desktop_text_input` calls it), so the set is intentionally empty
// there and this assertion would not hold.
#[test]
#[cfg(target_family = "wasm")]
fn browser_paste_shortcut_is_left_to_the_browser() {
    let mut payload = key_payload("v", "KeyV");
    if crate::platform::OperatingSystem::get().is_mac() {
        payload.meta = true;
    } else {
        payload.ctrl = true;
    }
    assert!(convert_key(&payload).is_none());
}

#[test]
fn modifier_keydown_and_keyup_report_press_and_release() {
    let down = DesktopKeyboardPayload {
        kind: DomKeyEventKind::Down,
        shift: true,
        ..key_payload("Shift", "ShiftLeft")
    };
    let Some(KeyConversion::ModifierChanged { key_code, state }) = convert_key(&down) else {
        panic!("expected a ModifierChanged conversion");
    };
    assert_eq!(key_code, crate::platform::keyboard::KeyCode::ShiftLeft);
    assert!(matches!(state, crate::event::KeyState::Pressed));

    let up = DesktopKeyboardPayload {
        kind: DomKeyEventKind::Up,
        shift: false,
        ..key_payload("Shift", "ShiftLeft")
    };
    let Some(KeyConversion::ModifierChanged { state, .. }) = convert_key(&up) else {
        panic!("expected a ModifierChanged conversion");
    };
    assert!(matches!(state, crate::event::KeyState::Released));
}

#[test]
fn unmodified_non_modifier_keyup_is_ignored() {
    let payload = DesktopKeyboardPayload {
        kind: DomKeyEventKind::Up,
        ..key_payload("a", "KeyA")
    };
    assert!(convert_key(&payload).is_none());
}

#[test]
fn classify_input_type_covers_insert_and_delete_directions() {
    assert_eq!(
        classify_input_type("insertText"),
        InputClassification::Insert
    );
    assert_eq!(
        classify_input_type("insertCompositionText"),
        InputClassification::Insert
    );
    assert_eq!(
        classify_input_type("deleteContentBackward"),
        InputClassification::Delete(DeleteDirection::Backward)
    );
    assert_eq!(
        classify_input_type("deleteContentForward"),
        InputClassification::Delete(DeleteDirection::Forward)
    );
    assert_eq!(
        classify_input_type("formatBold"),
        InputClassification::Unsupported
    );
}

#[test]
fn classify_input_type_treats_empty_or_unrecognized_types_as_a_potential_insert() {
    // Tools that mutate `.value` directly and dispatch a generic `Event` (rather than a real
    // `InputEvent`) report no `inputType` at all; the bridge must still attempt the
    // sentinel-diff fallback for them instead of silently dropping the insertion.
    assert_eq!(classify_input_type(""), InputClassification::Insert);
    assert_eq!(
        classify_input_type("someFutureInputType"),
        InputClassification::Insert
    );
}

#[test]
fn extract_inserted_text_diffs_against_the_sentinel() {
    assert_eq!(
        extract_inserted_text(SENTINEL, " hello"),
        Some("hello".to_string())
    );
    assert_eq!(extract_inserted_text(SENTINEL, " "), None);
}

#[test]
fn extract_inserted_text_treats_a_full_replacement_without_the_sentinel_as_inserted_text() {
    // A dictation tool (e.g. MacWhisper) that fully replaces `.value` - rather than appending
    // after the sentinel - and dispatches a generic `input` event with no `InputEvent.data` must
    // still be recognized as an insertion instead of being silently dropped. This is the primary
    // scenario the desktop text-input bridge exists to support.
    assert_eq!(
        extract_inserted_text(SENTINEL, "transcription"),
        Some("transcription".to_string())
    );
    // An empty replacement (nothing was actually inserted) still yields no insertion.
    assert_eq!(extract_inserted_text(SENTINEL, ""), None);
}

#[test]
fn generic_full_replacement_dictation_produces_exactly_one_insertion() {
    // Mirrors the desktop bridge's `input` listener end-to-end at the reducer level: an
    // unrecognized/absent `inputType` first classifies as a potential insert, then the fallback
    // extracts the inserted text from the textarea's raw value when `InputEvent.data` is absent.
    // Together these must resolve to exactly one inserted-text event (and therefore exactly one
    // `TypedCharacters` dispatch), not zero.
    assert_eq!(classify_input_type(""), InputClassification::Insert);
    assert_eq!(
        extract_inserted_text(SENTINEL, "transcription"),
        Some("transcription".to_string())
    );
}

#[test]
fn composition_selection_range_strips_the_sentinel_and_clamps() {
    // Sentinel is 1 UTF-16 code unit; marked text is "ab" (2 units).
    assert_eq!(composition_selection_range(1, 2, 1, 3), 0..2);
    // A stale selection that runs past the marked text clamps to its end.
    assert_eq!(composition_selection_range(1, 2, 1, 10), 0..2);
    // A selection that hasn't caught up with the sentinel clamps to zero.
    assert_eq!(composition_selection_range(1, 2, 0, 0), 0..0);
}

#[test]
fn composition_commit_suppresses_exactly_the_trailing_input_event() {
    let mut tracker = CompositionTracker::default();
    tracker.on_composition_start();
    assert!(tracker.is_composing());

    let action = tracker.on_composition_end("こんにちは".to_string());
    assert_eq!(action, CompositionAction::Commit("こんにちは".to_string()));
    assert!(!tracker.is_composing());

    // The browser's matching post-`compositionend` `input` event must be dropped, not dispatched
    // as a second insertion of the same text.
    assert!(tracker.should_ignore_input_event(false, Some("こんにちは")));

    // Suppression is one-shot: a later, unrelated `input` event must not also be swallowed.
    assert!(!tracker.should_ignore_input_event(false, Some("more")));
}

#[test]
fn a_non_matching_input_event_after_a_commit_is_not_dropped() {
    // A regression test for trading a double-insert for a dropped insert: if the trailing
    // `input` event's data doesn't match what was just committed (a different browser ordering,
    // or a distinct direct-input event arriving instead of the expected duplicate), it must be
    // treated as a real, separate insertion rather than silently swallowed.
    let mut tracker = CompositionTracker::default();
    tracker.on_composition_start();
    tracker.on_composition_end("hello".to_string());

    assert!(!tracker.should_ignore_input_event(false, Some("world")));
    // The pending suppression is one-shot and was already consumed above, so a subsequent event
    // carrying the originally-committed text is no longer treated as the (already-passed)
    // trailing duplicate either.
    assert!(!tracker.should_ignore_input_event(false, Some("hello")));
}

#[test]
fn an_input_event_with_no_data_after_a_commit_is_not_dropped() {
    // A tool that mutates `.value` directly and fires a generic `input` event carries no
    // `InputEvent.data`; it must not be mistaken for the trailing composition duplicate.
    let mut tracker = CompositionTracker::default();
    tracker.on_composition_start();
    tracker.on_composition_end("hello".to_string());
    assert!(!tracker.should_ignore_input_event(false, None));
}

#[test]
fn cancelled_composition_does_not_suppress_the_next_input_event() {
    let mut tracker = CompositionTracker::default();
    tracker.on_composition_start();

    let action = tracker.on_composition_end(String::new());
    assert_eq!(action, CompositionAction::Cancel);
    assert!(!tracker.should_ignore_input_event(false, None));
}

#[test]
fn input_events_are_ignored_while_composing() {
    let mut tracker = CompositionTracker::default();
    tracker.on_composition_start();
    assert!(tracker.should_ignore_input_event(false, None));
    // Also honors the DOM InputEvent's own isComposing flag, independent of tracked state.
    let mut idle_tracker = CompositionTracker::default();
    assert!(idle_tracker.should_ignore_input_event(true, None));
}

#[test]
fn reset_cancels_an_in_progress_composition_and_clears_suppression() {
    let mut tracker = CompositionTracker::default();
    tracker.on_composition_start();
    assert_eq!(tracker.reset(), Some(CompositionAction::Cancel));
    assert!(!tracker.is_composing());
    // The old surface's composition is gone; a fresh input event on the new surface must not be
    // suppressed by state left over from the old one.
    assert!(!tracker.should_ignore_input_event(false, None));
}

#[test]
fn reset_is_a_no_op_when_not_composing() {
    let mut tracker = CompositionTracker::default();
    assert_eq!(tracker.reset(), None);
}

#[test]
fn a_stale_pending_commit_does_not_survive_a_later_cancelled_composition() {
    // A regression test for a stale suppression outliving the commit that armed it: if a
    // commit's own trailing `input` event never arrives (e.g. a browser quirk), and a later,
    // distinct composition is then cancelled before producing any text, the earlier suppression
    // must not still be sitting there waiting to swallow an unrelated `input` event.
    let mut tracker = CompositionTracker::default();
    tracker.on_composition_start();
    let action = tracker.on_composition_end("a".to_string());
    assert_eq!(action, CompositionAction::Commit("a".to_string()));
    // The trailing `input` event for this commit never arrives.

    tracker.on_composition_start();
    let action = tracker.on_composition_end(String::new());
    assert_eq!(action, CompositionAction::Cancel);

    // A later, ordinary `input` event carrying the same text as the earlier commit must be
    // treated as a real, separate insertion - not mistaken for that commit's already-passed
    // (and never-arrived) trailing duplicate.
    assert!(!tracker.should_ignore_input_event(false, Some("a")));
}
