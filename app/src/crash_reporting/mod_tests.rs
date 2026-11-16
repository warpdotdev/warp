use super::{TAGS, set_event_tags, set_task_id_tag};

#[test]
#[serial_test::serial]
fn task_id_tag_is_attached_to_sentry_events() {
    const TASK_ID: &str = "550e8400-e29b-41d4-a716-446655440000";

    let previous_task_id = TAGS.write().remove("warp.task_id");
    set_task_id_tag(TASK_ID);

    let mut event = sentry::protocol::Event::default();
    set_event_tags(&mut event);

    assert_eq!(
        event.tags.get("warp.task_id").map(String::as_str),
        Some(TASK_ID)
    );

    let mut tags = TAGS.write();
    match previous_task_id {
        Some(task_id) => {
            tags.insert("warp.task_id".to_string(), task_id);
        }
        None => {
            tags.remove("warp.task_id");
        }
    }
}
