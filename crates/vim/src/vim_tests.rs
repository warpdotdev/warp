use super::*;

/// Feed a sequence of characters to a fresh state machine and return the last
/// event it emitted, mirroring how keystrokes arrive one at a time.
fn last_event(keys: &str) -> Option<VimEvent> {
    let mut fsa = VimFSA::new();
    fsa.mode = VimMode::Normal;
    let mut last = None;
    for c in keys.chars() {
        if let Some(event) = fsa.typed_character(c) {
            last = Some(event);
        }
    }
    last
}

#[test]
fn gg_without_a_count_jumps_to_the_first_line() {
    let event = last_event("gg").expect("gg should emit an event");
    assert!(
        matches!(
            event.event_type,
            VimEventType::Navigate(VimMotion::JumpToFirstLine)
        ),
        "expected `gg` to jump to the first line, got {:?}",
        event.event_type
    );
}

#[test]
fn count_gg_jumps_to_that_line_in_normal_mode() {
    let event = last_event("5gg").expect("5gg should emit an event");
    assert!(
        matches!(
            event.event_type,
            VimEventType::Navigate(VimMotion::JumpToLine(5))
        ),
        "expected `5gg` to jump to line 5, got {:?}",
        event.event_type
    );
}

#[test]
fn count_gg_and_count_g_agree() {
    // `Ngg` and `NG` are the same motion in Vim; they should produce the same event.
    let gg = last_event("12gg").expect("12gg should emit an event");
    let g = last_event("12G").expect("12G should emit an event");
    assert!(
        matches!(
            gg.event_type,
            VimEventType::Navigate(VimMotion::JumpToLine(12))
        ),
        "expected `12gg` to jump to line 12, got {:?}",
        gg.event_type
    );
    assert!(
        matches!(
            g.event_type,
            VimEventType::Navigate(VimMotion::JumpToLine(12))
        ),
        "expected `12G` to jump to line 12, got {:?}",
        g.event_type
    );
}

#[test]
fn count_gg_jumps_to_that_line_in_visual_mode() {
    let event = last_event("v5gg").expect("v5gg should emit an event");
    assert!(
        matches!(
            event.event_type,
            VimEventType::Navigate(VimMotion::JumpToLine(5))
        ),
        "expected `v5gg` to extend the selection to line 5, got {:?}",
        event.event_type
    );
}

#[test]
fn operator_count_gg_uses_that_line_as_the_motion() {
    // `d3gg` deletes linewise from the cursor to line 3.
    let event = last_event("d3gg").expect("d3gg should emit an event");
    assert!(
        matches!(
            event.event_type,
            VimEventType::Operation {
                operand: VimOperand::Motion {
                    motion: VimMotion::JumpToLine(3),
                    ..
                },
                ..
            }
        ),
        "expected `d3gg` to delete to line 3, got {:?}",
        event.event_type
    );
}
