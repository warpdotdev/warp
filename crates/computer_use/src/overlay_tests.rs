use std::time::Duration;

use super::{ActionLogEntry, OverlayKind, build_overlay_ass};

fn entry(offset_ms: u64, kind: OverlayKind, label: &str, dur_ms: u64) -> ActionLogEntry {
    ActionLogEntry {
        offset: Duration::from_millis(offset_ms),
        kind,
        label: label.to_string(),
        show_duration: Duration::from_millis(dur_ms),
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
    let ass = build_overlay_ass(
        &[entry(1000, OverlayKind::Key, "ctrl+a", 1500)],
        (1920, 1080),
    );
    assert!(ass.contains("PlayResX: 1920"));
    assert!(ass.contains("PlayResY: 1080"));
    assert!(ass.contains("Style: Pill,DejaVu Sans,48"));
    // BorderStyle=3, Outline=16, Shadow=0, Alignment=2 (bottom-center), margins.
    assert!(ass.contains(",3,16,0,2,40,40,90,1"));
    assert!(ass.contains("Dialogue: 0,0:00:01.00,0:00:02.50,Pill,,0,0,0,,ctrl+a"));
}

#[test]
fn pill_end_is_clamped_to_next_entry_start() {
    let entries = vec![
        entry(1000, OverlayKind::Key, "ctrl+a", 1500),
        entry(2000, OverlayKind::Type, "typing…", 1500),
    ];
    let ass = build_overlay_ass(&entries, (1280, 720));
    // First pill would run to 2.5s but is cut to the next pill's 2.0s start.
    assert!(
        ass.contains("Dialogue: 0,0:00:01.00,0:00:02.00,Pill,,0,0,0,,ctrl+a"),
        "{ass}"
    );
    assert!(
        ass.contains("Dialogue: 0,0:00:02.00,0:00:03.50,Pill,,0,0,0,,typing…"),
        "{ass}"
    );
}

#[test]
fn entries_are_ordered_by_timecode() {
    let entries = vec![
        entry(3000, OverlayKind::Type, "typing…", 1000),
        entry(1000, OverlayKind::Key, "ctrl+a", 1000),
    ];
    let ass = build_overlay_ass(&entries, (1280, 720));
    let first = ass.find("ctrl+a").unwrap();
    let second = ass.find("typing…").unwrap();
    assert!(
        first < second,
        "earlier offset should be emitted first\n{ass}"
    );
}
