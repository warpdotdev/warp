use crate::telemetry::event_store::{
    EventPayload, EventStore, SESSION_CONTINUATION_THRESHOLD_SECONDS,
};
use crate::time::{get_current_time, test_offset_time};
use chrono::{Duration, TimeZone, Utc};

#[test]
fn test_initialize_session() {
    test_offset_time(5);
    let event_store = EventStore::new();
    assert_eq!(
        event_store.current_session_created_at,
        Utc.timestamp_opt(5, 0).unwrap()
    );
}

#[test]
fn test_event_queue_empty() {
    let user_id = Some("user123".to_string());
    let anonymous_id = "anon-user-xyz".to_string();
    let mut event_store = EventStore::new();
    event_store.record_app_active(user_id.clone(), anonymous_id.clone(), get_current_time());
    let session_created_at_0 = event_store.current_session_created_at;

    // Queue is empty and an event comes in while session is fresh
    event_store.events.clear();
    test_offset_time(1);
    event_store.record_app_active(user_id.clone(), anonymous_id.clone(), get_current_time());
    assert_eq!(
        event_store.events.back().unwrap().session_created_at,
        session_created_at_0
    );

    // Queue is empty and an event comes in while session is stale
    event_store.events.clear();
    let inactivity_duration = SESSION_CONTINUATION_THRESHOLD_SECONDS as i64 + 1;
    test_offset_time(inactivity_duration);
    let now = get_current_time();
    event_store.record_app_active(user_id, anonymous_id, get_current_time());
    assert_eq!(event_store.events.back().unwrap().session_created_at, now);
}

#[test]
fn test_app_active_after_inactivity() {
    let user_id = Some("user123".to_string());
    let anonymous_id = "anon-user-xyz".to_string();
    let mut event_store = EventStore::new();
    let inactivity_duration = SESSION_CONTINUATION_THRESHOLD_SECONDS as i64 + 1;
    event_store.record_app_active(user_id.clone(), anonymous_id.clone(), get_current_time());
    let session_created_at_0 = event_store.current_session_created_at;

    test_offset_time(inactivity_duration);

    event_store.record_app_active(user_id.clone(), anonymous_id.clone(), get_current_time());
    assert_eq!(
        event_store.events.back().unwrap().payload,
        EventPayload::AppActive {
            user_id: user_id.clone(),
            anonymous_id: anonymous_id.clone()
        }
    );
    let session_created_at_1 = event_store.events.back().unwrap().session_created_at;
    assert_eq!(
        session_created_at_1 - session_created_at_0,
        Duration::seconds(inactivity_duration)
    );

    test_offset_time(inactivity_duration);

    event_store.record_event(
        user_id.clone(),
        anonymous_id.clone(),
        "Block Creation".into(),
        None,
        false, /* contains_ugc */
        get_current_time(),
    );
    assert_eq!(
        event_store.events.back().unwrap().payload,
        EventPayload::NamedEvent {
            user_id,
            anonymous_id,
            name: "Block Creation".into(),
            value: None
        }
    );
    let session_created_at_2 = event_store.events.back().unwrap().session_created_at;
    assert_eq!(
        session_created_at_2 - session_created_at_1,
        Duration::seconds(inactivity_duration)
    );
}

#[test]
fn test_app_active_after_activity() {
    let user_id = Some("user123".to_string());
    let anonymous_id = "anon-user-xyz".to_string();
    let mut event_store = EventStore::new();
    event_store.record_app_active(user_id.clone(), anonymous_id.clone(), get_current_time());
    assert_eq!(event_store.events.len(), 1);
    let timestamp_0 = event_store.events.back().unwrap().timestamp;
    let session_created_at_0 = event_store.events.back().unwrap().session_created_at;

    // Check that the last app active event was updated
    test_offset_time(5);
    event_store.record_app_active(user_id.clone(), anonymous_id.clone(), get_current_time());
    assert_eq!(event_store.events.len(), 1);
    let timestamp_1 = event_store.events.back().unwrap().timestamp;
    let session_created_at_1 = event_store.events.back().unwrap().session_created_at;
    assert_eq!(session_created_at_0, session_created_at_1);
    assert_eq!(timestamp_1 - timestamp_0, Duration::seconds(5));

    test_offset_time(5);
    event_store.record_event(
        user_id.clone(),
        anonymous_id.clone(),
        "Block Creation".into(),
        None,
        false, /* contains_ugc */
        get_current_time(),
    );
    assert_eq!(event_store.events.len(), 2);
    let timestamp_2 = event_store.events.back().unwrap().timestamp;
    let session_created_at_2 = event_store.events.back().unwrap().session_created_at;
    assert_eq!(session_created_at_1, session_created_at_2);
    assert_eq!(timestamp_2 - timestamp_1, Duration::seconds(5));

    test_offset_time(5);
    event_store.record_event(
        user_id.clone(),
        anonymous_id.clone(),
        "Block Creation".into(),
        None,
        false, /* contains_ugc */
        get_current_time(),
    );
    assert_eq!(event_store.events.len(), 3);
    let timestamp_3 = event_store.events.back().unwrap().timestamp;
    let session_created_at_3 = event_store.events.back().unwrap().session_created_at;
    assert_eq!(session_created_at_2, session_created_at_3);
    assert_eq!(timestamp_3 - timestamp_2, Duration::seconds(5));

    test_offset_time(5);
    event_store.record_app_active(user_id, anonymous_id, get_current_time());
    assert_eq!(event_store.events.len(), 4);
    let timestamp_4 = event_store.events.back().unwrap().timestamp;
    let session_created_at_4 = event_store.events.back().unwrap().session_created_at;
    assert_eq!(session_created_at_3, session_created_at_4);
    assert_eq!(timestamp_4 - timestamp_3, Duration::seconds(5));
}
