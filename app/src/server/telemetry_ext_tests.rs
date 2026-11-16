use chrono::Utc;
use serde_json::json;
use warpui::telemetry::{Event, EventPayload};

use super::*;

// AWS-style access key example used in tests; matches `AWS_ACCESS_ID` in
// `DEFAULT_REGEXES_WITH_NAMES`. The example value is the standard one used in
// AWS documentation and is not a real key.
const AWS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";

/// Constructs a minimal `Event` with a `NamedEvent` payload for testing.
fn make_named_event(value: serde_json::Value, contains_ugc: bool) -> Event {
    let now = Utc::now();
    Event {
        payload: EventPayload::NamedEvent {
            user_id: None,
            anonymous_id: "anon".to_string(),
            name: "TestEvent".into(),
            value: Some(value),
        },
        session_created_at: now,
        timestamp: now,
        contains_ugc,
    }
}

/// Extracts the inner payload `Value` from a `Track`-typed `BatchMessageItem`.
/// This mirrors the structure produced by `form_rudder_track_message`, which
/// wraps the event payload under `properties.payload`.
fn extract_payload(message: RudderBatchMessageItem) -> serde_json::Value {
    let track = match message {
        RudderBatchMessageItem::Track(track) => track,
        other => panic!("expected Track message, got {other:?}"),
    };
    track
        .properties
        .expect("track properties should be set")
        .get("payload")
        .cloned()
        .expect("payload should be set in properties")
}

#[test]
fn to_rudder_batch_message_redacts_ugc_named_events() {
    let payload = json!({
        "command": format!("aws s3 cp {AWS_KEY} ./file"),
        "ok": true,
    });
    let event = make_named_event(payload, /*contains_ugc=*/ true);

    let batch = event.to_rudder_batch_message();
    assert!(batch.contains_ugc);

    let payload = extract_payload(batch.message);
    assert_eq!(
        payload["command"],
        format!("aws s3 cp {} ./file", "*".repeat(AWS_KEY.len())),
    );
    assert_eq!(payload["ok"], true);
}

#[test]
fn to_rudder_batch_message_does_not_redact_non_ugc_named_events() {
    let original_command = format!("aws s3 cp {AWS_KEY} ./file");
    let payload = json!({
        "command": original_command.clone(),
        "ok": true,
    });
    let event = make_named_event(payload, /*contains_ugc=*/ false);

    let batch = event.to_rudder_batch_message();
    assert!(!batch.contains_ugc);

    let payload = extract_payload(batch.message);
    // No redaction should have been applied since the event is not flagged as UGC.
    assert_eq!(payload["command"], original_command);
    assert_eq!(payload["ok"], true);
}

#[test]
fn to_rudder_batch_message_redacts_nested_strings_in_ugc_payload() {
    let payload = json!({
        "outer": {
            "inner_array": [
                format!("first secret: {AWS_KEY}"),
                "no secret here",
            ],
            "scalar_int": 42,
        }
    });
    let event = make_named_event(payload, /*contains_ugc=*/ true);

    let batch = event.to_rudder_batch_message();
    let payload = extract_payload(batch.message);

    assert_eq!(
        payload["outer"]["inner_array"][0],
        format!("first secret: {}", "*".repeat(AWS_KEY.len())),
    );
    assert_eq!(payload["outer"]["inner_array"][1], "no secret here");
    assert_eq!(payload["outer"]["scalar_int"], 42);
}
