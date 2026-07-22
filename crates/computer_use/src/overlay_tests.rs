use std::time::Duration;

use super::{
    ActionLogEntry, KeepSegment, build_keep_segments, build_overlay_ass,
    is_meaningful_action_group, overlay_labels_for, remap_source_interval,
};
use crate::{Action, Key, MouseButton, ScrollDirection, ScrollDistance, TargetedAction, Vector2I};

fn screen(action: Action) -> TargetedAction {
    TargetedAction::screen(action)
}

fn entry(start_ms: u64, finish_ms: u64, labels: &[&str]) -> ActionLogEntry {
    ActionLogEntry {
        offset: Duration::from_millis(start_ms),
        finish_offset: Duration::from_millis(finish_ms),
        labels: labels.iter().map(ToString::to_string).collect(),
    }
}

fn seg(source_start_ms: u64, source_end_ms: u64, output_start_ms: u64) -> KeepSegment {
    KeepSegment {
        source_start: Duration::from_millis(source_start_ms),
        source_end: Duration::from_millis(source_end_ms),
        output_start: Duration::from_millis(output_start_ms),
    }
}

const SOURCE_TEN_SECS: Duration = Duration::from_secs(10);
const FRAME_RATE_15: u32 = 15;

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
fn is_meaningful_action_group_true_for_real_interactions() {
    let click = [screen(Action::MouseDown {
        button: MouseButton::Left,
        at: Vector2I::new(1, 1),
    })];
    assert!(is_meaningful_action_group(&click));

    // A real interaction mixed with an explicit wait still qualifies as one
    // contiguous group; the wait is not split into an inferred gap.
    let mixed = [
        screen(Action::Wait(Duration::from_millis(500))),
        screen(Action::TypeText {
            text: "hi".to_string(),
        }),
    ];
    assert!(is_meaningful_action_group(&mixed));

    // A pointer-only batch qualifies (with empty labels) so its on-screen
    // effects are retained.
    let pointer_only = [screen(Action::MouseMove {
        to: Vector2I::new(2, 2),
    })];
    assert!(is_meaningful_action_group(&pointer_only));
}

#[test]
fn is_meaningful_action_group_false_for_wait_only_or_empty() {
    let zero_wait = [screen(Action::Wait(Duration::ZERO))];
    assert!(!is_meaningful_action_group(&zero_wait));

    let nonzero_wait = [screen(Action::Wait(Duration::from_millis(500)))];
    assert!(!is_meaningful_action_group(&nonzero_wait));

    assert!(!is_meaningful_action_group(&[]));
}

