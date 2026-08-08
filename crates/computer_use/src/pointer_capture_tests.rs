//! Tests for the shared pointer-event capture helpers. Both actors route their
//! pointer events through these, so the capture-space resolution, session
//! continuity, and mismatched-surface behavior are covered once here.

use std::sync::{Arc, Mutex};

use instant::Instant;
use pathfinder_geometry::vector::Vector2I;

use super::{record_positioned_event, record_up, resolve_capture_point};
use crate::overlay::{PointerEvent, PointerEventKind};
use crate::{MouseButton, PointerSession, PointerSink, Target};

const RECORDED_WINDOW: Target = Target::Window {
    window_id: 7,
    pid: 100,
};
const OTHER_WINDOW: Target = Target::Window {
    window_id: 9,
    pid: 200,
};

fn sink(recording_target: Target) -> PointerSink {
    PointerSink {
        started_at: Instant::now(),
        recording_target,
        events: Arc::new(Mutex::new(Vec::new())),
        session: PointerSession::new(),
    }
}

fn drain(sink: &PointerSink) -> Vec<PointerEvent> {
    sink.events.lock().expect("pointer events poisoned").clone()
}

#[test]
fn screen_recording_keeps_global_pixels_for_every_surface() {
    let local = Vector2I::new(10, 20);
    let global = Vector2I::new(310, 420);

    for action_target in [Target::Screen, RECORDED_WINDOW, OTHER_WINDOW] {
        assert_eq!(
            resolve_capture_point(Target::Screen, action_target, local, global),
            Some(global)
        );
    }
}

#[test]
fn window_recording_keeps_window_local_pixels_and_omits_other_surfaces() {
    let local = Vector2I::new(10, 20);
    let global = Vector2I::new(310, 420);

    assert_eq!(
        resolve_capture_point(RECORDED_WINDOW, RECORDED_WINDOW, local, global),
        Some(local)
    );
    assert_eq!(
        resolve_capture_point(RECORDED_WINDOW, OTHER_WINDOW, local, global),
        None
    );
    assert_eq!(
        resolve_capture_point(RECORDED_WINDOW, Target::Screen, local, global),
        None
    );
}

#[test]
fn records_a_press_move_release_gesture_in_order() {
    let sink = sink(Target::Screen);
    let press = Vector2I::new(100, 100);
    let moved = Vector2I::new(180, 140);

    record_positioned_event(
        Some(&sink),
        PointerEventKind::Down,
        Some(MouseButton::Left),
        Target::Screen,
        press,
        press,
    );
    record_positioned_event(
        Some(&sink),
        PointerEventKind::Move,
        None,
        Target::Screen,
        moved,
        moved,
    );
    record_up(Some(&sink), MouseButton::Left);

    let events = drain(&sink);
    let shape: Vec<_> = events
        .iter()
        .map(|event| (event.kind, event.button, event.point))
        .collect();
    assert_eq!(
        shape,
        vec![
            (PointerEventKind::Down, Some(MouseButton::Left), press),
            (PointerEventKind::Move, None, moved),
            // The release carries no coordinate; the session supplies the last one.
            (PointerEventKind::Up, Some(MouseButton::Left), moved),
        ]
    );
    assert!(
        events
            .windows(2)
            .all(|pair| pair[0].offset <= pair[1].offset),
        "offsets should be non-decreasing, got {events:?}"
    );
}

#[test]
fn a_release_split_into_a_later_call_reuses_the_session_point() {
    let session = PointerSession::new();
    let press_call = PointerSink {
        started_at: Instant::now(),
        recording_target: Target::Screen,
        events: Arc::new(Mutex::new(Vec::new())),
        session: session.clone(),
    };
    let release_call = PointerSink {
        started_at: press_call.started_at,
        recording_target: Target::Screen,
        events: Arc::new(Mutex::new(Vec::new())),
        session,
    };
    let press = Vector2I::new(64, 32);

    record_positioned_event(
        Some(&press_call),
        PointerEventKind::Down,
        Some(MouseButton::Left),
        Target::Screen,
        press,
        press,
    );
    record_up(Some(&release_call), MouseButton::Left);

    let released = drain(&release_call);
    assert_eq!(released.len(), 1);
    assert_eq!(released[0].kind, PointerEventKind::Up);
    assert_eq!(released[0].point, press);
}

#[test]
fn a_scroll_sample_tracks_the_pointer_without_taking_the_button() {
    let sink = sink(Target::Screen);
    let press = Vector2I::new(10, 10);
    let scrolled = Vector2I::new(400, 300);

    record_positioned_event(
        Some(&sink),
        PointerEventKind::Down,
        Some(MouseButton::Left),
        Target::Screen,
        press,
        press,
    );
    record_positioned_event(
        Some(&sink),
        PointerEventKind::Scroll,
        None,
        Target::Screen,
        scrolled,
        scrolled,
    );
    record_up(Some(&sink), MouseButton::Left);

    let events = drain(&sink);
    assert_eq!(events.len(), 3);
    assert_eq!(events[1].kind, PointerEventKind::Scroll);
    assert_eq!(events[1].button, None);
    // The release still matches the held button, at the scroll's position.
    assert_eq!(events[2].kind, PointerEventKind::Up);
    assert_eq!(events[2].point, scrolled);
}

#[test]
fn an_action_on_another_surface_is_omitted_and_clears_the_session() {
    let sink = sink(RECORDED_WINDOW);
    let press = Vector2I::new(10, 10);
    let stray = Vector2I::new(500, 500);

    record_positioned_event(
        Some(&sink),
        PointerEventKind::Down,
        Some(MouseButton::Left),
        RECORDED_WINDOW,
        press,
        Vector2I::new(910, 610),
    );
    record_positioned_event(
        Some(&sink),
        PointerEventKind::Move,
        None,
        OTHER_WINDOW,
        stray,
        stray,
    );
    record_up(Some(&sink), MouseButton::Left);

    let events = drain(&sink);
    let shape: Vec<_> = events
        .iter()
        .map(|event| (event.kind, event.point))
        .collect();
    assert_eq!(
        shape,
        vec![(PointerEventKind::Down, press)],
        "only the recorded window's press is kept, and the stray move drops the pending release"
    );
}

#[test]
fn a_release_with_no_matching_press_is_ignored() {
    let sink = sink(Target::Screen);
    let press = Vector2I::new(5, 5);

    record_positioned_event(
        Some(&sink),
        PointerEventKind::Down,
        Some(MouseButton::Left),
        Target::Screen,
        press,
        press,
    );
    record_up(Some(&sink), MouseButton::Right);

    let events = drain(&sink);
    assert_eq!(events.len(), 1, "a mismatched release records nothing");
}

#[test]
fn recording_is_a_no_op_without_a_sink() {
    record_positioned_event(
        None,
        PointerEventKind::Down,
        Some(MouseButton::Left),
        Target::Screen,
        Vector2I::new(1, 1),
        Vector2I::new(1, 1),
    );
    record_up(None, MouseButton::Left);
}
