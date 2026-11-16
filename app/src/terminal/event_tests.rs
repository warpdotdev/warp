use super::Event;

#[test]
fn pluggable_notification_debug_redacts_content() {
    let event = Event::PluggableNotification {
        title: Some("private title".to_owned()),
        body: "private body".to_owned(),
    };

    assert_eq!(format!("{event:?}"), "PluggableNotification");
}
