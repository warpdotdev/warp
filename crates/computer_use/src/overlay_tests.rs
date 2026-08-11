use std::time::Duration;

use super::{
    ActionLogEntry, CLICK_RING_MAX_RADIUS, DRAG_ANCHOR_RADIUS, HELD_INDICATOR_RADIUS, KeepSegment,
    PointerEvent, PointerEventKind, ass_circle_path, build_keep_segments, build_overlay_ass,
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
        pointer_events: Vec::new(),
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
fn is_meaningful_action_group_keeps_nonzero_waits_but_not_no_ops() {
    let zero_wait = [screen(Action::Wait(Duration::ZERO))];
    assert!(!is_meaningful_action_group(&zero_wait));

    let nonzero_wait = [screen(Action::Wait(Duration::from_millis(500)))];
    assert!(is_meaningful_action_group(&nonzero_wait));

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
    // pill, not a single frame: the overlay lingers the 1000 ms post-action
    // margin past the action. Segment [750, 2000+frame]; display interval
    // [1000, 2000] remaps to [250, 1250] ms (~1000 ms visible).
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

// --- Pointer (click ripple / drag trail) rendering ---------------------------

fn down(offset_ms: u64, x: i32, y: i32) -> PointerEvent {
    PointerEvent {
        offset: Duration::from_millis(offset_ms),
        kind: PointerEventKind::Down,
        button: Some(MouseButton::Left),
        point: Vector2I::new(x, y),
    }
}

fn mv(offset_ms: u64, x: i32, y: i32) -> PointerEvent {
    PointerEvent {
        offset: Duration::from_millis(offset_ms),
        kind: PointerEventKind::Move,
        button: None,
        point: Vector2I::new(x, y),
    }
}

fn up(offset_ms: u64, x: i32, y: i32) -> PointerEvent {
    PointerEvent {
        offset: Duration::from_millis(offset_ms),
        kind: PointerEventKind::Up,
        button: Some(MouseButton::Left),
        point: Vector2I::new(x, y),
    }
}

fn scroll(offset_ms: u64, x: i32, y: i32) -> PointerEvent {
    PointerEvent {
        offset: Duration::from_millis(offset_ms),
        kind: PointerEventKind::Scroll,
        button: None,
        point: Vector2I::new(x, y),
    }
}

fn pointer_entry(
    start_ms: u64,
    finish_ms: u64,
    labels: &[&str],
    pointer_events: Vec<PointerEvent>,
) -> ActionLogEntry {
    ActionLogEntry {
        offset: Duration::from_millis(start_ms),
        finish_offset: Duration::from_millis(finish_ms),
        labels: labels.iter().map(ToString::to_string).collect(),
        pointer_events,
    }
}

// Gesture drawings (rings, trails, anchors, held dots) render on layer 1;
// the synthetic cursor glyph renders above them on layer 2.
fn cursor_dialogues(ass: &str) -> Vec<&str> {
    ass.lines()
        .filter(|line| line.starts_with("Dialogue: 1,") && line.contains(",Cursor,"))
        .collect()
}

fn cursor_glyph_dialogues(ass: &str) -> Vec<&str> {
    ass.lines()
        .filter(|line| line.starts_with("Dialogue: 2,") && line.contains(",Cursor,"))
        .collect()
}

fn pill_dialogues(ass: &str) -> Vec<&str> {
    ass.lines()
        .filter(|line| line.starts_with("Dialogue:") && line.contains(",Pill,"))
        .collect()
}

// Only the click ring uses a 4 px outline; the held/anchor/trail fills use
// `\bord0`, so `\bord4` uniquely identifies a ring dialogue.
fn ring_dialogues(ass: &str) -> Vec<&str> {
    cursor_dialogues(ass)
        .into_iter()
        .filter(|line| line.contains("\\bord4"))
        .collect()
}

#[test]
fn single_click_emits_one_expanding_ring() {
    let click = pointer_entry(
        1000,
        2000,
        &[],
        vec![down(1000, 100, 200), up(1000, 100, 200)],
    );
    let ass = build_overlay_ass(&[click], (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    let rings = ring_dialogues(&ass);
    assert_eq!(rings.len(), 1, "{ass}");
    let ring = rings[0];
    // Segment [750, 3000]; ring [1000, 1900] remaps to [250, 1150] ms.
    assert!(
        ring.contains("Dialogue: 1,0:00:00.25,0:00:01.15,Cursor,"),
        "{ass}"
    );
    assert!(ring.contains("\\an7\\pos(100,200)"), "{ass}");
    assert!(ring.contains("\\3c&H2850FF&"), "{ass}");
    assert!(ring.contains("\\fscx50\\fscy50"), "{ass}");
    assert!(
        ring.contains("\\t(0,900,\\fscx100\\fscy100\\3a&HFF&)"),
        "{ass}"
    );
    assert!(ring.contains("\\clip(0,0,1280,720)"), "{ass}");
    // A pointer-only group renders no pill.
    assert!(pill_dialogues(&ass).is_empty(), "{ass}");
}

#[test]
fn multi_click_emits_one_ring_per_completed_click() {
    for (clicks, expected) in [(2u64, 2usize), (3, 3)] {
        let mut events = Vec::new();
        for i in 0..clicks {
            let t = 1000 + i * 150;
            events.push(down(t, 10, 10));
            events.push(up(t, 10, 10));
        }
        let entry = pointer_entry(1000, 2000, &[], events);
        let ass = build_overlay_ass(&[entry], (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
        assert_eq!(ring_dialogues(&ass).len(), expected, "{ass}");
    }
}

#[test]
fn drag_emits_trail_anchor_held_and_no_ring() {
    let drag = pointer_entry(
        1000,
        2000,
        &[],
        vec![down(1000, 100, 100), mv(1200, 300, 400), up(1400, 300, 400)],
    );
    let ass = build_overlay_ass(&[drag], (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    let cursor = cursor_dialogues(&ass);
    // Exclusivity: a drag release emits no click ring.
    assert!(ring_dialogues(&ass).is_empty(), "{ass}");
    // Trail: filled polyline (fill alpha 73) with a 600 ms release fade. Segment
    // [750, 3000]; [1000, 2000] remaps to [250, 1250] ms, so the fade runs
    // [400, 1000] ms into the dialogue.
    let trail = cursor
        .iter()
        .find(|line| line.contains("\\1a&H73&"))
        .expect("trail dialogue");
    assert!(
        trail.contains("Dialogue: 1,0:00:00.25,0:00:01.25,Cursor,"),
        "{ass}"
    );
    assert!(trail.contains("\\an7\\pos(0,0)"), "{ass}");
    assert!(trail.contains("\\t(400,1000,\\1a&HFF&)"), "{ass}");
    // Anchor at the press point, alpha 87, centered on it via `\an7`.
    let anchor = cursor
        .iter()
        .find(|line| line.contains("\\1a&H87&"))
        .expect("anchor dialogue");
    assert!(anchor.contains("\\an7\\pos(100,100)"), "{ass}");
    // Held dot follows the same eased motion as the cursor glyph: a glide of
    // four 50 ms pieces into the move point ([1000, 1200] -> [250, 450]), then
    // a hold at the release point until the release at 650 ms.
    let held: Vec<_> = cursor
        .iter()
        .filter(|line| line.contains("\\1a&H4B&"))
        .collect();
    assert_eq!(held.len(), 5, "{ass}");
    assert!(
        held[0].contains("\\an7\\move(100,100,131,147,0,50)"),
        "{ass}"
    );
    assert!(
        held[3].contains("\\an7\\move(269,353,300,400,0,50)"),
        "{ass}"
    );
    assert!(
        held[4].contains("Dialogue: 1,0:00:00.45,0:00:00.65,Cursor,"),
        "{ass}"
    );
    assert!(held[4].contains("\\an7\\pos(300,400)"), "{ass}");
}

#[test]
fn multi_segment_drag_trail_has_a_quad_per_nonzero_segment() {
    let drag = pointer_entry(
        1000,
        2000,
        &[],
        vec![
            down(1000, 0, 0),
            mv(1100, 100, 0),
            mv(1200, 100, 100),
            up(1300, 100, 100),
        ],
    );
    let ass = build_overlay_ass(&[drag], (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    let trail = cursor_dialogues(&ass)
        .into_iter()
        .find(|line| line.contains("\\1a&H73&"))
        .expect("trail dialogue");
    // Points (0,0)->(100,0)->(100,100)->(100,100): the final zero-length segment
    // is dropped, leaving two quads (each a `m ... l ... l ... l ...` subpath).
    assert_eq!(trail.matches("m ").count(), 2, "{trail}");
}

#[test]
fn press_held_at_end_renders_held_indicator_without_ring() {
    let held_press = pointer_entry(1000, 2000, &[], vec![down(1000, 50, 50)]);
    let ass = build_overlay_ass(&[held_press], (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    assert!(ring_dialogues(&ass).is_empty(), "{ass}");
    // A lone held press has a single stationary waypoint: the dot rests at
    // the press point (no `\move` schedule).
    let held = cursor_dialogues(&ass)
        .into_iter()
        .find(|line| line.contains("\\1a&H4B&"))
        .expect("held dialogue");
    assert!(held.contains("\\an7\\pos(50,50)"), "{ass}");
}

#[test]
fn mixed_group_renders_pill_and_pointer_without_leaking_text() {
    let mixed = pointer_entry(
        1000,
        2000,
        &["typing\u{2026}"],
        vec![down(1000, 50, 60), up(1000, 50, 60)],
    );
    let ass = build_overlay_ass(&[mixed], (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    assert!(
        pill_dialogues(&ass)
            .iter()
            .any(|line| line.contains("typing\u{2026}")),
        "{ass}"
    );
    assert_eq!(ring_dialogues(&ass).len(), 1, "{ass}");
    // No typed text leaks into any pointer (Cursor) dialogue.
    for line in cursor_dialogues(&ass) {
        assert!(!line.contains("typing"), "{line}");
    }
}

#[test]
fn wait_only_group_renders_no_dialogue() {
    let wait_only = pointer_entry(1000, 2000, &[], vec![]);
    let ass = build_overlay_ass(&[wait_only], (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    assert!(
        !ass.lines().any(|line| line.starts_with("Dialogue:")),
        "{ass}"
    );
}

#[test]
fn out_of_bounds_point_is_clamped_into_frame() {
    let click = pointer_entry(
        1000,
        2000,
        &[],
        vec![down(1000, 5000, -20), up(1000, 5000, -20)],
    );
    let ass = build_overlay_ass(&[click], (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    let ring = ring_dialogues(&ass);
    assert_eq!(ring.len(), 1, "{ass}");
    // x clamps to width-1 (1279), y clamps to 0; the ring is centered there.
    assert!(ring[0].contains("\\an7\\pos(1279,0)"), "{ass}");
}

#[test]
fn click_animation_fits_within_retained_post_action_margin() {
    // A click at the group's finish: the 900 ms ring fits entirely within the
    // 1000 ms post-action margin the cut already retains, so it is not clipped
    // and no build_keep_segments change is needed. Pointer events must not alter
    // the retained segments.
    let with = pointer_entry(
        1000,
        2000,
        &[],
        vec![down(2000, 500, 500), up(2000, 500, 500)],
    );
    let without = entry(1000, 2000, &[]);
    assert_eq!(
        build_keep_segments(std::slice::from_ref(&with), SOURCE_TEN_SECS, FRAME_RATE_15),
        build_keep_segments(
            std::slice::from_ref(&without),
            SOURCE_TEN_SECS,
            FRAME_RATE_15
        ),
        "pointer events must not change the retained segments",
    );
    let ass = build_overlay_ass(&[with], (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    let rings = ring_dialogues(&ass);
    assert_eq!(rings.len(), 1, "{ass}");
    // [2000, 2900] remaps through [750, 3000] to [1250, 2150] = a full 900 ms.
    assert!(
        rings[0].contains("Dialogue: 1,0:00:01.25,0:00:02.15,Cursor,"),
        "{ass}"
    );
    assert!(rings[0].contains("\\t(0,900,"), "{ass}");
}

// --- Centering (QUALITY-1169 follow-up) --------------------------------------

#[test]
fn circle_path_is_origin_centered() {
    // `ass_circle_path(r)` draws a circle centered at the drawing origin (0, 0):
    // its coordinate extrema are ±r on both axes, so the drawing origin is the
    // circle's geometric center. This is what makes `\an7` center the circle on
    // `\pos` (the origin maps to `\pos`).
    for radius in [
        CLICK_RING_MAX_RADIUS,
        DRAG_ANCHOR_RADIUS,
        HELD_INDICATOR_RADIUS,
    ] {
        let path = ass_circle_path(radius);
        let r = radius.round() as i32;
        assert!(path.contains(&format!("m {r} 0")), "{path}");
        assert!(path.contains(&format!("-{r} 0")), "{path}");
        assert!(path.contains(&format!("0 -{r}")), "{path}");
        assert!(path.contains(&format!("0 {r}")), "{path}");
    }
}

#[test]
fn click_ring_and_drag_circles_center_via_an7() {
    // A click and a drag in one recording exercise every circle dialogue: the
    // click ring, the drag anchor, and the held indicator. Centering is proven
    // here at the ASS-string level — each circle dialogue carries `\an7`, and
    // `circle_path_is_origin_centered` proves the path is centered on the
    // drawing origin, so `\pos` lands the circle's center on the cursor. The
    // libass pixel-level confirmation is the synthetic-frame proof in the PR
    // body, not a model assertion here.
    let entries = vec![
        pointer_entry(
            1000,
            2000,
            &[],
            vec![down(1000, 100, 200), up(1000, 100, 200)],
        ),
        pointer_entry(
            3000,
            4000,
            &[],
            vec![down(3000, 50, 60), mv(3200, 70, 80), up(3400, 70, 80)],
        ),
    ];
    let ass = build_overlay_ass(&entries, (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    let cursor = cursor_dialogues(&ass);

    // The trail polyline keeps its absolute-coordinate `\an7\pos(0,0)`.
    let trail = cursor
        .iter()
        .find(|line| line.contains("\\1a&H73&"))
        .expect("trail dialogue");
    assert!(trail.contains("\\an7\\pos(0,0)"), "{trail}");

    // The click ring centers on (100, 200) via `\an7`.
    let rings = ring_dialogues(&ass);
    let ring = rings
        .iter()
        .find(|line| line.contains("\\pos(100,200)"))
        .expect("ring dialogue");
    assert!(ring.contains("\\an7\\pos(100,200)"), "{ring}");

    // The drag anchor and held indicator center on their points via `\an7`.
    let anchor = cursor
        .iter()
        .find(|line| line.contains("\\1a&H87&"))
        .expect("anchor dialogue");
    assert!(anchor.contains("\\an7\\pos(50,60)"), "{anchor}");
    let held: Vec<_> = cursor
        .iter()
        .filter(|line| line.contains("\\1a&H4B&"))
        .collect();
    assert!(
        held.iter().any(|line| line.contains("\\an7\\move(50,60,")),
        "{ass}"
    );
    assert!(
        held.iter().any(|line| line.contains("\\an7\\pos(70,80)")),
        "{ass}"
    );
    // Clipping is retained on every pointer dialogue.
    for line in &cursor {
        assert!(line.contains("\\clip(0,0,1280,720)"), "{line}");
    }
}

// --- Split-call drag reconstruction (QUALITY-1169 follow-up) -----------------

#[test]
fn split_call_drag_renders_one_trail_like_a_canonical_drag() {
    // The same Down -> Move -> Up sequence, but split across three
    // `ActionLogEntry` boundaries (three separate `UseComputer` calls) instead
    // of one. The release in the third call reuses the last resolved point from
    // the recording-scoped pointer session, and the flattened recording-level
    // classifier stitches the three entries into a single drag.
    let split = vec![
        pointer_entry(1000, 1100, &[], vec![down(1000, 100, 100)]),
        pointer_entry(1200, 1300, &[], vec![mv(1200, 300, 400)]),
        pointer_entry(1400, 1500, &[], vec![up(1400, 300, 400)]),
    ];
    // The canonical same-call drag carries the identical events in one entry.
    let canonical = vec![pointer_entry(
        1000,
        2000,
        &[],
        vec![down(1000, 100, 100), mv(1200, 300, 400), up(1400, 300, 400)],
    )];
    let split_ass = build_overlay_ass(&split, (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    let canonical_ass = build_overlay_ass(&canonical, (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    // A drag never emits a click ring, in either form.
    assert!(ring_dialogues(&split_ass).is_empty(), "{split_ass}");
    assert!(ring_dialogues(&canonical_ass).is_empty(), "{canonical_ass}");
    // The split and canonical forms produce identical pointer dialogues: one
    // trail, one anchor, one held indicator with a release fade — the same single
    // gesture, timing, and path.
    assert_eq!(
        cursor_dialogues(&split_ass),
        cursor_dialogues(&canonical_ass),
        "split:\n{split_ass}\ncanonical:\n{canonical_ass}"
    );
    // Sanity: one trail, one anchor, and the held dot's eased motion (four
    // glide pieces plus the hold at the release point).
    let cursor = cursor_dialogues(&split_ass);
    assert_eq!(cursor.len(), 7, "{split_ass}");
    assert!(
        cursor.iter().any(|l| l.contains("\\1a&H73&")),
        "{split_ass}"
    );
    assert!(
        cursor.iter().any(|l| l.contains("\\1a&H87&")),
        "{split_ass}"
    );
    assert!(
        cursor.iter().any(|l| l.contains("\\1a&H4B&")),
        "{split_ass}"
    );
}

#[test]
fn split_call_drag_with_moves_across_two_entries_renders_one_trail() {
    // Down in call A, Moves in calls B and C, Up in call D — the full
    // four-call split described in the spec. The trail path contains every
    // non-zero move segment and the release fades.
    let split = vec![
        pointer_entry(1000, 1100, &[], vec![down(1000, 0, 0)]),
        pointer_entry(1200, 1300, &[], vec![mv(1200, 100, 0)]),
        pointer_entry(1400, 1500, &[], vec![mv(1400, 100, 100)]),
        pointer_entry(1600, 1700, &[], vec![up(1600, 100, 100)]),
    ];
    let ass = build_overlay_ass(&split, (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    assert!(ring_dialogues(&ass).is_empty(), "{ass}");
    let trail = cursor_dialogues(&ass)
        .into_iter()
        .find(|line| line.contains("\\1a&H73&"))
        .expect("trail dialogue");
    // Two non-zero segments: (0,0)->(100,0) and (100,0)->(100,100); the final
    // move-to-release segment is zero-length and dropped, leaving two quads.
    assert_eq!(trail.matches("m ").count(), 2, "{trail}");
    // The release fade is present (a drag with a release).
    assert!(trail.contains("\\t("), "release fade missing: {trail}");
    assert!(trail.contains("\\1a&HFF&"), "release fade missing: {trail}");
}

// --- Button identity and boundary cases (QUALITY-1169 follow-up) -------------

fn down_with(offset_ms: u64, x: i32, y: i32, button: MouseButton) -> PointerEvent {
    PointerEvent {
        offset: Duration::from_millis(offset_ms),
        kind: PointerEventKind::Down,
        button: Some(button),
        point: Vector2I::new(x, y),
    }
}

fn up_with(offset_ms: u64, x: i32, y: i32, button: MouseButton) -> PointerEvent {
    PointerEvent {
        offset: Duration::from_millis(offset_ms),
        kind: PointerEventKind::Up,
        button: Some(button),
        point: Vector2I::new(x, y),
    }
}

#[test]
fn right_and_middle_clicks_render_rings() {
    // A right press + release and a middle press + release are each their own
    // click ring; button identity does not merge them into a drag.
    let right = pointer_entry(
        1000,
        2000,
        &[],
        vec![
            down_with(1000, 10, 10, MouseButton::Right),
            up_with(1000, 10, 10, MouseButton::Right),
        ],
    );
    let middle = pointer_entry(
        2100,
        2200,
        &[],
        vec![
            down_with(2100, 20, 20, MouseButton::Middle),
            up_with(2100, 20, 20, MouseButton::Middle),
        ],
    );
    let ass = build_overlay_ass(
        &[right, middle],
        (1280, 720),
        SOURCE_TEN_SECS,
        FRAME_RATE_15,
    );
    assert_eq!(ring_dialogues(&ass).len(), 2, "{ass}");
}

#[test]
fn unmatched_release_and_stray_move_render_nothing() {
    // A release whose button was never pressed, and a move with no owning press,
    // carry no drawable gesture.
    let stray_release = pointer_entry(
        1000,
        1100,
        &[],
        vec![up_with(1000, 30, 30, MouseButton::Left)],
    );
    let ass = build_overlay_ass(
        &[stray_release],
        (1280, 720),
        SOURCE_TEN_SECS,
        FRAME_RATE_15,
    );
    assert!(cursor_dialogues(&ass).is_empty(), "{ass}");

    let stray_move = pointer_entry(2000, 2100, &[], vec![mv(2000, 40, 50)]);
    let ass = build_overlay_ass(&[stray_move], (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    assert!(cursor_dialogues(&ass).is_empty(), "{ass}");
}

#[test]
fn unmatched_release_for_a_different_button_does_not_close_a_drag() {
    // A Left press + move is a drag in progress; a Right release (a different
    // button) does not close it. A following Left release closes it, so one drag
    // (no ring) renders.
    let events = vec![
        down_with(1000, 100, 100, MouseButton::Left),
        mv(1200, 200, 200),
        up_with(1300, 200, 200, MouseButton::Right),
        up_with(1400, 200, 200, MouseButton::Left),
    ];
    let entry = pointer_entry(1000, 1500, &[], events);
    let ass = build_overlay_ass(&[entry], (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    assert!(ring_dialogues(&ass).is_empty(), "{ass}");
    let cursor = cursor_dialogues(&ass);
    // One drag: a single trail and anchor; the held dot renders as its eased
    // motion dialogues (four glide pieces plus the hold at the release point).
    assert_eq!(
        cursor.iter().filter(|l| l.contains("\\1a&H73&")).count(),
        1,
        "{ass}"
    );
    assert_eq!(
        cursor.iter().filter(|l| l.contains("\\1a&H87&")).count(),
        1,
        "{ass}"
    );
    assert_eq!(
        cursor.iter().filter(|l| l.contains("\\1a&H4B&")).count(),
        5,
        "{ass}"
    );
    // The held dot glides to and then rests at the Left release point.
    assert!(
        cursor
            .iter()
            .any(|l| l.contains("\\1a&H4B&") && l.contains("\\an7\\pos(200,200)")),
        "{ass}"
    );
}

// --- Synthetic cursor glyph ---------------------------------------------------

#[test]
fn cursor_glyph_holds_at_click_point_through_retained_footage() {
    let click = pointer_entry(
        1000,
        2000,
        &[],
        vec![down(1000, 100, 200), up(1000, 100, 200)],
    );
    let ass = build_overlay_ass(&[click], (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    let glyphs = cursor_glyph_dialogues(&ass);
    // The zero-length down->up hold renders nothing; the release holds from
    // 1000 ms through the end of the retained segment [750, 3000] -> [250, 2250].
    assert_eq!(glyphs.len(), 1, "{ass}");
    assert!(
        glyphs[0].contains("Dialogue: 2,0:00:00.25,0:00:02.25,Cursor,"),
        "{ass}"
    );
    // The arrow's tip (the drawing origin) lands on the pointer position.
    assert!(glyphs[0].contains("\\an7\\pos(100,200)"), "{ass}");
    // The first cursor dialogue fades in instead of popping into existence.
    assert!(glyphs[0].contains("\\fad(150,0)"), "{ass}");
    // White fill, black outline, frame clip: legible on any background and
    // never addressing outside the frame.
    assert!(glyphs[0].contains("\\1c&HFFFFFF&"), "{ass}");
    assert!(glyphs[0].contains("\\3c&H000000&"), "{ass}");
    assert!(glyphs[0].contains("\\clip(0,0,1280,720)"), "{ass}");
    // The glyph path starts at the origin (tip hotspot).
    assert!(glyphs[0].contains("}m 0 0 l "), "{ass}");
}

#[test]
fn cursor_glyph_glides_eased_into_each_drag_point() {
    let drag = pointer_entry(
        1000,
        2000,
        &[],
        vec![down(1000, 100, 100), mv(1200, 300, 400), up(1400, 300, 400)],
    );
    let ass = build_overlay_ass(&[drag], (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    let glyphs = cursor_glyph_dialogues(&ass);
    // Segment [750, 3000]: the press-to-move gap ([250, 450] on the output
    // timeline) is shorter than the max glide, so the whole gap is one eased
    // glide of four 50 ms `\move` pieces arriving exactly at the move event;
    // the move-to-release gap is stationary ([450, 650]); the release holds
    // through the end of the retained footage ([650, 2250]).
    assert_eq!(glyphs.len(), 6, "{ass}");
    assert!(
        glyphs[0].contains("2,0:00:00.25,0:00:00.30,Cursor,"),
        "{ass}"
    );
    assert!(glyphs[0].contains("\\move(100,100,131,147,0,50)"), "{ass}");
    // The fade-in clamps to the first (50 ms) piece.
    assert!(glyphs[0].contains("\\fad(50,0)"), "{ass}");
    // Ease-in-out: the middle pieces cover more distance than the ends.
    assert!(glyphs[1].contains("\\move(131,147,200,250,0,50)"), "{ass}");
    assert!(glyphs[2].contains("\\move(200,250,269,353,0,50)"), "{ass}");
    assert!(
        glyphs[3].contains("2,0:00:00.40,0:00:00.45,Cursor,"),
        "{ass}"
    );
    assert!(glyphs[3].contains("\\move(269,353,300,400,0,50)"), "{ass}");
    assert!(
        glyphs[4].contains("2,0:00:00.45,0:00:00.65,Cursor,"),
        "{ass}"
    );
    assert!(glyphs[4].contains("\\pos(300,400)"), "{ass}");
    assert!(
        glyphs[5].contains("2,0:00:00.65,0:00:02.25,Cursor,"),
        "{ass}"
    );
    assert!(glyphs[5].contains("\\pos(300,400)"), "{ass}");
}

#[test]
fn cursor_glide_bridges_a_removed_gap_smoothly() {
    // Click in group A, then a later click in group B with a removed gap
    // between them. The cursor holds at A's point, then glides into B's click
    // over the max glide window on the *output* timeline — the glide starts in
    // A's retained tail and finishes in B's lead-in, bridging the cut seam
    // (output 2250 ms) smoothly instead of being swallowed by the removed gap.
    let a = pointer_entry(1000, 2000, &[], vec![down(1000, 10, 10), up(1000, 10, 10)]);
    let b = pointer_entry(5000, 6000, &[], vec![down(5000, 20, 20), up(5000, 20, 20)]);
    let ass = build_overlay_ass(&[a, b], (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    let glyphs = cursor_glyph_dialogues(&ass);
    // Segments [750,3000]->0 and [4750,7000]->2250; B's click lands at output
    // 2500 ms. Hold [250, 2000], ten 50 ms glide pieces [2000, 2500], and the
    // final hold [2500, 4500].
    assert_eq!(glyphs.len(), 12, "{ass}");
    assert!(
        glyphs[0].contains("2,0:00:00.25,0:00:02.00,Cursor,"),
        "{ass}"
    );
    assert!(glyphs[0].contains("\\pos(10,10)"), "{ass}");
    // The glide is continuous across the seam: the piece beginning at the
    // seam picks up exactly where the previous piece left off.
    assert!(
        glyphs[6].contains("2,0:00:02.25,0:00:02.30,Cursor,"),
        "{ass}"
    );
    assert!(glyphs[6].contains("\\move(15,15,16,16,0,50)"), "{ass}");
    assert!(
        glyphs[11].contains("2,0:00:02.50,0:00:04.50,Cursor,"),
        "{ass}"
    );
    assert!(glyphs[11].contains("\\pos(20,20)"), "{ass}");
}

#[test]
fn cursor_glide_is_clamped_by_the_previous_event() {
    // Two clicks 300 ms apart — closer than the 500 ms max glide. The glide
    // starts at the previous event (no intermediate hold) and spans the whole
    // gap, still arriving exactly at the second click.
    let entry = pointer_entry(
        1000,
        2000,
        &[],
        vec![
            down(1000, 10, 10),
            up(1000, 10, 10),
            down(1300, 200, 200),
            up(1300, 200, 200),
        ],
    );
    let ass = build_overlay_ass(&[entry], (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    let glyphs = cursor_glyph_dialogues(&ass);
    // Glide [250, 550] in six 50 ms pieces, then the final hold [550, 2250].
    assert_eq!(glyphs.len(), 7, "{ass}");
    assert!(
        glyphs[0].contains("2,0:00:00.25,0:00:00.30,Cursor,"),
        "{ass}"
    );
    assert!(glyphs[0].contains("\\move(10,10,"), "{ass}");
    assert!(glyphs[5].contains(",0:00:00.55,Cursor,"), "{ass}");
    assert!(glyphs[5].contains(",200,200,0,50)"), "{ass}");
    assert!(
        glyphs[6].contains("2,0:00:00.55,0:00:02.25,Cursor,"),
        "{ass}"
    );
    assert!(glyphs[6].contains("\\pos(200,200)"), "{ass}");
}

fn move_tags(lines: &[&str]) -> Vec<String> {
    lines
        .iter()
        .filter_map(|line| {
            let start = line.find("\\move(")?;
            let end = line[start..].find(')')? + start;
            Some(line[start..=end].to_string())
        })
        .collect()
}

#[test]
fn held_dot_and_cursor_glide_along_the_same_path() {
    // During a drag the held dot and the cursor glyph must stay glued
    // together: they emit identical `\move` schedules (same coordinates and
    // timing), differing only in styling and layer — including through a
    // multi-waypoint path.
    let drag = pointer_entry(
        1000,
        2000,
        &[],
        vec![
            down(1000, 100, 100),
            mv(1200, 300, 400),
            mv(1400, 500, 200),
            up(1600, 500, 200),
        ],
    );
    let ass = build_overlay_ass(&[drag], (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    let dot_lines: Vec<&str> = cursor_dialogues(&ass)
        .into_iter()
        .filter(|line| line.contains("\\1a&H4B&"))
        .collect();
    let dot_moves = move_tags(&dot_lines);
    let glyph_moves = move_tags(&cursor_glyph_dialogues(&ass));
    assert!(!dot_moves.is_empty(), "{ass}");
    assert_eq!(dot_moves, glyph_moves, "{ass}");
}

#[test]
fn drag_cursor_moves_continuously_through_waypoints() {
    // A multi-waypoint drag must not pause at sampled waypoints: the first
    // moving leg eases in, intermediate legs are linear (one exact `\move`
    // filling the whole gap), the last leg eases out, and the only stationary
    // dialogues are the real dwell at the end point before the release and
    // the post-release hold.
    let drag = pointer_entry(
        1000,
        2200,
        &[],
        vec![
            down(1000, 0, 0),
            mv(1300, 90, 0),
            mv(1600, 180, 0),
            mv(1900, 270, 0),
            up(2100, 270, 0),
        ],
    );
    let ass = build_overlay_ass(&[drag], (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    let glyphs = cursor_glyph_dialogues(&ass);
    // Segment [750, 3200] -> output = source - 750. Legs [250,550] (ease-in,
    // six pieces), [550,850] (linear, one piece), [850,1150] (ease-out, six
    // pieces), dwell [1150,1350], final hold [1350,2450].
    assert_eq!(glyphs.len(), 15, "{ass}");
    // Ease-in: the first piece covers little distance (accelerating from rest).
    assert!(
        glyphs[0].contains("2,0:00:00.25,0:00:00.30,Cursor,"),
        "{ass}"
    );
    assert!(glyphs[0].contains("\\move(0,0,3,0,0,50)"), "{ass}");
    // The intermediate leg is one linear `\move` spanning its whole gap.
    assert!(
        glyphs[6].contains("2,0:00:00.55,0:00:00.85,Cursor,"),
        "{ass}"
    );
    assert!(glyphs[6].contains("\\move(90,0,180,0,0,300)"), "{ass}");
    // Ease-out: the last leg starts fast and settles into the final point.
    assert!(glyphs[7].contains("\\move(180,0,208,0,0,50)"), "{ass}");
    assert!(glyphs[12].contains(",0:00:01.15,Cursor,"), "{ass}");
    assert!(glyphs[12].contains(",270,0,0,50)"), "{ass}");
    // No stationary cursor anywhere along the path: the only `\pos` holds are
    // the pre-release dwell and the post-release hold.
    let holds: Vec<_> = glyphs
        .iter()
        .filter(|line| line.contains("\\pos("))
        .collect();
    assert_eq!(holds.len(), 2, "{ass}");
    assert!(
        holds[0].contains("2,0:00:01.15,0:00:01.35,Cursor,"),
        "{ass}"
    );
    assert!(holds[0].contains("\\pos(270,0)"), "{ass}");
    assert!(
        holds[1].contains("2,0:00:01.35,0:00:02.45,Cursor,"),
        "{ass}"
    );
}

#[test]
fn split_call_drag_glides_across_removed_gaps_without_stopping() {
    // A drag split across three calls with multi-second thinking gaps: the
    // cut compresses the gaps, and the drag leg fills the whole compressed
    // gap — the cursor starts moving at the press and never holds mid-path,
    // even across the removed-gap seams. The held dot follows identically.
    let split = vec![
        pointer_entry(1000, 1100, &[], vec![down(1000, 100, 100)]),
        pointer_entry(5000, 5100, &[], vec![mv(5000, 300, 400)]),
        pointer_entry(9000, 9100, &[], vec![up(9000, 300, 400)]),
    ];
    let ass = build_overlay_ass(&split, (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    let glyphs = cursor_glyph_dialogues(&ass);
    // Segments [750,2100]->0, [4750,6100]->1350, [8750,10000]->2700. The leg
    // spans [250, 1600] (sixteen pieces), dwell [1600, 2950], final hold
    // [2950, 3950].
    assert_eq!(glyphs.len(), 18, "{ass}");
    // Motion starts at the press itself — no hold at the press point.
    assert!(glyphs[0].contains("2,0:00:00.25,"), "{ass}");
    assert!(glyphs[0].contains("\\move(100,100,"), "{ass}");
    // The lone moving leg is eased in-out: its midpoint passes (200, 250).
    assert!(ass.contains(",200,250,0,"), "{ass}");
    // The first stationary dialogue is the dwell at the move point.
    let first_hold = glyphs
        .iter()
        .find(|line| line.contains("\\pos("))
        .expect("dwell dialogue");
    assert!(
        first_hold.contains("2,0:00:01.60,0:00:02.95,Cursor,"),
        "{ass}"
    );
    assert!(first_hold.contains("\\pos(300,400)"), "{ass}");
    // The held dot's motion schedule matches the cursor's exactly.
    let dot_lines: Vec<&str> = cursor_dialogues(&ass)
        .into_iter()
        .filter(|line| line.contains("\\1a&H4B&"))
        .collect();
    assert_eq!(move_tags(&dot_lines), move_tags(&glyphs), "{ass}");
}

#[test]
fn scroll_only_group_renders_cursor_but_no_gesture() {
    let entry = pointer_entry(1000, 2000, &[], vec![scroll(1000, 50, 60)]);
    let ass = build_overlay_ass(&[entry], (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    // A scroll sample carries no click/drag gesture...
    assert!(cursor_dialogues(&ass).is_empty(), "{ass}");
    // ...but the cursor glyph tracks the wheel position.
    let glyphs = cursor_glyph_dialogues(&ass);
    assert_eq!(glyphs.len(), 1, "{ass}");
    assert!(glyphs[0].contains("\\pos(50,60)"), "{ass}");
}

#[test]
fn scroll_sample_between_press_and_release_keeps_the_click() {
    // A scroll between a press and its matching release is position-only: the
    // gesture stays a click (one ring, no trail), while the cursor glyph
    // tracks the wheel position.
    let entry = pointer_entry(
        1000,
        2000,
        &[],
        vec![
            down(1000, 100, 100),
            scroll(1200, 300, 300),
            up(1400, 300, 300),
        ],
    );
    let ass = build_overlay_ass(&[entry], (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    assert_eq!(ring_dialogues(&ass).len(), 1, "{ass}");
    assert!(
        !cursor_dialogues(&ass)
            .iter()
            .any(|line| line.contains("\\1a&H73&")),
        "a scroll sample must not create a drag trail: {ass}"
    );
    // The glyph glides into the wheel position (four pieces over [250, 450])
    // and rests there through the release and the retained tail.
    let glyphs = cursor_glyph_dialogues(&ass);
    assert_eq!(glyphs.len(), 6, "{ass}");
    assert!(glyphs[3].contains("\\move(269,269,300,300,0,50)"), "{ass}");
    assert!(glyphs[4].contains("\\pos(300,300)"), "{ass}");
}

#[test]
fn keyboard_only_entry_renders_no_cursor_glyph() {
    // No pointer events -> no cursor: its position is unknown.
    let ass = build_overlay_ass(
        &[entry(1000, 2000, &["Return"])],
        (1280, 720),
        SOURCE_TEN_SECS,
        FRAME_RATE_15,
    );
    assert!(cursor_glyph_dialogues(&ass).is_empty(), "{ass}");
}

#[test]
fn cursor_glyph_point_is_clamped_into_frame() {
    let entry = pointer_entry(1000, 2000, &[], vec![scroll(1000, 5000, -20)]);
    let ass = build_overlay_ass(&[entry], (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    let glyphs = cursor_glyph_dialogues(&ass);
    assert_eq!(glyphs.len(), 1, "{ass}");
    assert!(glyphs[0].contains("\\an7\\pos(1279,0)"), "{ass}");
}

#[test]
fn second_press_while_held_closes_prior_gesture_deterministically() {
    // A second Down with no intervening release closes the first press as a
    // held drag (no release); classification restarts at the new press, which
    // then completes as a drag with a release.
    let events = vec![
        down(1000, 100, 100),
        mv(1200, 200, 200),
        down(1300, 300, 300),
        mv(1400, 400, 400),
        up(1500, 400, 400),
    ];
    let entry = pointer_entry(1000, 1600, &[], events);
    let ass = build_overlay_ass(&[entry], (1280, 720), SOURCE_TEN_SECS, FRAME_RATE_15);
    // Two drags: the first (held, no release) and the second (with release),
    // each emitting its own anchor. No click ring.
    assert_eq!(
        cursor_dialogues(&ass)
            .iter()
            .filter(|l| l.contains("\\1a&H87&"))
            .count(),
        2,
        "{ass}"
    );
    assert!(ring_dialogues(&ass).is_empty(), "{ass}");
}
