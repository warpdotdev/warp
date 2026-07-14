use std::time::Duration;

use super::{ActionLogEntry, build_overlay_ass, overlay_labels_for};
use crate::{Action, Key, MouseButton, ScrollDirection, ScrollDistance, TargetedAction, Vector2I};

fn screen(action: Action) -> TargetedAction {
    TargetedAction::screen(action)
}

fn entry(offset_ms: u64, labels: &[&str]) -> ActionLogEntry {
    ActionLogEntry {
        offset: Duration::from_millis(offset_ms),
        labels: labels.iter().map(ToString::to_string).collect(),
    }
}

#[test]
fn maps_semantic_labels_in_action_order() {
    let ctrl = Key::Keycode(0xFFE3);
    let enter = Key::Keycode(0xFF0D);
    let actions = vec![
        screen(Action::KeyDown { key: ctrl.clone() }),
        screen(Action::KeyDown {
            key: Key::Char('a'),
        }),
        screen(Action::KeyUp {
            key: Key::Char('a'),
        }),
        screen(Action::KeyUp { key: ctrl }),
        screen(Action::TypeText {
            text: "secret".to_string(),
        }),
        screen(Action::MouseWheel {
            at: Vector2I::new(0, 0),
            direction: ScrollDirection::Down,
            distance: ScrollDistance::Clicks(3),
        }),
        screen(Action::KeyDown { key: enter.clone() }),
        screen(Action::KeyUp { key: enter }),
    ];
    assert_eq!(
        overlay_labels_for(&actions, "mixed"),
        ["ctrl+a", "typing\u{2026}", "scroll \u{2193}", "Return"]
    );
}

#[test]
fn redacts_printable_keys_and_omits_pointer_actions() {
    let printable = [
        screen(Action::KeyDown {
            key: Key::Char('p'),
        }),
        screen(Action::KeyUp {
            key: Key::Char('p'),
        }),
    ];
    assert_eq!(
        overlay_labels_for(&printable, "Key \"ctrl+p\""),
        ["typing\u{2026}"]
    );

    let omitted = [
        screen(Action::MouseMove {
            to: Vector2I::new(3, 4),
        }),
        screen(Action::MouseDown {
            button: MouseButton::Left,
            at: Vector2I::new(3, 4),
        }),
        screen(Action::MouseUp {
            button: MouseButton::Left,
        }),
        screen(Action::Wait(Duration::ZERO)),
    ];
    assert!(overlay_labels_for(&omitted, "irrelevant").is_empty());
}

#[test]
fn maps_all_scroll_directions_without_distance() {
    for (direction, label) in [
        (ScrollDirection::Up, "scroll \u{2191}"),
        (ScrollDirection::Down, "scroll \u{2193}"),
        (ScrollDirection::Left, "scroll \u{2190}"),
        (ScrollDirection::Right, "scroll \u{2192}"),
    ] {
        let actions = [screen(Action::MouseWheel {
            at: Vector2I::new(0, 0),
            direction,
            distance: ScrollDistance::Pixels(100),
        })];
        assert_eq!(overlay_labels_for(&actions, "irrelevant"), [label]);
    }
}

#[test]
fn empty_entries_produce_no_dialogue() {
    let ass = build_overlay_ass(&[], (1280, 720));
    assert!(ass.contains("[Events]"));
    assert!(!ass.contains("Dialogue:"));
}

#[test]
fn bottom_center_pill_style_and_dimensions() {
    let ass = build_overlay_ass(&[entry(1000, &["ctrl+a"])], (1920, 1080));
    assert!(ass.contains("PlayResX: 1920"));
    assert!(ass.contains("PlayResY: 1080"));
    assert!(ass.contains("Style: Pill,DejaVu Sans Mono,48"));
    assert!(
        ass.contains("Dialogue: 0,0:00:01.00,0:00:02.50,Pill,,0,0,0,,{\\an2\\pos(960,990)}ctrl+a")
    );
}

#[test]
fn labels_in_a_group_share_timing_and_position() {
    let ass = build_overlay_ass(
        &[entry(1000, &["ctrl+a", "typing…", "Return"])],
        (1920, 1080),
    );
    let dialogue_lines = ass
        .lines()
        .filter(|line| line.starts_with("Dialogue:"))
        .collect::<Vec<_>>();
    assert_eq!(dialogue_lines.len(), 3);
    assert!(
        dialogue_lines
            .iter()
            .all(|line| line.contains("0:00:01.00,0:00:02.50"))
    );
    assert!(dialogue_lines[0].contains("\\pos(715,990)}ctrl+a"));
    assert!(dialogue_lines[1].contains("\\pos(959,990)}typing…"));
    assert!(dialogue_lines[2].contains("\\pos(1204,990)}Return"));
}

#[test]
fn group_end_is_clamped_to_next_entry_start() {
    let entries = vec![
        entry(1000, &["ctrl+a", "typing…"]),
        entry(2000, &["scroll ↓"]),
    ];
    let ass = build_overlay_ass(&entries, (1280, 720));
    let first_group = ass
        .lines()
        .filter(|line| line.contains("ctrl+a") || line.contains("typing…"))
        .collect::<Vec<_>>();
    assert!(
        first_group
            .iter()
            .all(|line| line.contains("0:00:01.00,0:00:02.00")),
        "{ass}"
    );
    assert!(ass.contains("0:00:02.00,0:00:03.50"));
}

#[test]
fn entries_are_ordered_by_timecode() {
    let entries = vec![entry(3000, &["typing…"]), entry(1000, &["ctrl+a"])];
    let ass = build_overlay_ass(&entries, (1280, 720));
    assert!(ass.find("ctrl+a").unwrap() < ass.find("typing…").unwrap());
}
