use super::*;
use crate::{MouseButton, PointerEventKind, RecordingHandle, Vector2I};

fn action_log() -> RecordingActionLog {
    let (handle, _) = RecordingHandle::new_test(1920, 1080);
    RecordingActionLog::new(Target::Screen, handle.geometry())
}

fn pointer_down() -> PointerEvent {
    PointerEvent {
        offset: Duration::from_millis(10),
        kind: PointerEventKind::Down,
        button: Some(MouseButton::Left),
        point: Vector2I::new(5, 5),
    }
}

#[test]
fn committed_groups_are_logged_with_their_labels() {
    let mut actions = action_log();
    let (group, sink) = actions.begin_group(vec!["ctrl+s".to_owned()]);
    assert_eq!(sink.recording_target, Target::Screen);
    actions.commit_group(group);

    let [entry] = actions.entries() else {
        panic!("expected one logged action group");
    };
    assert_eq!(entry.labels, ["ctrl+s"]);
    assert!(entry.finish_offset >= entry.offset);
}

#[test]
fn pointer_events_reported_to_the_sink_are_logged_with_their_group() {
    let mut actions = action_log();
    let (group, sink) = actions.begin_group(Vec::new());
    sink.events.lock().unwrap().push(pointer_down());
    actions.commit_group(group);

    let [entry] = actions.entries() else {
        panic!("expected one logged action group");
    };
    assert_eq!(entry.pointer_events, [pointer_down()]);
}

#[test]
fn explicit_finish_is_clamped_to_the_group_start() {
    let mut actions = action_log();
    std::thread::sleep(Duration::from_millis(5));
    let (group, _sink) = actions.begin_group(vec!["a".to_owned()]);
    actions.commit_group_at(group, Duration::ZERO);

    let [entry] = actions.entries() else {
        panic!("expected one logged action group");
    };
    assert!(entry.offset > Duration::ZERO);
    assert_eq!(entry.finish_offset, entry.offset);
}

#[test]
fn discarded_groups_are_not_logged_and_forget_their_press() {
    let mut actions = action_log();
    let (group, sink) = actions.begin_group(vec!["click".to_owned()]);
    sink.session.record_press_or_move(
        PointerEventKind::Down,
        Some(MouseButton::Left),
        Vector2I::new(5, 5),
    );
    actions.discard_group(group);

    assert!(actions.entries().is_empty());
    assert_eq!(sink.session.record_release(MouseButton::Left), None);

    let (group, _sink) = actions.begin_group(vec!["type".to_owned()]);
    actions.commit_group(group);
    assert_eq!(actions.entries().len(), 1);
}