#[test]
fn empty_entries_produce_no_dialogue() {
    let ass = build_overlay_ass(&[], (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    assert!(ass.contains("[Events]"));
    assert!(!ass.contains("Dialogue:"));
}

#[test]
fn bottom_center_pill_style_and_dimensions() {
    let ass = build_overlay_ass(
        &[entry(1000, 2000, &["ctrl+a"])],
        (1920, 1080),
        SOURCE_TEN_SECS,
        FRAME_RATE_15,
    );
    assert!(ass.contains("PlayResX: 1920"));
    assert!(ass.contains("PlayResY: 1080"));
    assert!(ass.contains("Style: Pill,DejaVu Sans Mono,48"));
    // The single segment is [750, 3000] (output_start 0); the group displays
    // [1000, 3000] (lingering 1000 ms past finish) and remaps to [250, 2250] ms
    // on the output timeline.
    assert!(
        ass.contains("Dialogue: 0,0:00:00.25,0:00:02.25,Pill,,0,0,0,,{\\an2\\pos(960,990)}ctrl+a")
    );
}

#[test]
fn labels_in_a_group_share_timing_and_position() {
    let ass = build_overlay_ass(
        &[entry(1000, 2000, &["ctrl+a", "typing…", "Return"])],
        (1920, 1080),
        SOURCE_TEN_SECS,
        FRAME_RATE_15,
    );
    let dialogue_lines = ass
        .lines()
        .filter(|line| line.starts_with("Dialogue:"))
        .collect::<Vec<_>>();
    assert_eq!(dialogue_lines.len(), 3);
    assert!(
        dialogue_lines
            .iter()
            .all(|line| line.contains("0:00:00.25,0:00:02.25"))
    );
    assert!(dialogue_lines[0].contains("\\pos(715,990)}ctrl+a"));
    assert!(dialogue_lines[1].contains("\\pos(959,990)}typing…"));
    assert!(dialogue_lines[2].contains("\\pos(1204,990)}Return"));
}

#[test]
fn entries_are_ordered_by_timecode() {
    let entries = vec![
        entry(5000, 6000, &["typing…"]),
        entry(1000, 2000, &["ctrl+a"]),
    ];
    let ass = build_overlay_ass(&entries, (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    assert!(ass.find("ctrl+a").unwrap() < ass.find("typing…").unwrap());
}

#[test]
fn build_keep_segments_empty_when_no_entries() {
    assert!(build_keep_segments(&[], SOURCE_TEN_SECS, FRAME_RATE_15).is_empty());
}

#[test]
fn build_action_segments_uses_finish_offsets_and_drops_blocked_gaps() {
    // Two real action groups separated by a long blocked gap. The segment
    // builder must use each group's finish offset (not a fixed duration), apply
    // the asymmetric pre/post margins, leave the gap removed, and assign ordered
    // output starts.
    let entries = vec![entry(1000, 2000, &["a"]), entry(5000, 6000, &["b"])];
    let segments = build_keep_segments(&entries, SOURCE_TEN_SECS, FRAME_RATE_15);

    assert_eq!(
        segments,
        vec![
            // Group A: [1000, 2000] expanded by 250 ms before / 1000 ms after.
            seg(750, 3000, 0),
            // Group B: [5000, 6000] expanded; output starts after A's kept
            // duration (2250 ms), so the [3000, 4750] gap is removed.
            seg(4750, 7000, 2250),
        ]
    );
    // The blocked gap is absent from the output timeline: B's output start
    // equals A's kept duration, not A's source end.
    assert_eq!(
        segments[1].output_start,
        segments[0].source_end - segments[0].source_start
    );
}

#[test]
fn one_group_produces_one_segment() {
    let segments =
        build_keep_segments(&[entry(1000, 2000, &["a"])], SOURCE_TEN_SECS, FRAME_RATE_15);
    assert_eq!(segments, vec![seg(750, 3000, 0)]);
}

#[test]
fn start_at_zero_clamps_margin_to_source_start() {
    let segments = build_keep_segments(&[entry(0, 500, &["a"])], SOURCE_TEN_SECS, FRAME_RATE_15);
    assert_eq!(segments, vec![seg(0, 1500, 0)]);
}

#[test]
fn finish_after_source_end_clamps_to_source_duration() {
    let segments = build_keep_segments(
        &[entry(9500, 12000, &["a"])],
        SOURCE_TEN_SECS,
        FRAME_RATE_15,
    );
    assert_eq!(segments, vec![seg(9250, 10000, 0)]);
}

#[test]
fn out_of_order_groups_are_sorted_by_source_start() {
    let entries = vec![entry(5000, 6000, &["b"]), entry(1000, 2000, &["a"])];
    let segments = build_keep_segments(&entries, SOURCE_TEN_SECS, FRAME_RATE_15);
    assert_eq!(segments, vec![seg(750, 3000, 0), seg(4750, 7000, 2250)]);
}

#[test]
fn duplicate_starts_merge_into_one_segment() {
    let entries = vec![entry(1000, 2000, &["a"]), entry(1000, 1500, &["b"])];
    let segments = build_keep_segments(&entries, SOURCE_TEN_SECS, FRAME_RATE_15);
    assert_eq!(segments, vec![seg(750, 3000, 0)]);
}

#[test]
fn adjacent_margin_windows_merge() {
    // With a 250 ms pre-margin and 1000 ms post-margin the windows overlap
    // (A ends at 3000, B starts at 2750), so they merge into one contiguous
    // segment with no removed gap.
    let entries = vec![entry(1000, 2000, &["a"]), entry(3000, 4000, &["b"])];
    let segments = build_keep_segments(&entries, SOURCE_TEN_SECS, FRAME_RATE_15);
    assert_eq!(segments, vec![seg(750, 5000, 0)]);
}

#[test]
fn overlapping_margin_windows_merge() {
    let entries = vec![entry(1000, 2500, &["a"]), entry(2000, 3000, &["b"])];
    let segments = build_keep_segments(&entries, SOURCE_TEN_SECS, FRAME_RATE_15);
    assert_eq!(segments, vec![seg(750, 4000, 0)]);
}

#[test]
fn equal_frame_start_finish_enforces_one_frame_minimum() {
    // An instantaneous call (start == finish) still keeps a one-source-frame
    // window so its single frame is retained by the cut.
    let frame = Duration::from_secs_f64(1.0 / FRAME_RATE_15 as f64);
    let segments =
        build_keep_segments(&[entry(1000, 1000, &["a"])], SOURCE_TEN_SECS, FRAME_RATE_15);
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].source_start, Duration::from_millis(750));
    // source_end == action finish (start + one frame) + trailing post-margin.
    assert_eq!(
        segments[0].source_end,
        Duration::from_millis(1000) + frame + Duration::from_millis(1000)
    );
}

#[test]
fn entries_beyond_source_duration_produce_no_segment() {
    let segments = build_keep_segments(
        &[entry(11000, 12000, &["a"])],
        SOURCE_TEN_SECS,
        FRAME_RATE_15,
    );
    assert!(segments.is_empty());
}

#[test]
fn source_duration_shorter_than_margin_clamps_window() {
    let segments = build_keep_segments(
        &[entry(0, 100, &["a"])],
        Duration::from_millis(200),
        FRAME_RATE_15,
    );
    assert_eq!(segments, vec![seg(0, 200, 0)]);
}

#[test]
fn remap_source_interval_clamps_and_omits_across_removed_gaps() {
    // Same layout as the regression test: two segments with a removed gap.
    let segments = vec![seg(500, 2500, 0), seg(4500, 6500, 2000)];

    // A group before the gap keeps its source-relative timing.
    assert_eq!(
        remap_source_interval(
            Duration::from_millis(1000),
            Duration::from_millis(2000),
            &segments
        ),
        Some((Duration::from_millis(500), Duration::from_millis(1500)))
    );
    // A group after the gap shifts left by the removed gap duration (2000 ms).
    assert_eq!(
        remap_source_interval(
            Duration::from_millis(5000),
            Duration::from_millis(6000),
            &segments
        ),
        Some((Duration::from_millis(2500), Duration::from_millis(3500)))
    );
    // A group that starts in the gap and extends into the next segment is
    // clamped to the retained boundary (the next segment's start).
    assert_eq!(
        remap_source_interval(
            Duration::from_millis(3000),
            Duration::from_millis(5000),
            &segments
        ),
        Some((Duration::from_millis(2000), Duration::from_millis(2500)))
    );
    // A group wholly inside a removed gap is omitted.
    assert_eq!(
        remap_source_interval(
            Duration::from_millis(3000),
            Duration::from_millis(4000),
            &segments
        ),
        None
    );
}

#[test]
fn overlay_remaps_pill_timings_through_cut_segments() {
    // Two groups with a removed gap: the first pill keeps its time, the second
    // shifts left by the removed gap, and the ASS centisecond timecodes are
    // derived from the finish-offset-based remap.
    let entries = vec![entry(1000, 2000, &["a"]), entry(5000, 6000, &["b"])];
    let ass = build_overlay_ass(&entries, (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    // Single-char pills on 1280x720: pill width 61, left = (1280-61)/2 = 609,
    // x = 609 + 30 = 639, y = 720 - 90 = 630.
    assert!(
        ass.contains("Dialogue: 0,0:00:00.25,0:00:02.25,Pill,,0,0,0,,{\\an2\\pos(639,630)}a"),
        "{ass}"
    );
    assert!(
        ass.contains("Dialogue: 0,0:00:02.50,0:00:04.50,Pill,,0,0,0,,{\\an2\\pos(639,630)}b"),
        "{ass}"
    );
}

#[test]
fn instantaneous_action_pill_lingers_past_finish() {
    // An instantaneous action (finish == offset) must still show a readable
    // pill, not a single frame: the overlay lingers OVERLAY_LINGER (1000 ms)
    // past the action. Segment [750, 2000+frame]; display interval [1000, 2000]
    // remaps to [250, 1250] ms (~1000 ms visible).
    let ass = build_overlay_ass(
        &[entry(1000, 1000, &["Return"])],
        (1280, 720),
        SOURCE_TEN_SECS,
        FRAME_RATE_15,
    );
    let dialogue = ass
        .lines()
        .find(|line| line.starts_with("Dialogue:"))
        .expect("expected one pill dialogue");
    assert!(dialogue.contains("0:00:00.25,0:00:01.25"), "{ass}");
}
