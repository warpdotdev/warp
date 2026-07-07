use std::time::Duration;

use computer_use::{
    Action, Key, OverlayKind, ScrollDirection, ScrollDistance, TargetedAction, Vector2I,
};

use super::{key_label_from_summary, overlay_entry_for};

fn screen(action: Action) -> TargetedAction {
    TargetedAction::screen(action)
}

#[test]
fn key_actions_map_to_key_pill_with_summary_label() {
    let actions = vec![
        screen(Action::KeyDown {
            key: Key::Char('a'),
        }),
        screen(Action::KeyUp {
            key: Key::Char('a'),
        }),
    ];
    assert_eq!(
        overlay_entry_for(&actions, "Key \"ctrl+a\""),
        Some((OverlayKind::Key, "ctrl+a".to_string()))
    );
}

#[test]
fn type_actions_map_to_generic_typing_and_never_leak_the_payload() {
    let actions = vec![screen(Action::TypeText {
        text: "super-secret-password".to_string(),
    })];
    let entry = overlay_entry_for(&actions, "Type \"super-secret-password\"");
    assert_eq!(
        entry,
        Some((OverlayKind::Type, "typing\u{2026}".to_string()))
    );
    let (_, label) = entry.unwrap();
    assert!(
        !label.contains("secret"),
        "typed payload must never surface"
    );
}

#[test]
fn pointer_scroll_and_noop_actions_produce_no_entry() {
    let cases = [
        Action::MouseMove {
            to: Vector2I::new(3, 4),
        },
        Action::MouseWheel {
            at: Vector2I::new(0, 0),
            direction: ScrollDirection::Down,
            distance: ScrollDistance::Clicks(3),
        },
        Action::Wait(Duration::from_millis(0)),
    ];
    for action in cases {
        assert_eq!(overlay_entry_for(&[screen(action)], "irrelevant"), None);
    }
}

#[test]
fn key_label_falls_back_when_summary_is_unquoted_or_empty() {
    assert_eq!(key_label_from_summary("Key cmd+c"), "Key cmd+c");
    assert_eq!(key_label_from_summary(""), "key");
}
